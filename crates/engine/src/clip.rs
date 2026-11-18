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
}

impl fmt::Display for ClipError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Wav(e) => write!(f, "WAV-Datei nicht lesbar: {e}"),
            Self::UnsupportedChannels(n) => {
                write!(f, "{n} Kanäle werden nicht unterstützt (nur Mono und Stereo)")
            }
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
    /// Lädt eine WAV-Datei (8–32 Bit Integer oder 32 Bit Float, Mono oder Stereo).
    pub fn load_wav(path: impl AsRef<Path>) -> Result<Self, ClipError> {
        let reader = hound::WavReader::open(path)?;
        let spec = reader.spec();
        if !(1..=2).contains(&spec.channels) {
            return Err(ClipError::UnsupportedChannels(spec.channels));
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
        Ok(Self { sample_rate: spec.sample_rate, samples: samples.into() })
    }

    pub fn frames(&self) -> u64 {
        (self.samples.len() / 2) as u64
    }
}
