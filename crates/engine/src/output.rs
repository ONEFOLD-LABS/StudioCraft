//! Audio-Ausgabe über cpal: ein Code-Pfad für alle Systeme.
//!
//! | System | Hosts |
//! | --- | --- |
//! | Linux, FreeBSD | PipeWire (Feature `pipewire`, Standard), ALSA, JACK (Feature `jack`) |
//! | Windows | WASAPI, ASIO (Feature `asio`) |
//! | macOS | CoreAudio, JACK (Feature `jack`) |
//! | Web | Web Audio (wasm32) |

use std::sync::{
    Arc,
    atomic::{AtomicBool, Ordering},
};
use std::time::{Duration, Instant};

use cpal::{
    BufferSize, Device, FromSample, Host, SampleFormat, SizedSample, Stream, StreamConfig,
    SupportedBufferSize,
    traits::{DeviceTrait, HostTrait, StreamTrait},
};

use crate::EngineCore;

/// Gewünschte Audio-Geräte (Dialog „Audio- und MIDI-Geräte“); `None` heißt Standard.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct DeviceChoice {
    pub host: Option<String>,
    pub output: Option<String>,
    pub input: Option<String>,
    pub sample_rate: Option<u32>,
    pub block_frames: Option<u32>,
}

/// Host nach Namen; ohne Namen der Standard des Systems.
pub(crate) fn host_by_name(name: Option<&str>) -> Result<Host, String> {
    match name {
        None => Ok(cpal::default_host()),
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
            cpal::host_from_id(id).map_err(|e| format!("Audio-Host „{name}“: {e}"))
        }
    }
}

pub(crate) fn device_name(d: &Device) -> String {
    d.description()
        .map(|d| d.name().to_owned())
        .unwrap_or_else(|_| "unbekannt".into())
}

/// Namen der Aus- und Eingabegeräte eines Hosts.
pub fn device_names(host: Option<&str>) -> Result<(Vec<String>, Vec<String>), String> {
    let host = host_by_name(host)?;
    let outputs = host
        .output_devices()
        .map(|it| it.map(|d| device_name(&d)).collect())
        .unwrap_or_default();
    let inputs = host
        .input_devices()
        .map(|it| it.map(|d| device_name(&d)).collect())
        .unwrap_or_default();
    Ok((outputs, inputs))
}

/// Übliche Abtastraten und Blockgrößen für die Auswahl im Dialog.
pub const SAMPLE_RATES: [u32; 6] = [44_100, 48_000, 88_200, 96_000, 176_400, 192_000];
pub const BLOCK_SIZES: [u32; 7] = [32, 64, 128, 256, 512, 1024, 2048];

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
    stream: Stream,
    pub info: OutputInfo,
    shared: Arc<StreamShared>,
    /// Gibt der Audio-Thread den Kern zurück (Gerätewechsel), landet er hier.
    handback: rtrb::Consumer<EngineCore>,
    /// Weg des Kerns in den Audio-Thread (nach dem Aufbau des Streams, siehe `start`).
    inbound: rtrb::Producer<EngineCore>,
}

/// Zwischen Haupt- und Audio-Thread, nur Atomics.
#[derive(Default)]
struct StreamShared {
    /// Bitte den Kern zurückgeben und danach Stille ausgeben.
    give_back: AtomicBool,
    /// Der Stream meldete einen Fehler (z. B. Gerät entfernt).
    lost: AtomicBool,
}

impl AudioOutput {
    /// Ob das Gerät einen Fehler gemeldet hat (z. B. abgezogen).
    pub fn device_lost(&self) -> bool {
        self.shared.lost.load(Ordering::Relaxed)
    }

    /// Holt den Kern aus dem laufenden Stream zurück und beendet den Stream, damit er auf
    /// einem anderen Gerät weiterläuft (gleiche Abtastrate). Der Kern behält Song, Plugins und
    /// Transport. Antwortet der Audio-Thread nicht (Gerät weg), bleibt der Stream bestehen und
    /// kommt mit dem Fehler zurück: so wird der Kern samt Plugins nie nebenbei zerstört.
    pub fn take_core(mut self, timeout: Duration) -> Result<EngineCore, Box<(Self, String)>> {
        self.shared.give_back.store(true, Ordering::Release);
        let deadline = Instant::now() + timeout;
        loop {
            if let Ok(core) = self.handback.pop() {
                drop(self.stream);
                return Ok(core);
            }
            if Instant::now() >= deadline {
                self.shared.give_back.store(false, Ordering::Release);
                // Kam der Kern doch noch, geht er zurück in den Stream.
                if let Ok(core) = self.handback.pop() {
                    let _ = self.inbound.push(core);
                }
                return Err(Box::new((
                    self,
                    "Das Audiogerät antwortet nicht; der Wechsel gilt nach einem Neustart".into(),
                )));
            }
            std::thread::sleep(Duration::from_millis(2));
        }
    }
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
        Self::open_with(&DeviceChoice {
            host: host_name.map(str::to_owned),
            sample_rate: Some(preferred_rate),
            ..DeviceChoice::default()
        })
    }

    /// Wie [`Self::open`], mit Gerät, Abtastrate und Blockgröße aus `choice`. Ein genanntes
    /// Gerät, das es nicht (mehr) gibt, ist ein Fehler.
    pub fn open_with(choice: &DeviceChoice) -> Result<Self, String> {
        let host = host_by_name(choice.host.as_deref())?;
        let preferred_rate = choice.sample_rate.unwrap_or(48_000);
        let device = match &choice.output {
            None => host
                .default_output_device()
                .ok_or("Kein Audio-Ausgabegerät gefunden")?,
            Some(name) => host
                .output_devices()
                .map_err(|e| format!("Ausgabegeräte: {e}"))?
                .find(|d| device_name(d) == *name)
                .ok_or_else(|| format!("Ausgabegerät „{name}“ nicht gefunden"))?,
        };
        let device_name = device_name(&device);

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

        let wanted = choice.block_frames.unwrap_or(DEFAULT_BLOCK_FRAMES);
        let block = match supported.buffer_size() {
            SupportedBufferSize::Range { min, max } if (*min..=*max).contains(&wanted) => {
                Some(wanted)
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

    /// Startet den Stream; `core` muss mit `info.sample_rate` laufen. Der Kern geht erst in
    /// den Audio-Thread, wenn der Stream läuft; scheitert der Start, kommt er mit dem Fehler
    /// zurück und wird nicht zerstört (Plugins, deren Fenster die Oberfläche hält).
    pub fn start(self, core: EngineCore) -> Result<AudioOutput, Box<(EngineCore, String)>> {
        let Self {
            device,
            config,
            format,
            info,
        } = self;
        let shared = Arc::new(StreamShared::default());
        let (back_tx, handback) = rtrb::RingBuffer::new(1);
        let (mut inbound, inbound_rx) = rtrb::RingBuffer::new(1);
        let parts = (inbound_rx, shared.clone(), back_tx);
        let stream = match format {
            SampleFormat::F32 => build::<f32>(&device, &config, parts),
            SampleFormat::F64 => build::<f64>(&device, &config, parts),
            SampleFormat::I16 => build::<i16>(&device, &config, parts),
            SampleFormat::I32 => build::<i32>(&device, &config, parts),
            SampleFormat::U16 => build::<u16>(&device, &config, parts),
            SampleFormat::U8 => build::<u8>(&device, &config, parts),
            other => Err(format!("Sampleformat {other} wird nicht unterstützt")),
        };
        let stream = match stream {
            Ok(s) => s,
            Err(e) => return Err(Box::new((core, e))),
        };
        if let Err(e) = stream.play() {
            return Err(Box::new((core, format!("Stream-Start: {e}"))));
        }
        // Platz für genau einen Kern, der Ring ist leer: geht immer.
        if let Err(rtrb::PushError::Full(core)) = inbound.push(core) {
            return Err(Box::new((
                core,
                "Audio-Stream: Übergabe des Kerns fehlgeschlagen".into(),
            )));
        }
        Ok(AudioOutput {
            stream,
            info,
            shared,
            handback,
            inbound,
        })
    }
}

type StreamParts = (
    rtrb::Consumer<EngineCore>,
    Arc<StreamShared>,
    rtrb::Producer<EngineCore>,
);

fn build<T>(device: &Device, config: &StreamConfig, parts: StreamParts) -> Result<Stream, String>
where
    T: SizedSample + FromSample<f32>,
{
    let (mut inbound, shared, mut back_tx) = parts;
    let mut core: Option<EngineCore> = None;
    let lost = shared.clone();
    let channels = usize::from(config.channels);
    let mut scratch = vec![0.0f32; SCRATCH_FRAMES * 2].into_boxed_slice();
    device
        .build_output_stream(
            *config,
            move |data: &mut [T], info: &cpal::OutputCallbackInfo| {
                // Echtzeit-Callback: keine Allokation, kein Lock.
                // Kern übernehmen, sobald er da ist (nach dem Start des Streams).
                let giving_back = shared.give_back.load(Ordering::Acquire);
                if core.is_none()
                    && !giving_back
                    && let Ok(c) = inbound.pop()
                {
                    core = Some(c);
                }
                // Gerätewechsel: Kern zurückgeben (rtrb, vorab angelegt), danach Stille.
                if giving_back
                    && let Some(c) = core.take()
                    && let Err(rtrb::PushError::Full(c)) = back_tx.push(c)
                {
                    core = Some(c);
                }
                let Some(core) = core.as_mut() else {
                    data.fill(T::EQUILIBRIUM);
                    return;
                };
                let ts = info.timestamp();
                let latency = ts
                    .playback
                    .saturating_duration_since(ts.callback)
                    .as_nanos() as u64;
                core.set_block_played_at(crate::clock::now_nanos() + latency);
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
            move |err| {
                lost.lost.store(true, Ordering::Relaxed);
                eprintln!("Audio-Stream: {err}");
            },
            None,
        )
        .map_err(|e| format!("Stream-Aufbau: {e}"))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Gerätewechsel ohne Neustart: der Kern kommt aus dem Stream zurück und läuft auf einem
    /// neuen Stream weiter. Braucht ein Audiogerät: `cargo test -p studiocraft-engine -- --ignored`.
    #[test]
    #[ignore = "braucht ein Audiogerät"]
    fn core_moves_to_a_new_stream() {
        let prepared = PreparedOutput::open(None, 48_000).unwrap();
        let rate = prepared.info.sample_rate;
        let (core, ctl) = crate::engine(rate, studiocraft_project::TempoMap::default());
        let out = prepared.start(core).map_err(|b| b.1).unwrap();
        std::thread::sleep(Duration::from_millis(200));
        let core = out
            .take_core(Duration::from_secs(1))
            .map_err(|b| b.1)
            .unwrap();
        let again = PreparedOutput::open_with(&DeviceChoice {
            sample_rate: Some(rate),
            block_frames: Some(512),
            ..DeviceChoice::default()
        })
        .unwrap();
        assert_eq!(again.info.sample_rate, rate);
        let out = again.start(core).map_err(|b| b.1).unwrap();
        std::thread::sleep(Duration::from_millis(200));
        assert!(!out.device_lost());
        // The same control still reaches the moved core.
        assert_eq!(ctl.sample_rate, rate);
        let core = out
            .take_core(Duration::from_secs(1))
            .map_err(|b| b.1)
            .unwrap();
        drop(core);
    }
}
