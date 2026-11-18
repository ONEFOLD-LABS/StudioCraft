//! Audio-Ausgabe über cpal: ein Code-Pfad für alle Systeme.
//!
//! | System | Hosts |
//! | --- | --- |
//! | Linux, FreeBSD | PipeWire (Feature `pipewire`, Standard), ALSA, JACK (Feature `jack`) |
//! | Windows | WASAPI, ASIO (Feature `asio`) |
//! | macOS | CoreAudio, JACK (Feature `jack`) |
//! | Web | Web Audio (wasm32) |

use cpal::{
    BufferSize, Device, FromSample, SampleFormat, SizedSample, Stream, StreamConfig,
    SupportedBufferSize,
    traits::{DeviceTrait, HostTrait, StreamTrait},
};

use crate::EngineCore;

/// Gewünschte Blockgröße in Frames (Spec-Ziel: ≤ 10 ms bei 48 kHz).
pub const DEFAULT_BLOCK_FRAMES: u32 = 256;
/// Größter Block, den der Kern in einem Stück rendert.
const SCRATCH_FRAMES: usize = 8192;

/// Was für die Ausgabe gewählt wurde.
#[derive(Debug, Clone)]
pub struct OutputInfo {
    pub host: String,
    pub device: String,
    pub sample_rate: u32,
    pub channels: u16,
    /// `None`: Blockgröße bestimmt der Treiber.
    pub block_frames: Option<u32>,
    pub sample_format: String,
}

/// Gewähltes Gerät samt Konfiguration, noch ohne laufenden Stream.
/// Der Aufrufer kennt damit die Abtastrate, bevor er den Kern baut.
pub struct PreparedOutput {
    device: Device,
    config: StreamConfig,
    format: SampleFormat,
    pub info: OutputInfo,
}

/// Laufende Ausgabe. Beim Drop wird der Stream beendet.
pub struct AudioOutput {
    _stream: Stream,
    pub info: OutputInfo,
}

/// Namen aller Hosts, die in diesen Build einkompiliert sind.
pub fn available_hosts() -> Vec<&'static str> {
    cpal::available_hosts()
        .into_iter()
        .map(|h| h.name())
        .collect()
}

impl PreparedOutput {
    /// Wählt Host und Standard-Ausgabegerät. Unterstützt das Gerät
    /// `preferred_rate`, wird diese genommen, sonst seine Standardrate.
    pub fn open(host_name: Option<&str>, preferred_rate: u32) -> Result<Self, String> {
        let host = match host_name {
            None => cpal::default_host(),
            Some(name) => {
                let id = cpal::available_hosts()
                    .into_iter()
                    .find(|h| h.name().eq_ignore_ascii_case(name))
                    .ok_or_else(|| {
                        format!(
                            "Audio-Host „{name}“ gibt es in diesem Build nicht (verfügbar: {})",
                            available_hosts().join(", ")
                        )
                    })?;
                cpal::host_from_id(id).map_err(|e| format!("Audio-Host „{name}“: {e}"))?
            }
        };
        let device = host
            .default_output_device()
            .ok_or("Kein Audio-Ausgabegerät gefunden")?;
        let device_name = device
            .description()
            .map(|d| d.name().to_owned())
            .unwrap_or_else(|_| "unbekannt".into());

        let default = device
            .default_output_config()
            .map_err(|e| format!("Gerät „{device_name}“: {e}"))?;
        let supported = device
            .supported_output_configs()
            .ok()
            .and_then(|mut it| {
                it.find(|c| {
                    c.sample_format() == default.sample_format()
                        && c.channels() == default.channels()
                })
                .and_then(|c| c.try_with_sample_rate(preferred_rate))
            })
            .unwrap_or(default);

        let block = match supported.buffer_size() {
            SupportedBufferSize::Range { min, max }
                if (*min..=*max).contains(&DEFAULT_BLOCK_FRAMES) =>
            {
                Some(DEFAULT_BLOCK_FRAMES)
            }
            _ => None,
        };
        let format = supported.sample_format();
        let mut config: StreamConfig = supported.into();
        config.buffer_size = block.map_or(BufferSize::Default, BufferSize::Fixed);

        let info = OutputInfo {
            host: host.id().name().to_owned(),
            device: device_name,
            sample_rate: config.sample_rate,
            channels: config.channels,
            block_frames: block,
            sample_format: format.to_string(),
        };
        Ok(Self {
            device,
            config,
            format,
            info,
        })
    }

    /// Startet den Stream; `core` muss mit `info.sample_rate` laufen.
    pub fn start(self, core: EngineCore) -> Result<AudioOutput, String> {
        let Self {
            device,
            config,
            format,
            info,
        } = self;
        let stream = match format {
            SampleFormat::F32 => build::<f32>(&device, &config, core),
            SampleFormat::F64 => build::<f64>(&device, &config, core),
            SampleFormat::I16 => build::<i16>(&device, &config, core),
            SampleFormat::I32 => build::<i32>(&device, &config, core),
            SampleFormat::U16 => build::<u16>(&device, &config, core),
            SampleFormat::U8 => build::<u8>(&device, &config, core),
            other => Err(format!("Sampleformat {other} wird nicht unterstützt")),
        }?;
        stream.play().map_err(|e| format!("Stream-Start: {e}"))?;
        Ok(AudioOutput {
            _stream: stream,
            info,
        })
    }
}

fn build<T>(device: &Device, config: &StreamConfig, mut core: EngineCore) -> Result<Stream, String>
where
    T: SizedSample + FromSample<f32>,
{
    let channels = usize::from(config.channels);
    let mut scratch = vec![0.0f32; SCRATCH_FRAMES * 2].into_boxed_slice();
    device
        .build_output_stream(
            *config,
            move |data: &mut [T], _| {
                // Echtzeit-Callback: keine Allokation, kein Lock.
                for out in data.chunks_mut(SCRATCH_FRAMES * channels) {
                    let frames = out.len() / channels;
                    let block = &mut scratch[..frames * 2];
                    core.process(block);
                    for (frame, lr) in out.chunks_exact_mut(channels).zip(block.chunks_exact(2)) {
                        if channels == 1 {
                            frame[0] = T::from_sample((lr[0] + lr[1]) * 0.5);
                            continue;
                        }
                        frame[0] = T::from_sample(lr[0]);
                        frame[1] = T::from_sample(lr[1]);
                        for s in &mut frame[2..] {
                            *s = T::EQUILIBRIUM;
                        }
                    }
                }
            },
            |err| eprintln!("Audio-Stream: {err}"),
            None,
        )
        .map_err(|e| format!("Stream-Aufbau: {e}"))
}
