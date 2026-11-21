//! Audio-Clips: eine Datei, vollständig in den Speicher geladen, als
//! verschachteltes Stereo in `f32`.

use std::{fmt, path::Path, sync::Arc};

#[derive(Debug, Clone)]
pub struct AudioClip {
    pub sample_rate: u32,
    /// Verschachtelte Stereo-Samples (L, R, L, R, …).
    pub samples: Arc<[f32]>,
}

#[derive(Debug)]
pub enum ClipError {
    Wav(hound::Error),
    UnsupportedChannels(u16),
    Io(std::io::Error),
    Decode(String),
    /// Abtastrate außerhalb von [`SAMPLE_RATES`].
    UnsupportedRate(u32),
    /// Bittiefe, die es nicht gibt (WAV: 1–32 Bit Integer).
    UnsupportedBits(u16),
}

/// Abtastraten, die ein Clip haben darf; darunter oder darüber ist die Datei kaputt.
pub const SAMPLE_RATES: std::ops::RangeInclusive<u32> = 1_000..=768_000;

fn check_rate(rate: u32) -> Result<u32, ClipError> {
    if SAMPLE_RATES.contains(&rate) {
        Ok(rate)
    } else {
        Err(ClipError::UnsupportedRate(rate))
    }
}

impl fmt::Display for ClipError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Wav(e) => write!(f, "WAV-Datei nicht lesbar: {e}"),
            Self::UnsupportedChannels(n) => write!(f, "{n} Kanäle werden nicht unterstützt"),
            Self::Io(e) => write!(f, "Datei nicht lesbar: {e}"),
            Self::Decode(e) => write!(f, "Audioformat nicht lesbar: {e}"),
            Self::UnsupportedRate(r) => write!(f, "Abtastrate {r} Hz wird nicht unterstützt"),
            Self::UnsupportedBits(b) => write!(f, "{b} Bit werden nicht unterstützt"),
        }
    }
}

impl std::error::Error for ClipError {}

impl From<hound::Error> for ClipError {
    fn from(e: hound::Error) -> Self {
        Self::Wav(e)
    }
}

impl AudioClip {
    /// Lädt eine Audiodatei über craft-audio: WAV, AIFF, FLAC, MP3, Ogg/Vorbis,
    /// Opus, AAC/M4A, CAF u. a. Mono wird auf beide Seiten gelegt, bei mehr als
    /// zwei Kanälen gelten die ersten beiden.
    pub fn load(path: impl AsRef<Path>) -> Result<Self, ClipError> {
        let path = path.as_ref();
        let bytes = std::fs::read(path).map_err(ClipError::Io)?;
        let ext = path.extension().and_then(|e| e.to_str());
        Self::from_bytes(&bytes, ext)
    }

    /// Wie [`Self::load`], aber aus Bytes (etwa eine im Browser gewählte Datei); `ext` ist die
    /// Dateiendung als Hinweis auf das Format.
    pub fn from_bytes(bytes: &[u8], ext: Option<&str>) -> Result<Self, ClipError> {
        Self::from_bytes_channel(bytes, ext, None)
    }

    /// Wie [`Self::from_bytes`]; mit `channel` nur dieser Kanal der Datei (0-basiert), auf
    /// beide Seiten gelegt (etwa ein Kanal einer Mehrkanaldatei aus einem AAF-Projekt).
    pub fn from_bytes_channel(
        bytes: &[u8],
        ext: Option<&str>,
        channel: Option<usize>,
    ) -> Result<Self, ClipError> {
        let ext = ext.map(str::to_ascii_lowercase);
        let (_, buf) = craft_audio::decode(bytes, ext.as_deref())
            .map_err(|e| ClipError::Decode(e.to_string()))?;
        let sample_rate = check_rate(buf.sample_rate)?;
        let n = buf.channels.len();
        let (l, r) = match channel {
            Some(c) => {
                let one = buf
                    .channels
                    .get(c)
                    .ok_or(ClipError::UnsupportedChannels(n as u16))?;
                (one, one)
            }
            None => {
                let l = buf
                    .channels
                    .first()
                    .ok_or(ClipError::UnsupportedChannels(0))?;
                (l, buf.channels.get(1).unwrap_or(l))
            }
        };
        let samples: Vec<f32> = l.iter().zip(r).flat_map(|(&a, &b)| [a, b]).collect();
        Ok(Self {
            sample_rate,
            samples: samples.into(),
        })
    }

    /// Rohes PCM (little-endian, verschachtelt; 8 Bit ohne Vorzeichen, 16/24/32 Bit mit), wie
    /// es AAF und OMF einbetten. Mit `channel` nur dieser Kanal, sonst die ersten beiden.
    pub fn from_pcm(
        data: &[u8],
        channels: u16,
        bits: u16,
        sample_rate: u32,
        channel: Option<usize>,
    ) -> Result<Self, ClipError> {
        let sample_rate = check_rate(sample_rate)?;
        if channels == 0 {
            return Err(ClipError::UnsupportedChannels(0));
        }
        let width = match bits {
            8 | 16 | 24 | 32 => usize::from(bits / 8),
            b => return Err(ClipError::UnsupportedBits(b)),
        };
        let n = usize::from(channels);
        let pick = |c: usize| c.min(n - 1);
        let (l, r) = match channel {
            Some(c) if c >= n => return Err(ClipError::UnsupportedChannels(channels)),
            Some(c) => (c, c),
            None => (0, pick(1)),
        };
        let sample = |s: &[u8]| -> f32 {
            match s {
                [a] => (f32::from(*a) - 128.0) / 128.0,
                [a, b] => f32::from(i16::from_le_bytes([*a, *b])) / 32_768.0,
                [a, b, c] => (i32::from_le_bytes([0, *a, *b, *c]) >> 8) as f32 / 8_388_608.0,
                [a, b, c, d] => i32::from_le_bytes([*a, *b, *c, *d]) as f32 / 2_147_483_648.0,
                _ => 0.0,
            }
        };
        let frame = width * n;
        let samples: Vec<f32> = data
            .chunks_exact(frame)
            .flat_map(|f| {
                let at = |c: usize| sample(f.get(c * width..(c + 1) * width).unwrap_or(&[]));
                [at(l), at(r)]
            })
            .collect();
        Ok(Self {
            sample_rate,
            samples: samples.into(),
        })
    }

    /// Dateiendungen, die [`Self::load`] kennt (für Dateidialoge).
    pub fn extensions() -> &'static [&'static str] {
        craft_audio::DECODE_EXTENSIONS
    }

    /// Lädt eine WAV-Datei (8–32 Bit Integer oder 32 Bit Float, Mono oder Stereo).
    pub fn load_wav(path: impl AsRef<Path>) -> Result<Self, ClipError> {
        let reader = hound::WavReader::open(path)?;
        let spec = reader.spec();
        if !(1..=2).contains(&spec.channels) {
            return Err(ClipError::UnsupportedChannels(spec.channels));
        }
        check_rate(spec.sample_rate)?;
        if spec.sample_format == hound::SampleFormat::Int
            && !(1..=32).contains(&spec.bits_per_sample)
        {
            return Err(ClipError::UnsupportedBits(spec.bits_per_sample));
        }
        let raw: Vec<f32> = match spec.sample_format {
            hound::SampleFormat::Float => reader.into_samples::<f32>().collect::<Result<_, _>>()?,
            hound::SampleFormat::Int => {
                let scale = 1.0 / (1u64 << (spec.bits_per_sample - 1)) as f32;
                reader
                    .into_samples::<i32>()
                    .map(|s| s.map(|v| v as f32 * scale))
                    .collect::<Result<_, _>>()?
            }
        };
        let samples: Vec<f32> = if spec.channels == 1 {
            raw.iter().flat_map(|&s| [s, s]).collect()
        } else {
            raw
        };
        Ok(Self {
            sample_rate: spec.sample_rate,
            samples: samples.into(),
        })
    }

    pub fn frames(&self) -> u64 {
        (self.samples.len() / 2) as u64
    }

    /// Rechnet den Clip auf eine andere Abtastrate um (lineare Interpolation).
    /// Für den Durchstich gut genug; später übernimmt das ein hochwertiger
    /// Resampler (craft-audio).
    pub fn resampled(&self, rate: u32) -> Self {
        if rate == self.sample_rate || self.samples.is_empty() || rate == 0 || self.sample_rate == 0
        {
            return self.clone();
        }
        let src = &self.samples;
        let src_frames = src.len() / 2;
        let ratio = f64::from(self.sample_rate) / f64::from(rate);
        let dst_frames = ((src_frames as f64) / ratio).floor() as usize;
        let mut out = Vec::with_capacity(dst_frames * 2);
        for i in 0..dst_frames {
            let pos = i as f64 * ratio;
            let i0 = pos as usize;
            let i1 = (i0 + 1).min(src_frames - 1);
            let t = (pos - i0 as f64) as f32;
            for ch in 0..2 {
                let a = src[i0 * 2 + ch];
                let b = src[i1 * 2 + ch];
                out.push(a + (b - a) * t);
            }
        }
        Self {
            sample_rate: rate,
            samples: out.into(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn corrupt_wav_headers_and_truncated_files_give_errors_not_panics() {
        let dir = std::env::temp_dir().join(format!("studiocraft-clip-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let good = dir.join("good.wav");
        let spec = hound::WavSpec {
            channels: 1,
            sample_rate: 48_000,
            bits_per_sample: 16,
            sample_format: hound::SampleFormat::Int,
        };
        let mut w = hound::WavWriter::create(&good, spec).unwrap();
        for i in 0..100i16 {
            w.write_sample(i * 100).unwrap();
        }
        w.finalize().unwrap();
        let bytes = std::fs::read(&good).unwrap();
        let data = bytes.windows(4).position(|w| w == b"data").unwrap();
        let patch = |at: usize, value: &[u8]| {
            let mut b = bytes.clone();
            b[at..at + value.len()].copy_from_slice(value);
            b
        };
        let mut variants: Vec<(String, Vec<u8>)> = vec![
            ("channels 0".into(), patch(22, &0u16.to_le_bytes())),
            ("channels 9".into(), patch(22, &9u16.to_le_bytes())),
            ("rate 0".into(), patch(24, &0u32.to_le_bytes())),
            ("rate max".into(), patch(24, &u32::MAX.to_le_bytes())),
            ("byte rate 0".into(), patch(28, &0u32.to_le_bytes())),
            ("block align 0".into(), patch(32, &0u16.to_le_bytes())),
            ("bits 0".into(), patch(34, &0u16.to_le_bytes())),
            ("bits 64".into(), patch(34, &64u16.to_le_bytes())),
            ("bits 200".into(), patch(34, &200u16.to_le_bytes())),
            (
                "data size max".into(),
                patch(data + 4, &u32::MAX.to_le_bytes()),
            ),
            ("riff size 0".into(), patch(4, &0u32.to_le_bytes())),
        ];
        for cut in (0..bytes.len()).step_by(5) {
            variants.push((format!("cut at {cut}"), bytes[..cut].to_vec()));
        }
        for (name, b) in &variants {
            let path = dir.join("bad.wav");
            std::fs::write(&path, b).unwrap();
            for c in [AudioClip::load(&path), AudioClip::load_wav(&path)]
                .into_iter()
                .flatten()
            {
                assert!(SAMPLE_RATES.contains(&c.sample_rate), "{name}");
                let _ = c.resampled(44_100).frames();
            }
        }
        let rate0 = dir.join("rate0.wav");
        std::fs::write(&rate0, &variants[2].1).unwrap();
        assert!(AudioClip::load_wav(&rate0).is_err(), "rate 0 is rejected");
        assert!(AudioClip::load(&rate0).is_err(), "rate 0 is rejected");
        // A clip with a broken rate (built in code) still resamples without panicking.
        let broken = AudioClip {
            sample_rate: 0,
            samples: vec![0.0; 8].into(),
        };
        assert_eq!(broken.resampled(48_000).frames(), 4);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn resample_keeps_duration_and_shape() {
        let samples: Vec<f32> = (0..441).flat_map(|i| [i as f32, 0.0]).collect();
        let clip = AudioClip {
            sample_rate: 44_100,
            samples: samples.into(),
        };
        let r = clip.resampled(48_000);
        assert_eq!(r.sample_rate, 48_000);
        assert_eq!(r.frames(), 480);
        // Frame 240 bei 48 kHz liegt bei 220,5 Frames der Quelle.
        assert!((r.samples[240 * 2] - 220.5).abs() < 1e-3);
    }

    #[test]
    fn same_rate_is_unchanged() {
        let clip = AudioClip {
            sample_rate: 48_000,
            samples: vec![0.5; 8].into(),
        };
        assert_eq!(clip.resampled(48_000).samples, clip.samples);
    }
}

#[cfg(test)]
mod any_file {
    /// Lädt eine beliebige Datei: `STUDIOCRAFT_LOAD=<pfad> cargo test load_any_file -- --ignored --nocapture`
    #[test]
    #[ignore = "braucht eine Datei von außen"]
    fn load_any_file() {
        let Some(p) = std::env::var_os("STUDIOCRAFT_LOAD") else {
            return;
        };
        match super::AudioClip::load(&p) {
            Ok(c) => println!("geladen: {} Frames, {} Hz", c.frames(), c.sample_rate),
            Err(e) => println!("Fehler: {e}"),
        }
    }
}
