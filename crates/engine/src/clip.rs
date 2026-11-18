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
                write!(
                    f,
                    "{n} Kanäle werden nicht unterstützt (nur Mono und Stereo)"
                )
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
        if rate == self.sample_rate || self.samples.is_empty() {
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
