//! Offline-Ausgabe: Mixdown in eine WAV-Datei rendern, Samples als WAV speichern.

use std::path::Path;

use crate::Song;

/// Blockgröße beim Offline-Rendern.
const BLOCK: usize = 4096;

/// Rendert `start..end` (Frames) der Abspielliste als 24-Bit-WAV.
/// Werte über 0 dBFS werden begrenzt. Gibt die Spitze (linear) zurück.
pub fn render_wav(
    song: &Song,
    rate: u32,
    start: u64,
    end: u64,
    path: &Path,
) -> Result<f32, String> {
    let spec = hound::WavSpec {
        channels: 2,
        sample_rate: rate,
        bits_per_sample: 24,
        sample_format: hound::SampleFormat::Int,
    };
    let err = |e: hound::Error| format!("{}: {e}", path.display());
    let mut writer = hound::WavWriter::create(path, spec).map_err(err)?;
    let mut buf = vec![0.0f32; BLOCK * 2];
    let mut peak = 0.0f32;
    let mut pos = start;
    const MAX: f32 = 8_388_607.0;
    while pos < end {
        let n = ((end - pos) as usize).min(BLOCK);
        let block = &mut buf[..n * 2];
        block.fill(0.0);
        song.mix(block, pos, n);
        for &s in block.iter() {
            peak = peak.max(s.abs());
            writer
                .write_sample((s.clamp(-1.0, 1.0) * MAX).round() as i32)
                .map_err(err)?;
        }
        pos += n as u64;
    }
    writer.finalize().map_err(err)?;
    Ok(peak)
}

/// Rendert wie [`render_wav`], das Format folgt der Dateiendung: `.wav`
/// (24 Bit), `.flac` (24 Bit) oder `.aif`/`.aiff` (24 Bit). Gibt die Spitze zurück.
pub fn render_file(
    song: &Song,
    rate: u32,
    start: u64,
    end: u64,
    path: &Path,
) -> Result<f32, String> {
    let ext = path
        .extension()
        .and_then(|e| e.to_str())
        .map(str::to_ascii_lowercase);
    let format = match ext.as_deref() {
        Some("flac") => craft_audio::EncodeFormat::Flac,
        Some("aif" | "aiff") => craft_audio::EncodeFormat::Aiff,
        _ => return render_wav(song, rate, start, end, path),
    };
    let frames = (end.saturating_sub(start)) as usize;
    let mut inter = vec![0.0f32; frames * 2];
    let mut pos = 0usize;
    while pos < frames {
        let n = (frames - pos).min(BLOCK);
        song.mix(&mut inter[pos * 2..(pos + n) * 2], start + pos as u64, n);
        pos += n;
    }
    let peak = inter.iter().fold(0.0f32, |m, s| m.max(s.abs()));
    for s in &mut inter {
        *s = s.clamp(-1.0, 1.0);
    }
    let buf = craft_audio::AudioBuffer::from_interleaved(rate, 2, &inter);
    let bytes = craft_audio::encode(&buf, &craft_audio::EncodeOptions::new(format))
        .map_err(|e| format!("{}: {e}", path.display()))?;
    std::fs::write(path, bytes).map_err(|e| format!("{}: {e}", path.display()))?;
    Ok(peak)
}

/// Speichert verschachteltes Stereo als 32-Bit-Float-WAV (verlustfrei).
pub fn write_wav_f32(samples: &[f32], rate: u32, path: &Path) -> Result<(), String> {
    write_wav_f32_channels(samples, 2, rate, path)
}

/// Speichert verschachtelte Samples mit `channels` Kanälen als 32-Bit-Float-WAV.
pub fn write_wav_f32_channels(
    samples: &[f32],
    channels: u16,
    rate: u32,
    path: &Path,
) -> Result<(), String> {
    let spec = hound::WavSpec {
        channels,
        sample_rate: rate,
        bits_per_sample: 32,
        sample_format: hound::SampleFormat::Float,
    };
    let err = |e: hound::Error| format!("{}: {e}", path.display());
    let mut writer = hound::WavWriter::create(path, spec).map_err(err)?;
    for &s in samples {
        writer.write_sample(s).map_err(err)?;
    }
    writer.finalize().map_err(err)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::AudioClip;
    use std::sync::Arc;

    fn temp(name: &str) -> std::path::PathBuf {
        let dir = std::env::temp_dir().join(format!("studiocraft-test-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        dir.join(name)
    }

    #[test]
    fn mixdown_has_range_length_and_content() {
        let samples: Arc<[f32]> = (0..1000)
            .flat_map(|_| [0.5f32, -0.25])
            .collect::<Vec<_>>()
            .into();
        let song = Song::single(samples);
        let path = temp("mixdown.wav");
        let peak = render_wav(&song, 48_000, 100, 1100, &path).unwrap();
        assert_eq!(peak, 0.5);
        let clip = AudioClip::load_wav(&path).unwrap();
        assert_eq!(clip.frames(), 1000, "Bereich 100..1100");
        assert!((clip.samples[0] - 0.5).abs() < 1e-5);
        assert!((clip.samples[1] + 0.25).abs() < 1e-5);
        assert_eq!(clip.samples[1800], 0.0, "nach Clip-Ende (Frame 1000) still");
    }

    #[test]
    fn flac_and_aiff_export_load_back() {
        let samples: Arc<[f32]> = (0..4800)
            .flat_map(|i| {
                let v = (i as f32 * 0.05).sin() * 0.5;
                [v, -v]
            })
            .collect::<Vec<_>>()
            .into();
        let song = Song::single(samples.clone());
        for ext in ["flac", "aif"] {
            let path = temp(&format!("mix.{ext}"));
            render_file(&song, 48_000, 0, 4800, &path).unwrap();
            let clip = AudioClip::load(&path).unwrap();
            assert_eq!((clip.sample_rate, clip.frames()), (48_000, 4800), "{ext}");
            let err = clip
                .samples
                .iter()
                .zip(samples.iter())
                .map(|(a, b)| (a - b).abs())
                .fold(0.0f32, f32::max);
            assert!(err < 1e-5, "{ext}: 24 Bit verlustfrei, Abweichung {err}");
        }
    }

    #[test]
    fn float_wav_roundtrip() {
        let path = temp("clip.wav");
        let s = [0.1f32, -0.2, 0.3, -0.4];
        write_wav_f32(&s, 44_100, &path).unwrap();
        let clip = AudioClip::load_wav(&path).unwrap();
        assert_eq!(clip.sample_rate, 44_100);
        assert_eq!(&clip.samples[..], &s);
    }
}
