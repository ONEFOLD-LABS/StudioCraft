//! Offline-Ausgabe: Mixdown in eine WAV-Datei rendern, Samples als WAV speichern.

use std::path::Path;

use crate::{EngineError, Song};

/// Blockgröße beim Offline-Rendern.
const BLOCK: usize = 4096;

/// Längster Bereich einer Exportdatei in Frames: WAV und AIFF zählen ihre Größe in 32 Bit
/// (24 Bit Stereo, 6 Byte je Frame, Platz für den Kopf); FLAC entsteht erst im Speicher, die
/// gleiche Grenze hält diesen Puffer klein genug. Gut vier Stunden bei 48 kHz.
pub const MAX_EXPORT_FRAMES: u64 = (u32::MAX as u64 - 1024) / 6;

/// Größte Datenmenge einer WAV- oder AIFF-Datei in Byte: beide zählen sie in 32 Bit.
const MAX_FILE_DATA: u64 = u32::MAX as u64 - 1024;

/// Fehler, wenn `samples` Samples zu je `bytes` Byte nicht in eine Datei passen: Sonst liefe
/// die Größe im Dateikopf über und die Datei wäre unlesbar.
fn fits_file(samples: usize, bytes: u64) -> Result<(), EngineError> {
    let size = (samples as u64).saturating_mul(bytes);
    if size <= MAX_FILE_DATA {
        Ok(())
    } else {
        Err(EngineError::TooLarge {
            mb: size / (1 << 20),
        })
    }
}

/// Der Fehler für einen Bereich, den keine Exportdatei fasst.
fn range_too_long(rate: u32) -> EngineError {
    EngineError::RangeTooLong {
        max_secs: MAX_EXPORT_FRAMES / u64::from(rate.max(1)),
        rate,
    }
}

/// Fehler beim Schreiben von `path`.
fn file_error(path: &Path, e: impl std::fmt::Display) -> EngineError {
    EngineError::File {
        path: path.to_path_buf(),
        detail: e.to_string(),
    }
}

/// Die Länge von `start..end` in Frames, oder ein Fehler, wenn keine Datei sie fasst.
fn export_frames(start: u64, end: u64, rate: u32) -> Result<u64, EngineError> {
    let frames = end.saturating_sub(start);
    if frames <= MAX_EXPORT_FRAMES {
        return Ok(frames);
    }
    Err(range_too_long(rate))
}

/// Rendert `start..end` (Frames) der Abspielliste als 24-Bit-WAV.
/// Werte über 0 dBFS werden begrenzt. Gibt die Spitze (linear) zurück.
pub fn render_wav(
    song: &Song,
    rate: u32,
    start: u64,
    end: u64,
    path: &Path,
) -> Result<f32, EngineError> {
    export_frames(start, end, rate)?;
    let spec = hound::WavSpec {
        channels: 2,
        sample_rate: rate,
        bits_per_sample: 24,
        sample_format: hound::SampleFormat::Int,
    };
    let err = |e: hound::Error| file_error(path, e);
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
) -> Result<f32, EngineError> {
    let ext = path
        .extension()
        .and_then(|e| e.to_str())
        .map(str::to_ascii_lowercase);
    let format = match ext.as_deref() {
        Some("flac") => craft_audio::EncodeFormat::Flac,
        Some("aif" | "aiff") => craft_audio::EncodeFormat::Aiff,
        _ => return render_wav(song, rate, start, end, path),
    };
    let frames =
        usize::try_from(export_frames(start, end, rate)?).map_err(|_| range_too_long(rate))?;
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
        .map_err(|e| file_error(path, e))?;
    std::fs::write(path, bytes).map_err(|e| file_error(path, e))?;
    Ok(peak)
}

/// Kodiert fertiges verschachteltes Stereo wie der Mixdown: Format nach Endung `ext`
/// (`wav`, `flac`, `aif`/`aiff`, je 24 Bit), Werte über 0 dBFS begrenzt. Gibt die Bytes und die
/// Spitze (linear) zurück. Grundlage für Dateien und für Downloads im Browser.
pub fn encode_audio(samples: &[f32], rate: u32, ext: &str) -> Result<(Vec<u8>, f32), EngineError> {
    // 24 Bit je Sample; FLAC entsteht im Speicher und bekommt dieselbe Grenze.
    fits_file(samples.len(), 3)?;
    let peak = samples.iter().fold(0.0f32, |m, s| m.max(s.abs()));
    let format = match ext.to_ascii_lowercase().as_str() {
        "flac" => Some(craft_audio::EncodeFormat::Flac),
        "aif" | "aiff" => Some(craft_audio::EncodeFormat::Aiff),
        _ => None,
    };
    let Some(format) = format else {
        let spec = hound::WavSpec {
            channels: 2,
            sample_rate: rate,
            bits_per_sample: 24,
            sample_format: hound::SampleFormat::Int,
        };
        let mut out = std::io::Cursor::new(Vec::new());
        let err = |e: hound::Error| EngineError::Encode(format!("WAV: {e}"));
        let mut writer = hound::WavWriter::new(&mut out, spec).map_err(err)?;
        const MAX: f32 = 8_388_607.0;
        for &s in samples {
            writer
                .write_sample((s.clamp(-1.0, 1.0) * MAX).round() as i32)
                .map_err(err)?;
        }
        writer.finalize().map_err(err)?;
        return Ok((out.into_inner(), peak));
    };
    let clamped: Vec<f32> = samples.iter().map(|s| s.clamp(-1.0, 1.0)).collect();
    let buf = craft_audio::AudioBuffer::from_interleaved(rate, 2, &clamped);
    let bytes = craft_audio::encode(&buf, &craft_audio::EncodeOptions::new(format))
        .map_err(|e| EngineError::Encode(e.to_string()))?;
    Ok((bytes, peak))
}

/// Schreibt fertiges verschachteltes Stereo wie der Mixdown: Format nach Dateiendung
/// (`.wav`, `.flac`, `.aif`/`.aiff`, je 24 Bit), Werte über 0 dBFS begrenzt. Gibt die Spitze
/// (linear) zurück.
pub fn write_audio(samples: &[f32], rate: u32, path: &Path) -> Result<f32, EngineError> {
    let ext = path.extension().and_then(|e| e.to_str()).unwrap_or("wav");
    let (bytes, peak) = encode_audio(samples, rate, ext)?;
    std::fs::write(path, bytes).map_err(|e| file_error(path, e))?;
    Ok(peak)
}

/// Mischt `start..end` (Frames) der Abspielliste offline als verschachteltes Stereo.
pub fn mix_range(song: &Song, start: u64, end: u64) -> Vec<f32> {
    let frames = end.saturating_sub(start) as usize;
    let mut inter = vec![0.0f32; frames * 2];
    let mut pos = 0usize;
    while pos < frames {
        let n = (frames - pos).min(BLOCK);
        song.mix(&mut inter[pos * 2..(pos + n) * 2], start + pos as u64, n);
        pos += n;
    }
    inter
}

/// Kodiert verschachteltes Stereo als 32-Bit-Float-WAV (verlustfrei) in Bytes.
pub fn wav_f32_bytes(samples: &[f32], rate: u32) -> Result<Vec<u8>, EngineError> {
    fits_file(samples.len(), 4)?;
    let spec = hound::WavSpec {
        channels: 2,
        sample_rate: rate,
        bits_per_sample: 32,
        sample_format: hound::SampleFormat::Float,
    };
    let mut out = std::io::Cursor::new(Vec::new());
    let err = |e: hound::Error| EngineError::Encode(format!("WAV: {e}"));
    let mut writer = hound::WavWriter::new(&mut out, spec).map_err(err)?;
    for &s in samples {
        writer.write_sample(s).map_err(err)?;
    }
    writer.finalize().map_err(err)?;
    Ok(out.into_inner())
}

/// Speichert verschachteltes Stereo als 32-Bit-Float-WAV (verlustfrei).
pub fn write_wav_f32(samples: &[f32], rate: u32, path: &Path) -> Result<(), EngineError> {
    write_wav_f32_channels(samples, 2, rate, path)
}

/// Speichert verschachtelte Samples mit `channels` Kanälen als 32-Bit-Float-WAV.
pub fn write_wav_f32_channels(
    samples: &[f32],
    channels: u16,
    rate: u32,
    path: &Path,
) -> Result<(), EngineError> {
    fits_file(samples.len(), 4)?;
    let spec = hound::WavSpec {
        channels,
        sample_rate: rate,
        bits_per_sample: 32,
        sample_format: hound::SampleFormat::Float,
    };
    let err = |e: hound::Error| file_error(path, e);
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
    #[test]
    fn encoded_bytes_decode_back_for_every_format() {
        let samples: Vec<f32> = (0..2000).map(|i| ((i as f32) * 0.01).sin() * 0.5).collect();
        for ext in ["wav", "flac", "aiff"] {
            let (bytes, peak) = encode_audio(&samples, 48_000, ext).unwrap();
            assert!(peak > 0.49 && peak <= 0.5, "{ext}: {peak}");
            let clip = AudioClip::from_bytes(&bytes, Some(ext)).unwrap();
            assert_eq!(clip.sample_rate, 48_000, "{ext}");
            assert_eq!(clip.samples.len(), samples.len(), "{ext}");
            assert!((clip.samples[100] - samples[100]).abs() < 1e-3, "{ext}");
        }
        let raw = wav_f32_bytes(&samples, 44_100).unwrap();
        let clip = AudioClip::from_bytes(&raw, Some("wav")).unwrap();
        assert_eq!(clip.samples[..], samples[..]);
    }

    /// A range longer than a file can hold is refused before anything is written: WAV and
    /// AIFF count their size in 32 bits, FLAC would need the whole range in memory first.
    #[test]
    fn ranges_too_long_for_a_file_are_refused() {
        let song = Song::default();
        for ext in ["wav", "flac", "aif"] {
            let path = temp(&format!("zu-lang.{ext}"));
            let _ = std::fs::remove_file(&path);
            let err = render_file(&song, 48_000, 0, MAX_EXPORT_FRAMES + 1, &path)
                .err()
                .map(|e| e.to_string())
                .unwrap_or_default();
            assert!(err.contains("zu lang"), "{ext}: {err}");
            assert!(!path.exists(), "{ext}: nothing written");
            let err = render_file(&song, 48_000, 10, u64::MAX, &path)
                .err()
                .map(|e| e.to_string())
                .unwrap_or_default();
            assert!(err.contains("zu lang"), "{ext}: {err}");
        }
        // The limit itself fits a WAV header and holds over four hours at 48 kHz.
        const _: () = assert!(MAX_EXPORT_FRAMES * 6 + 44 <= u32::MAX as u64);
        const _: () = assert!(MAX_EXPORT_FRAMES / 48_000 > 4 * 3600);
    }

    /// Data that would not fit the 32-bit size of a WAV or AIFF header is refused before
    /// anything is written, instead of making a file whose header is wrong.
    #[test]
    fn writers_refuse_what_no_file_can_hold() {
        assert!(fits_file(0, 4).is_ok());
        let most = (MAX_FILE_DATA / 4) as usize;
        assert!(fits_file(most, 4).is_ok());
        let err = fits_file(most + 1, 4).unwrap_err().to_string();
        assert!(err.contains("4096 MB"), "{err}");
        assert!(
            fits_file(usize::MAX, 4).is_err(),
            "no overflow in the check"
        );
        // 24-bit stereo: the export limit is the same.
        assert!(fits_file(MAX_EXPORT_FRAMES as usize * 2, 3).is_ok());
    }
}
