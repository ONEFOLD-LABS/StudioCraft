//! Audio-Ausgabe über cpal: ein Code-Pfad für alle Systeme.
//!
//! | System | Hosts |
//! | --- | --- |
//! | Linux, FreeBSD | PipeWire (Feature `pipewire`, Standard), ALSA, JACK (Feature `jack`) |
//! | Windows | WASAPI, eigenes WASAPI exklusiv und mit geringer Latenz ([`crate::wasapi`]), ASIO (Feature `asio`) |
//! | macOS | CoreAudio, JACK (Feature `jack`) |
//! | Web | Web Audio (wasm32) |

use std::sync::{
    Arc,
    atomic::{AtomicBool, Ordering},
};
use std::time::Duration;

use web_time::Instant;

use cpal::{
    BufferSize, Device, FromSample, Host, SampleFormat, SizedSample, Stream, StreamConfig,
    SupportedBufferSize,
    traits::{DeviceTrait, HostTrait, StreamTrait},
};

use crate::{EngineCore, EngineError};

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
pub(crate) fn host_by_name(name: Option<&str>) -> Result<Host, EngineError> {
    // Die eigenen WASAPI-Hosts zeigen dieselben Geräte wie der WASAPI-Host von cpal.
    #[cfg(windows)]
    if name.and_then(crate::wasapi::Mode::from_host).is_some() {
        return cpal::host_from_id(cpal::HostId::Wasapi).map_err(|e| EngineError::Host {
            name: "WASAPI".into(),
            detail: e.to_string(),
        });
    }
    match name {
        None => Ok(cpal::default_host()),
        Some(name) => {
            let id = cpal::available_hosts()
                .into_iter()
                .find(|h| h.name().eq_ignore_ascii_case(name))
                .ok_or_else(|| EngineError::HostMissing {
                    name: name.to_owned(),
                    available: available_hosts().into_iter().map(str::to_owned).collect(),
                })?;
            cpal::host_from_id(id).map_err(|e| EngineError::Host {
                name: name.to_owned(),
                detail: e.to_string(),
            })
        }
    }
}

pub(crate) fn device_name(d: &Device) -> String {
    d.description()
        .map(|d| d.name().to_owned())
        .unwrap_or_else(|_| "unbekannt".into())
}

/// Namen der Aus- und Eingabegeräte eines Hosts.
pub fn device_names(host: Option<&str>) -> Result<(Vec<String>, Vec<String>), EngineError> {
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
    /// Ausgangslatenz in Frames, die der Treiber meldet (Puffer plus Gerät), falls bekannt.
    pub latency_frames: Option<u32>,
    /// Warum statt der Wahl ein anderer Modus läuft (z. B. exklusiv abgelehnt): je
    /// abgelehntem Host der Grund. Leer, wenn die Wahl läuft.
    pub fallback: Vec<(String, crate::WasapiProblem)>,
}

impl OutputInfo {
    /// Ausgangslatenz in Millisekunden, falls bekannt.
    pub fn latency_ms(&self) -> Option<f32> {
        let frames = self.latency_frames?;
        (self.sample_rate > 0).then(|| frames as f32 * 1000.0 / self.sample_rate as f32)
    }
}

/// Gewähltes Gerät samt Konfiguration, noch ohne laufenden Stream.
/// Der Aufrufer kennt damit die Abtastrate, bevor er den Kern baut.
pub struct PreparedOutput {
    backend: Backend,
    pub info: OutputInfo,
}

enum Backend {
    Cpal {
        device: Device,
        config: StreamConfig,
        format: SampleFormat,
    },
    #[cfg(windows)]
    Wasapi(crate::wasapi::Prepared),
}

/// Der laufende Stream: über cpal oder über den eigenen WASAPI-Pfad.
/// Gehalten, damit der Stream läuft; beim Drop endet er.
enum RunningStream {
    Cpal(#[allow(dead_code)] Stream),
    #[cfg(windows)]
    Wasapi(#[allow(dead_code)] crate::wasapi::Running),
}

/// Laufende Ausgabe. Beim Drop wird der Stream beendet.
pub struct AudioOutput {
    stream: RunningStream,
    pub info: OutputInfo,
    shared: Arc<StreamShared>,
    /// Gibt der Audio-Thread den Kern zurück (Gerätewechsel), landet er hier.
    handback: rtrb::Consumer<EngineCore>,
    /// Weg des Kerns in den Audio-Thread (nach dem Aufbau des Streams, siehe `start`).
    inbound: rtrb::Producer<EngineCore>,
}

/// Zwischen Haupt- und Audio-Thread, nur Atomics.
#[derive(Default)]
pub(crate) struct StreamShared {
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
    pub fn take_core(mut self, timeout: Duration) -> Result<EngineCore, Box<(Self, EngineError)>> {
        self.shared.give_back.store(true, Ordering::Release);
        // Im Browser läuft der Audio-Rückruf im selben Thread: Warten käme nie zum Ziel, und
        // `thread::sleep` gibt es dort nicht. Der Kern kommt nur zurück, wenn er schon da ist.
        #[cfg(target_arch = "wasm32")]
        let timeout = {
            let _ = timeout;
            Duration::ZERO
        };
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
                return Err(Box::new((self, EngineError::NotResponding)));
            }
            #[cfg(not(target_arch = "wasm32"))]
            std::thread::sleep(Duration::from_millis(2));
        }
    }
}

/// Namen aller Hosts, die in diesen Build einkompiliert sind.
pub fn available_hosts() -> Vec<&'static str> {
    #[allow(unused_mut)]
    let mut hosts: Vec<&'static str> = cpal::available_hosts()
        .into_iter()
        .map(|h| h.name())
        .collect();
    #[cfg(windows)]
    hosts.extend(crate::wasapi::Mode::ALL.iter().map(|m| m.host_name()));
    hosts
}

impl PreparedOutput {
    /// Wählt Host und Standard-Ausgabegerät. Unterstützt das Gerät
    /// `preferred_rate`, wird diese genommen, sonst seine Standardrate.
    pub fn open(host_name: Option<&str>, preferred_rate: u32) -> Result<Self, EngineError> {
        Self::open_with(&DeviceChoice {
            host: host_name.map(str::to_owned),
            sample_rate: Some(preferred_rate),
            ..DeviceChoice::default()
        })
    }

    /// Wie [`Self::open`], mit Gerät, Abtastrate und Blockgröße aus `choice`. Ein genanntes
    /// Gerät, das es nicht (mehr) gibt, ist ein Fehler.
    pub fn open_with(choice: &DeviceChoice) -> Result<Self, EngineError> {
        #[cfg(windows)]
        if let Some(mode) = choice
            .host
            .as_deref()
            .and_then(crate::wasapi::Mode::from_host)
        {
            return Self::open_wasapi(choice, mode);
        }
        Self::open_cpal(choice)
    }

    /// Eigener WASAPI-Pfad. Lehnt das Gerät den Modus ab (belegt, Format, Richtlinie), geht es
    /// eine Stufe zurück: exklusiv → geringe Latenz → gemeinsamer Modus über cpal. Der Grund
    /// steht in [`OutputInfo::note`].
    #[cfg(windows)]
    fn open_wasapi(choice: &DeviceChoice, mode: crate::wasapi::Mode) -> Result<Self, EngineError> {
        use crate::wasapi::{Direction, Mode};
        let host = host_by_name(choice.host.as_deref())?;
        let device = match &choice.output {
            None => host
                .default_output_device()
                .ok_or(EngineError::NoDevice { input: false })?,
            Some(name) => host
                .output_devices()
                .map_err(|e| EngineError::DeviceList {
                    input: false,
                    detail: e.to_string(),
                })?
                .find(|d| device_name(d) == *name)
                .ok_or_else(|| EngineError::DeviceNotFound {
                    input: false,
                    name: name.clone(),
                })?,
        };
        let name = device_name(&device);
        let id = device
            .id()
            .map_err(|e| EngineError::Device {
                input: false,
                name: name.clone(),
                detail: e.to_string(),
            })?
            .id()
            .to_owned();
        let rate = choice.sample_rate.unwrap_or(48_000);
        let block = choice.block_frames.unwrap_or(DEFAULT_BLOCK_FRAMES);
        let mut notes = Vec::new();
        let mut modes = vec![mode];
        if mode == Mode::Exclusive {
            modes.push(Mode::LowLatency);
        }
        for m in modes {
            match crate::wasapi::prepare(&id, Direction::Render, m, rate, block) {
                Ok(prepared) => {
                    let f = prepared.format;
                    let info = OutputInfo {
                        host: m.host_name().to_owned(),
                        device: name,
                        sample_rate: f.rate,
                        channels: f.channels,
                        block_frames: Some(f.period_frames),
                        sample_format: f.sample.name().to_owned(),
                        latency_frames: Some(f.latency_frames),
                        fallback: notes,
                    };
                    return Ok(Self {
                        backend: Backend::Wasapi(prepared),
                        info,
                    });
                }
                Err(e) => notes.push((m.host_name().to_owned(), e)),
            }
        }
        // Letzte Stufe: gemeinsamer Modus über cpal, gleiches Gerät.
        let mut shared = Self::open_cpal(&DeviceChoice {
            host: Some("WASAPI".into()),
            output: Some(name),
            ..choice.clone()
        })?;
        shared.info.fallback = notes;
        Ok(shared)
    }

    fn open_cpal(choice: &DeviceChoice) -> Result<Self, EngineError> {
        let host = host_by_name(choice.host.as_deref())?;
        let preferred_rate = choice.sample_rate.unwrap_or(48_000);
        let device = match &choice.output {
            None => host
                .default_output_device()
                .ok_or(EngineError::NoDevice { input: false })?,
            Some(name) => host
                .output_devices()
                .map_err(|e| EngineError::DeviceList {
                    input: false,
                    detail: e.to_string(),
                })?
                .find(|d| device_name(d) == *name)
                .ok_or_else(|| EngineError::DeviceNotFound {
                    input: false,
                    name: name.clone(),
                })?,
        };
        let device_name = device_name(&device);

        let default = device
            .default_output_config()
            .map_err(|e| EngineError::Device {
                input: false,
                name: device_name.clone(),
                detail: e.to_string(),
            })?;
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
            // cpal nennt die Latenz erst im laufenden Callback; im gemeinsamen Modus kommt
            // der Windows-Mixer dazu. Lieber keine Zahl als eine zu kleine.
            latency_frames: None,
            fallback: Vec::new(),
        };
        Ok(Self {
            backend: Backend::Cpal {
                device,
                config,
                format,
            },
            info,
        })
    }

    /// Startet den Stream; `core` muss mit `info.sample_rate` laufen. Der Kern geht erst in
    /// den Audio-Thread, wenn der Stream läuft; scheitert der Start, kommt er mit dem Fehler
    /// zurück und wird nicht zerstört (Plugins, deren Fenster die Oberfläche hält).
    pub fn start(self, core: EngineCore) -> Result<AudioOutput, Box<(EngineCore, EngineError)>> {
        let Self { backend, info } = self;
        let shared = Arc::new(StreamShared::default());
        let (back_tx, handback) = rtrb::RingBuffer::new(1);
        let (mut inbound, inbound_rx) = rtrb::RingBuffer::new(1);
        let renderer = Renderer {
            inbound: inbound_rx,
            shared: shared.clone(),
            back_tx,
            core: None,
            channels: usize::from(info.channels),
            scratch: vec![0.0f32; SCRATCH_FRAMES * 2].into_boxed_slice(),
        };
        let stream = match backend {
            Backend::Cpal {
                device,
                config,
                format,
            } => {
                let stream = match format {
                    SampleFormat::F32 => build::<f32>(&device, &config, renderer),
                    SampleFormat::F64 => build::<f64>(&device, &config, renderer),
                    SampleFormat::I16 => build::<i16>(&device, &config, renderer),
                    SampleFormat::I32 => build::<i32>(&device, &config, renderer),
                    SampleFormat::U16 => build::<u16>(&device, &config, renderer),
                    SampleFormat::U8 => build::<u8>(&device, &config, renderer),
                    other => Err(EngineError::SampleFormat(other.to_string())),
                };
                let stream = match stream {
                    Ok(s) => s,
                    Err(e) => return Err(Box::new((core, e))),
                };
                if let Err(e) = stream.play() {
                    return Err(Box::new((
                        core,
                        EngineError::StreamStart {
                            input: false,
                            detail: e.to_string(),
                        },
                    )));
                }
                RunningStream::Cpal(stream)
            }
            #[cfg(windows)]
            Backend::Wasapi(prepared) => {
                match prepared.start(crate::wasapi::Job::Render(Box::new(renderer))) {
                    Ok(running) => RunningStream::Wasapi(running),
                    Err(e) => return Err(Box::new((core, EngineError::Wasapi(e)))),
                }
            }
        };
        // Platz für genau einen Kern, der Ring ist leer: geht immer.
        if let Err(rtrb::PushError::Full(core)) = inbound.push(core) {
            return Err(Box::new((core, EngineError::Handover)));
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

/// So viele Gerätekanäle werden beschrieben; weitere bleiben still.
const MAX_DEVICE_CHANNELS: usize = 64;

/// Addiert einen Stereo-Bus auf seine Gerätekanäle: (erster Kanal, Stereo). Mono-Busse und
/// Mono-Geräte bekommen die Mitte. Liegt der Hauptmix außerhalb des Geräts, geht er auf die
/// ersten Kanäle (nie still, nur weil ein Gerät weniger Kanäle hat).
fn mix_bus(
    acc: &mut [f32],
    channels: usize,
    (first, stereo): (u16, bool),
    lr: [f32; 2],
    main: bool,
) {
    let n = channels.min(acc.len());
    let mut first = usize::from(first);
    let width = if stereo && n > 1 { 2 } else { 1 };
    if first + width > n {
        if !main {
            return;
        }
        first = 0;
    }
    if width == 2 {
        acc[first] += lr[0];
        acc[first + 1] += lr[1];
    } else {
        acc[first] += (lr[0] + lr[1]) * 0.5;
    }
}

/// Was der Audio-Callback braucht: Weg des Kerns hinein und zurück, Gerätekanäle und ein
/// vorab angelegter Puffer. Gleich für cpal und den eigenen WASAPI-Pfad.
pub(crate) struct Renderer {
    inbound: rtrb::Consumer<EngineCore>,
    shared: Arc<StreamShared>,
    back_tx: rtrb::Producer<EngineCore>,
    core: Option<EngineCore>,
    channels: usize,
    scratch: Box<[f32]>,
}

impl Renderer {
    /// Füllt einen verschachtelten Gerätepuffer. `latency_ns`: bis dieser Puffer hörbar ist.
    /// Echtzeit: keine Allokation, kein Lock.
    pub(crate) fn render<T>(&mut self, data: &mut [T], latency_ns: u64)
    where
        T: SizedSample + FromSample<f32>,
    {
        let channels = self.channels.max(1);
        // Kern übernehmen, sobald er da ist (nach dem Start des Streams).
        let giving_back = self.shared.give_back.load(Ordering::Acquire);
        if self.core.is_none()
            && !giving_back
            && let Ok(c) = self.inbound.pop()
        {
            self.core = Some(c);
        }
        // Gerätewechsel: Kern zurückgeben (rtrb, vorab angelegt), danach Stille.
        if giving_back
            && let Some(c) = self.core.take()
            && let Err(rtrb::PushError::Full(c)) = self.back_tx.push(c)
        {
            self.core = Some(c);
        }
        let Some(core) = self.core.as_mut() else {
            data.fill(T::EQUILIBRIUM);
            return;
        };
        core.set_block_played_at(crate::clock::now_nanos() + latency_ns);
        for out in data.chunks_mut(SCRATCH_FRAMES * channels) {
            let frames = out.len() / channels;
            let block = &mut self.scratch[..frames * 2];
            core.process(block);
            // Hauptmix und weitere Ausgangsbusse auf ihre Gerätekanäle (Audio-
            // Anschlüsse); überlappende Busse addieren sich.
            let layout = core.output_layout();
            let main = layout.first().copied().unwrap_or((0, true));
            for (i, frame) in out.chunks_exact_mut(channels).enumerate() {
                let mut acc = [0.0f32; MAX_DEVICE_CHANNELS];
                let lr = [block[i * 2], block[i * 2 + 1]];
                mix_bus(&mut acc, channels, main, lr, true);
                for (k, &bus) in layout.iter().enumerate().skip(1) {
                    let b = core.hw_block(k);
                    if let Some(lr) = b.get(i * 2..i * 2 + 2) {
                        mix_bus(&mut acc, channels, bus, [lr[0], lr[1]], false);
                    }
                }
                for (s, a) in frame.iter_mut().zip(acc.iter()) {
                    *s = T::from_sample(*a);
                }
                for s in frame.iter_mut().skip(MAX_DEVICE_CHANNELS) {
                    *s = T::EQUILIBRIUM;
                }
            }
        }
    }

    /// Das Gerät ist weg (Fehler des Streams).
    #[cfg_attr(not(windows), allow(dead_code))]
    pub(crate) fn mark_lost(&self) {
        self.shared.lost.store(true, Ordering::Relaxed);
    }
}

fn build<T>(
    device: &Device,
    config: &StreamConfig,
    mut renderer: Renderer,
) -> Result<Stream, EngineError>
where
    T: SizedSample + FromSample<f32>,
{
    let lost = renderer.shared.clone();
    device
        .build_output_stream(
            *config,
            move |data: &mut [T], info: &cpal::OutputCallbackInfo| {
                let ts = info.timestamp();
                let latency = ts
                    .playback
                    .saturating_duration_since(ts.callback)
                    .as_nanos() as u64;
                renderer.render(data, latency);
            },
            move |err| {
                lost.lost.store(true, Ordering::Relaxed);
                eprintln!("Audio-Stream: {err}");
            },
            None,
        )
        .map_err(|e| EngineError::StreamBuild {
            input: false,
            detail: e.to_string(),
        })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn busses_land_on_their_device_channels() {
        let mut acc = [0.0f32; MAX_DEVICE_CHANNELS];
        // Main on 3/4 of a 4-channel device, a mono bus on 1, a bus beyond the device.
        mix_bus(&mut acc, 4, (2, true), [0.5, -0.5], true);
        mix_bus(&mut acc, 4, (0, false), [1.0, 0.0], false);
        mix_bus(&mut acc, 4, (4, true), [1.0, 1.0], false);
        assert_eq!(&acc[..4], &[0.5, 0.0, 0.5, -0.5]);
        // Main beyond a stereo device falls back to 1/2; mono device gets the middle.
        let mut acc = [0.0f32; MAX_DEVICE_CHANNELS];
        mix_bus(&mut acc, 2, (6, true), [0.2, 0.4], true);
        assert_eq!(&acc[..2], &[0.2, 0.4]);
        let mut acc = [0.0f32; MAX_DEVICE_CHANNELS];
        mix_bus(&mut acc, 1, (0, true), [0.2, 0.4], true);
        assert!((acc[0] - 0.3).abs() < 1e-6);
    }

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

    /// Eigener WASAPI-Pfad auf dem Standardgerät: beide Modi öffnen, kurz laufen lassen, den
    /// Kern zurückholen. Braucht Windows mit Audiogerät:
    /// `cargo test -p studiocraft-engine -- --ignored own_wasapi`.
    #[cfg(windows)]
    #[test]
    #[ignore = "braucht ein Audiogerät unter Windows"]
    fn own_wasapi_hosts_play_and_hand_the_core_back() {
        for mode in crate::wasapi::Mode::ALL {
            let prepared = PreparedOutput::open_with(&DeviceChoice {
                host: Some(mode.host_name().into()),
                block_frames: Some(128),
                ..DeviceChoice::default()
            })
            .unwrap();
            let info = prepared.info.clone();
            eprintln!(
                "{}: {} Hz, Block {:?}, Latenz {:?} ms, {}, Rückfall {:?}",
                info.host,
                info.sample_rate,
                info.block_frames,
                info.latency_ms(),
                info.sample_format,
                info.fallback
            );
            let (core, _ctl) =
                crate::engine(info.sample_rate, studiocraft_project::TempoMap::default());
            let out = prepared.start(core).map_err(|b| b.1).unwrap();
            std::thread::sleep(Duration::from_millis(300));
            assert!(!out.device_lost());
            let core = out
                .take_core(Duration::from_secs(1))
                .map_err(|b| b.1)
                .unwrap();
            drop(core);
        }
    }

    #[test]
    fn latency_is_reported_in_milliseconds() {
        let mut info = OutputInfo {
            host: "x".into(),
            device: "y".into(),
            sample_rate: 48_000,
            channels: 2,
            block_frames: Some(128),
            sample_format: "f32".into(),
            latency_frames: Some(144),
            fallback: Vec::new(),
        };
        assert_eq!(info.latency_ms(), Some(3.0));
        info.latency_frames = None;
        assert_eq!(info.latency_ms(), None);
        info.latency_frames = Some(10);
        info.sample_rate = 0;
        assert_eq!(info.latency_ms(), None);
    }
}
