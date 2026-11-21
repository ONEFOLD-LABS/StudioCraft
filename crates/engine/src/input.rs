//! Audio-Eingang und Recorder.
//!
//! Der Eingangs-Callback (Echtzeit) schiebt alle Kanäle verschachtelt in einen
//! lock-freien Ringpuffer und veröffentlicht, wann sein erstes Sample erfasst
//! wurde. Ein eigener Recorder-Thread leert den Puffer laufend und schreibt
//! während einer Aufnahme direkt in eine WAV-Datei (32-Bit-Float, alle Kanäle
//! des Geräts). Welche Kanäle auf welche Spur gehen, entscheidet die
//! Oberfläche nach dem Stopp.

use std::{
    path::{Path, PathBuf},
    sync::{
        Arc,
        atomic::{AtomicU64, Ordering},
        mpsc,
    },
    thread::{self, JoinHandle},
    time::Duration,
};

use cpal::{
    Device, FromSample, SampleFormat, SizedSample, Stream, StreamConfig,
    traits::{DeviceTrait, HostTrait, StreamTrait},
};

use crate::clock::{CaptureClock, now_nanos};
use crate::meters::{InputLevels, MAX_INPUT_CHANNELS};
use crate::monitor::{MonitorInput, monitor_ring, push_frames};

/// Puffer zwischen Eingangs-Callback und Recorder in Sekunden.
const RING_SECONDS: usize = 4;

#[derive(Debug, Clone)]
pub struct InputInfo {
    pub host: String,
    pub device: String,
    pub sample_rate: u32,
    pub channels: u16,
}

/// Fertige Aufnahme: eine WAV-Datei mit allen Eingangskanälen.
#[derive(Debug, Clone, PartialEq)]
pub struct Recording {
    pub path: PathBuf,
    pub frames: u64,
    pub channels: u16,
    pub sample_rate: u32,
    /// Wann der erste Frame erfasst wurde (Uhr aus [`crate::clock`]), falls bekannt.
    pub first_captured_at: Option<u64>,
    /// Die Aufnahme wurde unterbrochen (z. B. Platte voll): `frames` ist, was bis dahin sicher
    /// in der Datei steht.
    pub problem: Option<String>,
}

/// Wie oft der WAV-Kopf während der Aufnahme nachgeführt wird: Nach einem Absturz ist die
/// Datei bis dahin lesbar.
const FLUSH_EVERY: Duration = if cfg!(test) {
    Duration::from_millis(20)
} else {
    Duration::from_secs(1)
};

enum Control {
    Start(PathBuf, mpsc::Sender<Result<(), String>>),
    Stop(mpsc::Sender<Result<Recording, String>>),
    Quit,
}

/// Steuerung des Recorder-Threads.
pub struct Recorder {
    control: mpsc::Sender<Control>,
    thread: Option<JoinHandle<()>>,
    /// Verlorene Samples, weil der Ringpuffer voll war.
    pub overruns: Arc<AtomicU64>,
    pub channels: u16,
    pub sample_rate: u32,
}

/// Laufender Eingang. Beim Drop endet der Stream.
pub struct AudioInput {
    _stream: InputStream,
    pub info: InputInfo,
    /// Pegel und Anzeige je Gerätekanal.
    pub levels: Arc<InputLevels>,
}

pub struct PreparedInput {
    backend: Backend,
    pub info: InputInfo,
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

/// Laufender Eingang über cpal oder den eigenen WASAPI-Pfad; beim Drop endet er.
enum InputStream {
    Cpal(#[allow(dead_code)] Stream),
    #[cfg(windows)]
    Wasapi(#[allow(dead_code)] crate::wasapi::Running),
}

impl PreparedInput {
    /// Standard-Eingangsgerät des Hosts mit genau `rate` (muss zur Ausgabe passen).
    pub fn open(host_name: Option<&str>, rate: u32) -> Result<Self, String> {
        Self::open_device(host_name, None, rate)
    }

    /// Wie [`Self::open`], mit einem Eingabegerät nach Namen (`None`: Standard).
    pub fn open_device(
        host_name: Option<&str>,
        device: Option<&str>,
        rate: u32,
    ) -> Result<Self, String> {
        #[cfg(windows)]
        if let Some(mode) = host_name.and_then(crate::wasapi::Mode::from_host) {
            return Self::open_wasapi(mode, device, rate);
        }
        Self::open_cpal(host_name, device, rate)
    }

    /// Eigener WASAPI-Eingang (exklusiv oder mit geringer Latenz, wie die Ausgabe). Lehnt das
    /// Gerät den Modus ab, geht es eine Stufe zurück bis zum gemeinsamen Modus über cpal.
    #[cfg(windows)]
    fn open_wasapi(
        mode: crate::wasapi::Mode,
        device: Option<&str>,
        rate: u32,
    ) -> Result<Self, String> {
        use crate::wasapi::{Direction, Mode};
        let host = crate::output::host_by_name(Some(mode.host_name()))?;
        let dev = match device {
            None => host
                .default_input_device()
                .ok_or("Kein Audio-Eingang gefunden")?,
            Some(wanted) => host
                .input_devices()
                .map_err(|e| format!("Eingabegeräte: {e}"))?
                .find(|d| crate::output::device_name(d) == wanted)
                .ok_or_else(|| format!("Eingabegerät „{wanted}“ nicht gefunden"))?,
        };
        let name = crate::output::device_name(&dev);
        let id = dev
            .id()
            .map_err(|e| format!("Eingang „{name}“: {e}"))?
            .id()
            .to_owned();
        let mut modes = vec![mode];
        if mode == Mode::Exclusive {
            modes.push(Mode::LowLatency);
        }
        for m in modes {
            // Fehler hier: nächste Stufe; der gemeinsame Modus meldet den Grund selbst.
            if let Ok(prepared) = crate::wasapi::prepare(
                &id,
                Direction::Capture,
                m,
                rate,
                crate::output::DEFAULT_BLOCK_FRAMES,
            ) && prepared.format.rate == rate
            {
                let info = InputInfo {
                    host: m.host_name().to_owned(),
                    device: name,
                    sample_rate: prepared.format.rate,
                    channels: prepared.format.channels,
                };
                return Ok(Self {
                    backend: Backend::Wasapi(prepared),
                    info,
                });
            }
        }
        Self::open_cpal(Some("WASAPI"), Some(&name), rate)
    }

    fn open_cpal(host_name: Option<&str>, device: Option<&str>, rate: u32) -> Result<Self, String> {
        let host = crate::output::host_by_name(host_name)?;
        let device = match device {
            None => host
                .default_input_device()
                .ok_or("Kein Audio-Eingang gefunden")?,
            Some(wanted) => host
                .input_devices()
                .map_err(|e| format!("Eingabegeräte: {e}"))?
                .find(|d| crate::output::device_name(d) == wanted)
                .ok_or_else(|| format!("Eingabegerät „{wanted}“ nicht gefunden"))?,
        };
        let name = crate::output::device_name(&device);
        let default = device
            .default_input_config()
            .map_err(|e| format!("Eingang „{name}“: {e}"))?;
        let supported = device
            .supported_input_configs()
            .ok()
            .and_then(|mut it| {
                it.find(|c| {
                    c.channels() == default.channels()
                        && c.sample_format() == default.sample_format()
                })
                .and_then(|c| c.try_with_sample_rate(rate))
            })
            .or_else(|| (default.sample_rate() == rate).then_some(default))
            .ok_or_else(|| format!("Eingang „{name}“ unterstützt {rate} Hz nicht"))?;
        let format = supported.sample_format();
        let config: StreamConfig = supported.into();
        let info = InputInfo {
            host: host.id().name().to_owned(),
            device: name,
            sample_rate: config.sample_rate,
            channels: config.channels,
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

    /// Startet Eingang und Recorder-Thread. Der dritte Teil speist das Mithören und gehört
    /// per [`crate::Command::SetMonitorInput`] in die Engine.
    pub fn start(self) -> Result<(AudioInput, Recorder, MonitorInput), String> {
        let Self { backend, info } = self;
        let capacity = info.sample_rate as usize * usize::from(info.channels) * RING_SECONDS;
        let (producer, consumer) = rtrb::RingBuffer::new(capacity);
        let (monitor_tx, monitor) =
            monitor_ring(info.channels, info.sample_rate, crate::core::SCRATCH_FRAMES);
        let overruns = Arc::new(AtomicU64::new(0));
        let clock = Arc::new(CaptureClock::default());
        let levels = Arc::new(InputLevels::default());
        let sink = InputSink {
            producer,
            monitor: monitor_tx,
            overruns: overruns.clone(),
            clock: clock.clone(),
            levels: levels.clone(),
            channels: usize::from(info.channels.max(1)),
            frames_seen: 0,
        };
        let stream = match backend {
            Backend::Cpal {
                device,
                config,
                format,
            } => {
                let stream = match format {
                    SampleFormat::F32 => build::<f32>(&device, &config, sink),
                    SampleFormat::I16 => build::<i16>(&device, &config, sink),
                    SampleFormat::I32 => build::<i32>(&device, &config, sink),
                    SampleFormat::U16 => build::<u16>(&device, &config, sink),
                    SampleFormat::F64 => build::<f64>(&device, &config, sink),
                    other => Err(format!("Sampleformat {other} wird nicht unterstützt")),
                }?;
                stream.play().map_err(|e| format!("Eingang starten: {e}"))?;
                InputStream::Cpal(stream)
            }
            #[cfg(windows)]
            Backend::Wasapi(prepared) => {
                InputStream::Wasapi(prepared.start(crate::wasapi::Job::Capture(Box::new(sink)))?)
            }
        };
        let recorder =
            Recorder::spawn_with_clock(consumer, info.channels, info.sample_rate, overruns, clock);
        Ok((
            AudioInput {
                _stream: stream,
                info,
                levels,
            },
            recorder,
            monitor,
        ))
    }
}

/// Was der Eingangs-Callback braucht; gleich für cpal und den eigenen WASAPI-Pfad.
pub(crate) struct InputSink {
    producer: rtrb::Producer<f32>,
    monitor: rtrb::Producer<f32>,
    overruns: Arc<AtomicU64>,
    clock: Arc<CaptureClock>,
    levels: Arc<InputLevels>,
    channels: usize,
    frames_seen: u64,
}

impl InputSink {
    /// Nimmt einen verschachtelten Gerätepuffer an. `latency_ns`: wie lange das erste Sample
    /// schon zurückliegt. Echtzeit: nur kopieren, nie warten.
    pub(crate) fn push<T>(&mut self, data: &[T], latency_ns: u64)
    where
        T: SizedSample,
        f32: FromSample<T>,
    {
        let channels = self.channels;
        // Wann das erste Sample dieses Puffers erfasst wurde.
        self.clock
            .publish(self.frames_seen, now_nanos().saturating_sub(latency_ns));
        self.frames_seen += (data.len() / channels) as u64;

        // Pegel der Eingangskanalzüge, einmal je Puffer gelesen.
        let mut gains = [1.0f32; MAX_INPUT_CHANNELS];
        for (c, g) in gains.iter_mut().enumerate().take(channels) {
            *g = self.levels.gain(c);
        }
        let sample = |i: usize, s: &T| -> f32 {
            let v = s.to_sample::<f32>();
            gains.get(i % channels).map_or(v, |g| v * g)
        };
        let mut peaks = [0.0f32; MAX_INPUT_CHANNELS];
        for (i, s) in data.iter().enumerate() {
            if let Some(p) = peaks.get_mut(i % channels) {
                *p = p.max(sample(i, s).abs());
            }
        }
        for (c, &p) in peaks.iter().enumerate().take(channels) {
            self.levels.feed(c, p);
        }

        let n = data.len().min(self.producer.slots());
        if let Ok(mut chunk) = self.producer.write_chunk_uninit(n) {
            let (a, b) = chunk.as_mut_slices();
            let mut src = data.iter().enumerate().map(|(i, s)| sample(i, s));
            for slot in a.iter_mut().chain(b.iter_mut()) {
                slot.write(src.next().unwrap_or(0.0));
            }
            // SAFETY: alle `n` Plätze wurden oben beschrieben.
            unsafe { chunk.commit_all() };
        }
        push_frames(
            &mut self.monitor,
            channels,
            data.iter().enumerate().map(|(i, s)| sample(i, s)),
        );
        if n < data.len() {
            self.overruns
                .fetch_add((data.len() - n) as u64, Ordering::Relaxed);
        }
    }
}

fn build<T>(device: &Device, config: &StreamConfig, mut sink: InputSink) -> Result<Stream, String>
where
    T: SizedSample,
    f32: FromSample<T>,
{
    device
        .build_input_stream(
            *config,
            move |data: &[T], info: &cpal::InputCallbackInfo| {
                let ts = info.timestamp();
                let latency = ts.callback.saturating_duration_since(ts.capture).as_nanos() as u64;
                sink.push(data, latency);
            },
            |err| eprintln!("Audio-Eingang: {err}"),
            None,
        )
        .map_err(|e| format!("Eingang öffnen: {e}"))
}

impl Recorder {
    /// Startet den Thread, der `consumer` leert (ohne Zeitzuordnung; für Tests ohne Gerät).
    pub fn spawn(
        consumer: rtrb::Consumer<f32>,
        channels: u16,
        sample_rate: u32,
        overruns: Arc<AtomicU64>,
    ) -> Self {
        Self::spawn_with_clock(
            consumer,
            channels,
            sample_rate,
            overruns,
            Arc::new(CaptureClock::default()),
        )
    }

    /// Wie [`Self::spawn`], mit der Zeitzuordnung des Eingangs-Callbacks.
    pub fn spawn_with_clock(
        consumer: rtrb::Consumer<f32>,
        channels: u16,
        sample_rate: u32,
        overruns: Arc<AtomicU64>,
        clock: Arc<CaptureClock>,
    ) -> Self {
        let (tx, rx) = mpsc::channel();
        let thread = thread::Builder::new()
            .name("recorder".into())
            .spawn(move || recorder_loop(consumer, rx, channels, sample_rate, &clock))
            .ok();
        Self {
            control: tx,
            thread,
            overruns,
            channels,
            sample_rate,
        }
    }

    /// Beginnt eine Aufnahme nach `path`. Was vorher im Puffer lag, wird verworfen. Fehler,
    /// wenn die Datei sich nicht anlegen lässt: dann läuft keine Aufnahme.
    pub fn start(&self, path: &Path) -> Result<(), String> {
        let (tx, rx) = mpsc::channel();
        self.control
            .send(Control::Start(path.to_path_buf(), tx))
            .map_err(|_| "Recorder läuft nicht".to_string())?;
        rx.recv_timeout(Duration::from_secs(5))
            .map_err(|_| "Recorder antwortet nicht".to_string())?
    }

    /// Beendet die Aufnahme und wartet, bis die Datei vollständig geschrieben ist.
    pub fn stop(&self) -> Result<Recording, String> {
        let (tx, rx) = mpsc::channel();
        self.control
            .send(Control::Stop(tx))
            .map_err(|_| "Recorder läuft nicht".to_string())?;
        rx.recv_timeout(Duration::from_secs(10))
            .map_err(|_| "Recorder antwortet nicht".to_string())?
    }
}

impl Drop for Recorder {
    fn drop(&mut self) {
        let _ = self.control.send(Control::Quit);
        if let Some(t) = self.thread.take() {
            let _ = t.join();
        }
    }
}

struct Active {
    writer: hound::WavWriter<std::io::BufWriter<std::fs::File>>,
    path: PathBuf,
    first_captured_at: Option<u64>,
    last_flush: std::time::Instant,
}

/// Schließt die Datei ab, auch nach einem Schreibfehler, und liest ihre wirkliche Länge: Was
/// in der Datei steht, wird der Take (AGENTS.md: ein Absturz darf keine Takes kosten).
fn finish(
    a: Active,
    failed: Option<String>,
    channels: u16,
    rate: u32,
) -> Result<Recording, String> {
    let Active {
        writer,
        path,
        first_captured_at,
        ..
    } = a;
    let finalized = writer
        .finalize()
        .map_err(|e| format!("{}: {e}", path.display()));
    let problem = failed.or(finalized.err());
    let frames = hound::WavReader::open(&path).map(|r| u64::from(r.duration()));
    match frames {
        Ok(frames) if frames > 0 || problem.is_none() => Ok(Recording {
            path,
            frames,
            channels,
            sample_rate: rate,
            first_captured_at,
            problem,
        }),
        Ok(_) => Err(problem.unwrap_or_default()),
        Err(e) => Err(problem.unwrap_or_else(|| format!("{}: {e}", path.display()))),
    }
}

fn recorder_loop(
    mut consumer: rtrb::Consumer<f32>,
    rx: mpsc::Receiver<Control>,
    channels: u16,
    rate: u32,
    clock: &CaptureClock,
) {
    let ch = u64::from(channels.max(1));
    // Wie viele Samples insgesamt aus dem Puffer geholt wurden (für die Zeitzuordnung).
    let mut consumed = 0u64;
    let mut active: Option<Active> = None;
    let mut failed: Option<String> = None;
    let spec = hound::WavSpec {
        channels,
        sample_rate: rate,
        bits_per_sample: 32,
        sample_format: hound::SampleFormat::Float,
    };
    loop {
        let msg = rx.recv_timeout(Duration::from_millis(5));
        // Zuerst alles abholen, was bis jetzt angekommen ist.
        let n = consumer.slots();
        if let Ok(chunk) = consumer.read_chunk(n) {
            if let Some(a) = active.as_mut().filter(|_| failed.is_none()) {
                let (x, y) = chunk.as_slices();
                for &s in x.iter().chain(y) {
                    if let Err(e) = a.writer.write_sample(s) {
                        failed = Some(format!("{}: {e}", a.path.display()));
                        break;
                    }
                }
                // Kopf nachführen: nach einem Absturz ist die Datei bis hierher lesbar. Mitten
                // in einem Frame meldet hound das, schreibt aber trotzdem.
                if failed.is_none() && a.last_flush.elapsed() >= FLUSH_EVERY {
                    a.last_flush = std::time::Instant::now();
                    match a.writer.flush() {
                        Ok(()) | Err(hound::Error::UnfinishedSample) => {}
                        Err(e) => failed = Some(format!("{}: {e}", a.path.display())),
                    }
                }
            }
            chunk.commit_all();
            consumed += n as u64;
        }
        match msg {
            Ok(Control::Start(path, reply)) => {
                // Puffer leeren: die Aufnahme beginnt jetzt.
                let n = consumer.slots();
                if let Ok(c) = consumer.read_chunk(n) {
                    c.commit_all();
                    consumed += n as u64;
                }
                failed = None;
                let first_captured_at = clock.time_of(consumed / ch, rate);
                // Eine noch laufende Aufnahme abschließen, nicht verwerfen.
                if let Some(old) = active.take() {
                    let _ = finish(old, None, channels, rate);
                }
                active = match hound::WavWriter::create(&path, spec) {
                    Ok(writer) => {
                        let _ = reply.send(Ok(()));
                        Some(Active {
                            writer,
                            path,
                            first_captured_at,
                            last_flush: std::time::Instant::now(),
                        })
                    }
                    Err(e) => {
                        let _ = reply.send(Err(format!("{}: {e}", path.display())));
                        None
                    }
                };
            }
            Ok(Control::Stop(reply)) => {
                let result = match active.take() {
                    Some(a) => finish(a, failed.take(), channels, rate),
                    None => Err("Es lief keine Aufnahme".into()),
                };
                let _ = reply.send(result);
            }
            Ok(Control::Quit) | Err(mpsc::RecvTimeoutError::Disconnected) => {
                if let Some(a) = active.take() {
                    let _ = a.writer.finalize();
                }
                return;
            }
            Err(mpsc::RecvTimeoutError::Timeout) => {}
        }
    }
}

/// Liest eine Aufnahme und baut daraus Stereo für eine Spur: `first` ist der
/// erste Eingangskanal, `stereo` nimmt `first` und `first + 1`, sonst Mono auf beide Seiten.
pub fn extract_channels(rec: &Recording, first: u16, stereo: bool) -> Result<Vec<f32>, String> {
    let reader =
        hound::WavReader::open(&rec.path).map_err(|e| format!("{}: {e}", rec.path.display()))?;
    let ch = usize::from(rec.channels.max(1));
    let all: Vec<f32> = reader
        .into_samples::<f32>()
        .collect::<Result<_, _>>()
        .map_err(|e| e.to_string())?;
    let l = usize::from(first).min(ch - 1);
    let r = if stereo { (l + 1).min(ch - 1) } else { l };
    Ok(all.chunks_exact(ch).flat_map(|f| [f[l], f[r]]).collect())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn records_only_after_start_and_splits_channels() {
        let (mut tx, rx) = rtrb::RingBuffer::new(1024);
        let clock = Arc::new(CaptureClock::default());
        let rec =
            Recorder::spawn_with_clock(rx, 3, 48_000, Arc::new(AtomicU64::new(0)), clock.clone());
        // Vor dem Start: wird verworfen. Der Eingang meldet: Frame 0 bei 1 s erfasst.
        clock.publish(0, 1_000_000_000);
        for _ in 0..30 {
            tx.push(9.0).unwrap();
        }
        std::thread::sleep(Duration::from_millis(30));
        let dir = std::env::temp_dir().join(format!("sc-rec-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("take.wav");
        rec.start(&path).unwrap();
        std::thread::sleep(Duration::from_millis(30));
        for i in 0..100 {
            for c in 0..3 {
                tx.push(i as f32 + c as f32 * 0.25).unwrap();
            }
        }
        std::thread::sleep(Duration::from_millis(30));
        let done = rec.stop().unwrap();
        assert_eq!((done.frames, done.channels), (100, 3));
        // 10 verworfene Frames (30 Samples / 3 Kanäle) nach 1 s: erster Frame bei 1 s + 10/48000 s.
        assert_eq!(
            done.first_captured_at,
            Some(1_000_000_000 + 10 * 1_000_000_000 / 48_000)
        );
        let mono2 = extract_channels(&done, 2, false).unwrap();
        assert_eq!(&mono2[..4], &[0.5, 0.5, 1.5, 1.5]);
        let st = extract_channels(&done, 0, true).unwrap();
        assert_eq!(&st[..4], &[0.0, 0.25, 1.0, 1.25]);
        assert!(rec.stop().is_err(), "zweites Stopp ohne Aufnahme");
        assert_eq!(done.problem, None);
    }

    fn recorder(channels: u16) -> (rtrb::Producer<f32>, Recorder, PathBuf) {
        let (tx, rx) = rtrb::RingBuffer::new(1 << 16);
        let rec = Recorder::spawn_with_clock(
            rx,
            channels,
            48_000,
            Arc::new(AtomicU64::new(0)),
            Arc::new(CaptureClock::default()),
        );
        let dir =
            std::env::temp_dir().join(format!("sc-rec-{}-{}", std::process::id(), now_nanos()));
        std::fs::create_dir_all(&dir).unwrap();
        (tx, rec, dir)
    }

    #[test]
    fn a_file_that_cannot_be_created_fails_the_start() {
        let (_tx, rec, dir) = recorder(2);
        let err = rec.start(&dir.join("fehlt").join("take.wav")).unwrap_err();
        assert!(err.contains("take.wav"), "{err}");
        assert!(rec.stop().is_err(), "keine Aufnahme gestartet");
        // Danach geht eine Aufnahme wie gewohnt.
        rec.start(&dir.join("take.wav")).unwrap();
        assert!(rec.stop().is_ok());
    }

    #[test]
    fn the_file_stays_readable_while_recording() {
        // Ein Absturz mitten in der Aufnahme: was bis zum letzten Nachführen geschrieben war,
        // ist eine gültige WAV-Datei.
        let (mut tx, rec, dir) = recorder(2);
        let path = dir.join("take.wav");
        rec.start(&path).unwrap();
        for i in 0..4800 {
            tx.push(i as f32).unwrap();
        }
        std::thread::sleep(Duration::from_millis(40));
        tx.push(0.0).unwrap();
        tx.push(0.0).unwrap();
        std::thread::sleep(Duration::from_millis(40));
        let frames = hound::WavReader::open(&path).unwrap().duration();
        assert!(frames >= 2400, "{frames} Frames vor dem Stopp lesbar");
        assert_eq!(rec.stop().unwrap().frames, 2401);
    }

    #[test]
    fn a_write_error_keeps_what_was_recorded() {
        let dir = std::env::temp_dir().join(format!("sc-rec-fail-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("take.wav");
        let spec = hound::WavSpec {
            channels: 2,
            sample_rate: 48_000,
            bits_per_sample: 32,
            sample_format: hound::SampleFormat::Float,
        };
        let mut writer = hound::WavWriter::create(&path, spec).unwrap();
        for i in 0..2000 {
            writer.write_sample(i as f32).unwrap();
        }
        let a = Active {
            writer,
            path: path.clone(),
            first_captured_at: Some(7),
            last_flush: std::time::Instant::now(),
        };
        let r = finish(a, Some("Platte voll".into()), 2, 48_000).unwrap();
        assert_eq!((r.frames, r.first_captured_at), (1000, Some(7)));
        assert_eq!(r.problem.as_deref(), Some("Platte voll"));
        let back = extract_channels(&r, 1, false).unwrap();
        assert_eq!(&back[..4], &[1.0, 1.0, 3.0, 3.0]);
    }
}
