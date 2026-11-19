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
}

enum Control {
    Start(PathBuf),
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
    _stream: Stream,
    pub info: InputInfo,
}

pub struct PreparedInput {
    device: Device,
    config: StreamConfig,
    format: SampleFormat,
    pub info: InputInfo,
}

impl PreparedInput {
    /// Standard-Eingangsgerät des Hosts mit genau `rate` (muss zur Ausgabe passen).
    pub fn open(host_name: Option<&str>, rate: u32) -> Result<Self, String> {
        let host = match host_name {
            None => cpal::default_host(),
            Some(name) => {
                let id = cpal::available_hosts()
                    .into_iter()
                    .find(|h| h.name().eq_ignore_ascii_case(name))
                    .ok_or_else(|| format!("Audio-Host „{name}“ gibt es in diesem Build nicht"))?;
                cpal::host_from_id(id).map_err(|e| format!("Audio-Host „{name}“: {e}"))?
            }
        };
        let device = host
            .default_input_device()
            .ok_or("Kein Audio-Eingang gefunden")?;
        let name = device
            .description()
            .map(|d| d.name().to_owned())
            .unwrap_or_else(|_| "unbekannt".into());
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
            device,
            config,
            format,
            info,
        })
    }

    /// Startet Eingang und Recorder-Thread. Der dritte Teil speist das Mithören und gehört
    /// per [`crate::Command::SetMonitorInput`] in die Engine.
    pub fn start(self) -> Result<(AudioInput, Recorder, MonitorInput), String> {
        let Self {
            device,
            config,
            format,
            info,
        } = self;
        let capacity = config.sample_rate as usize * usize::from(config.channels) * RING_SECONDS;
        let (producer, consumer) = rtrb::RingBuffer::new(capacity);
        let (monitor_tx, monitor) = monitor_ring(
            config.channels,
            config.sample_rate,
            crate::core::SCRATCH_FRAMES,
        );
        let overruns = Arc::new(AtomicU64::new(0));
        let clock = Arc::new(CaptureClock::default());
        let (o, c) = (overruns.clone(), clock.clone());
        let stream = match format {
            SampleFormat::F32 => build::<f32>(&device, &config, producer, monitor_tx, o, c),
            SampleFormat::I16 => build::<i16>(&device, &config, producer, monitor_tx, o, c),
            SampleFormat::I32 => build::<i32>(&device, &config, producer, monitor_tx, o, c),
            SampleFormat::U16 => build::<u16>(&device, &config, producer, monitor_tx, o, c),
            SampleFormat::F64 => build::<f64>(&device, &config, producer, monitor_tx, o, c),
            other => Err(format!("Sampleformat {other} wird nicht unterstützt")),
        }?;
        stream.play().map_err(|e| format!("Eingang starten: {e}"))?;
        let recorder =
            Recorder::spawn_with_clock(consumer, info.channels, info.sample_rate, overruns, clock);
        Ok((
            AudioInput {
                _stream: stream,
                info,
            },
            recorder,
            monitor,
        ))
    }
}

fn build<T>(
    device: &Device,
    config: &StreamConfig,
    mut producer: rtrb::Producer<f32>,
    mut monitor: rtrb::Producer<f32>,
    overruns: Arc<AtomicU64>,
    clock: Arc<CaptureClock>,
) -> Result<Stream, String>
where
    T: SizedSample,
    f32: FromSample<T>,
{
    let channels = usize::from(config.channels.max(1));
    let mut frames_seen = 0u64;
    device
        .build_input_stream(
            *config,
            move |data: &[T], info: &cpal::InputCallbackInfo| {
                // Echtzeit: nur kopieren, nie warten.
                // Wann das erste Sample dieses Puffers erfasst wurde.
                let ts = info.timestamp();
                let latency = ts.callback.saturating_duration_since(ts.capture).as_nanos() as u64;
                clock.publish(frames_seen, now_nanos().saturating_sub(latency));
                frames_seen += (data.len() / channels) as u64;

                let n = data.len().min(producer.slots());
                if let Ok(mut chunk) = producer.write_chunk_uninit(n) {
                    let (a, b) = chunk.as_mut_slices();
                    let mut src = data.iter().map(|s| s.to_sample::<f32>());
                    for slot in a.iter_mut().chain(b.iter_mut()) {
                        slot.write(src.next().unwrap_or(0.0));
                    }
                    // SAFETY: alle `n` Plätze wurden oben beschrieben.
                    unsafe { chunk.commit_all() };
                }
                push_frames(
                    &mut monitor,
                    channels,
                    data.iter().map(|s| s.to_sample::<f32>()),
                );
                if n < data.len() {
                    overruns.fetch_add((data.len() - n) as u64, Ordering::Relaxed);
                }
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

    /// Beginnt eine Aufnahme nach `path`. Was vorher im Puffer lag, wird verworfen.
    pub fn start(&self, path: &Path) {
        let _ = self.control.send(Control::Start(path.to_path_buf()));
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
    samples: u64,
    first_captured_at: Option<u64>,
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
                a.samples += n as u64;
            }
            chunk.commit_all();
            consumed += n as u64;
        }
        match msg {
            Ok(Control::Start(path)) => {
                // Puffer leeren: die Aufnahme beginnt jetzt.
                let n = consumer.slots();
                if let Ok(c) = consumer.read_chunk(n) {
                    c.commit_all();
                    consumed += n as u64;
                }
                failed = None;
                let first_captured_at = clock.time_of(consumed / ch, rate);
                active = match hound::WavWriter::create(&path, spec) {
                    Ok(writer) => Some(Active {
                        writer,
                        path,
                        samples: 0,
                        first_captured_at,
                    }),
                    Err(e) => {
                        failed = Some(format!("{}: {e}", path.display()));
                        None
                    }
                };
            }
            Ok(Control::Stop(reply)) => {
                let result = match (active.take(), failed.take()) {
                    (_, Some(e)) => Err(e),
                    (None, None) => Err("Es lief keine Aufnahme".into()),
                    (Some(a), None) => {
                        let frames = a.samples / ch;
                        let first_captured_at = a.first_captured_at;
                        a.writer
                            .finalize()
                            .map(|()| Recording {
                                path: a.path,
                                frames,
                                channels,
                                sample_rate: rate,
                                first_captured_at,
                            })
                            .map_err(|e| e.to_string())
                    }
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
        rec.start(&path);
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
    }
}
