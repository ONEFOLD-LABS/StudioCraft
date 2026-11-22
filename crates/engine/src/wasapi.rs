//! Eigener WASAPI-Pfad für Windows mit niedriger Latenz (Issue #110).
//!
//! cpal öffnet unter Windows jeden Stream im gemeinsamen Modus; der Ton läuft dann durch den
//! Windows-Mixer. Hier laufen zwei weitere Hosts auf den Treibern, die Windows mitbringt:
//!
//! - **WASAPI Exclusive**: ereignisgesteuert im exklusiven Modus. Die App bekommt den
//!   Gerätepuffer direkt, mit der Blockgröße der Wahl (mindestens der Geräteperiode).
//! - **WASAPI Low Latency**: gemeinsamer Modus über `IAudioClient3` mit der kleinsten Periode,
//!   die der Treiber anbietet (Windows 10 und neuer). Andere Apps spielen weiter.
//!
//! Lehnt das Gerät den exklusiven Modus ab, geht es eine Stufe zurück; das entscheiden
//! [`crate::output`] und [`crate::input`]. Alles COM läuft in einem eigenen Audio-Thread je
//! Stream: Er richtet das Gerät ein, meldet das Format, wartet auf den Start und rendert bzw.
//! nimmt auf, bis der Stream fällt. Keine COM-Objekte wechseln den Thread.
//!
//! Dieses Modul ist neben `crates/plugin-host` die einzige Stelle der Engine mit nennenswert
//! `unsafe`: WASAPI-Aufrufe und Gerätepuffer. Jeder Block hat einen `SAFETY:`-Kommentar.

/// Welcher eigene WASAPI-Modus.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Mode {
    Exclusive,
    LowLatency,
}

impl Mode {
    /// Alle Modi in der Reihenfolge der Hostliste.
    pub const ALL: [Self; 2] = [Self::Exclusive, Self::LowLatency];

    /// Name in der Hostliste. Feste Kennung (steht in den Einstellungen), nicht übersetzt.
    pub fn host_name(self) -> &'static str {
        match self {
            Self::Exclusive => "WASAPI Exclusive",
            Self::LowLatency => "WASAPI Low Latency",
        }
    }

    pub fn from_host(name: &str) -> Option<Self> {
        Self::ALL
            .into_iter()
            .find(|m| m.host_name().eq_ignore_ascii_case(name))
    }
}

/// Wie die Samples im Gerätepuffer liegen.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SampleKind {
    F32,
    I32,
    /// 24 gültige Bits in 32-Bit-Containern (linksbündig, geschrieben wie `i32`).
    I24In32,
    I16,
}

impl SampleKind {
    /// Reihenfolge, in der der exklusive Modus Formate probiert.
    #[cfg_attr(not(windows), allow(dead_code))]
    pub(crate) const PREFERRED: [Self; 4] = [Self::F32, Self::I32, Self::I24In32, Self::I16];

    pub fn name(self) -> &'static str {
        match self {
            Self::F32 => "f32",
            Self::I32 => "i32",
            Self::I24In32 => "i24 (32)",
            Self::I16 => "i16",
        }
    }

    /// (Bits je Container, gültige Bits, Gleitkomma).
    #[cfg_attr(not(windows), allow(dead_code))]
    pub(crate) fn bits(self) -> (u16, u16, bool) {
        match self {
            Self::F32 => (32, 32, true),
            Self::I32 => (32, 32, false),
            Self::I24In32 => (32, 24, false),
            Self::I16 => (16, 16, false),
        }
    }

    /// Erkennt ein Format aus Containerbits, gültigen Bits und Gleitkomma-Kennung.
    #[cfg_attr(not(windows), allow(dead_code))]
    pub(crate) fn from_bits(container: u16, valid: u16, float: bool) -> Option<Self> {
        let valid = if valid == 0 { container } else { valid };
        match (container, valid, float) {
            (32, 32, true) => Some(Self::F32),
            (32, 32, false) => Some(Self::I32),
            (32, 24, false) => Some(Self::I24In32),
            (16, 16, false) => Some(Self::I16),
            _ => None,
        }
    }
}

/// Was ein eingerichteter Stream tatsächlich bekommen hat.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct StreamFormat {
    pub rate: u32,
    pub channels: u16,
    pub sample: SampleKind,
    /// Frames je Ereignis (Periode).
    pub period_frames: u32,
    /// Größe des Gerätepuffers in Frames.
    pub buffer_frames: u32,
    /// Latenz in Frames: Puffer plus die vom Treiber gemeldete Stream-Latenz.
    pub latency_frames: u32,
}

/// 100-ns-Einheiten (REFERENCE_TIME) für `frames` bei `rate`, gerundet.
#[cfg_attr(not(windows), allow(dead_code))]
pub(crate) fn frames_to_hns(frames: u32, rate: u32) -> i64 {
    if rate == 0 {
        return 0;
    }
    ((u64::from(frames) * 10_000_000 + u64::from(rate) / 2) / u64::from(rate)) as i64
}

/// Frames für `hns` 100-ns-Einheiten bei `rate`, gerundet.
#[cfg_attr(not(windows), allow(dead_code))]
pub(crate) fn hns_to_frames(hns: i64, rate: u32) -> u32 {
    let hns = u64::try_from(hns).unwrap_or(0);
    let frames = (hns * u64::from(rate) + 5_000_000) / 10_000_000;
    u32::try_from(frames).unwrap_or(u32::MAX)
}

/// Periode im exklusiven Modus: die gewünschte Blockgröße, aber nie unter dem Minimum des
/// Geräts (100-ns-Einheiten).
#[cfg_attr(not(windows), allow(dead_code))]
pub(crate) fn exclusive_period_hns(block: u32, rate: u32, min_hns: i64) -> i64 {
    frames_to_hns(block, rate).max(min_hns).max(1)
}

/// Periode im gemeinsamen Modus mit `IAudioClient3`: die gewünschte Blockgröße, auf ein
/// Vielfaches der Grundperiode aufgerundet und auf den Bereich des Treibers begrenzt.
#[cfg_attr(not(windows), allow(dead_code))]
pub(crate) fn shared_period(wanted: u32, fundamental: u32, min: u32, max: u32) -> u32 {
    let (min, max) = (min.min(max), max.max(min));
    if fundamental == 0 {
        return wanted.clamp(min, max);
    }
    let rounded = wanted.div_ceil(fundamental).saturating_mul(fundamental);
    let clamped = rounded.clamp(min, max);
    // Der Bereich des Treibers besteht aus Vielfachen der Grundperiode; außer beim Minimum
    // abrunden, falls das Begrenzen ein krummes Maß ergab.
    if clamped % fundamental == 0 || clamped == min {
        clamped
    } else {
        (clamped / fundamental * fundamental).max(min)
    }
}

#[cfg(windows)]
pub(crate) use imp::{Direction, Job, Prepared, Running, prepare};

#[cfg(windows)]
mod imp {
    use std::sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
        mpsc,
    };
    use std::thread::JoinHandle;
    use std::time::Duration;

    use crate::WasapiProblem;

    use windows::Win32::Foundation::{CloseHandle, HANDLE, S_OK, WAIT_OBJECT_0};
    use windows::Win32::Media::Audio::{
        AUDCLNT_BUFFERFLAGS_SILENT, AUDCLNT_E_BUFFER_SIZE_NOT_ALIGNED, AUDCLNT_E_DEVICE_IN_USE,
        AUDCLNT_E_DEVICE_INVALIDATED, AUDCLNT_E_EXCLUSIVE_MODE_NOT_ALLOWED,
        AUDCLNT_E_UNSUPPORTED_FORMAT, AUDCLNT_SHAREMODE_EXCLUSIVE,
        AUDCLNT_STREAMFLAGS_EVENTCALLBACK, IAudioCaptureClient, IAudioClient, IAudioClient3,
        IAudioRenderClient, IMMDevice, IMMDeviceEnumerator, MMDeviceEnumerator, WAVEFORMATEX,
        WAVEFORMATEXTENSIBLE, WAVEFORMATEXTENSIBLE_0,
    };
    use windows::Win32::Media::KernelStreaming::{
        KSDATAFORMAT_SUBTYPE_PCM, WAVE_FORMAT_EXTENSIBLE,
    };
    use windows::Win32::Media::Multimedia::{
        KSDATAFORMAT_SUBTYPE_IEEE_FLOAT, WAVE_FORMAT_IEEE_FLOAT,
    };
    use windows::Win32::System::Com::{
        CLSCTX_ALL, COINIT_MULTITHREADED, CoCreateInstance, CoInitializeEx, CoTaskMemFree,
        CoUninitialize,
    };
    use windows::Win32::System::Threading::{
        AvRevertMmThreadCharacteristics, AvSetMmThreadCharacteristicsW, CreateEventW,
        WaitForSingleObject,
    };
    use windows::core::{Interface, PCWSTR, w};

    use super::{
        Mode, SampleKind, StreamFormat, exclusive_period_hns, frames_to_hns, hns_to_frames,
        shared_period,
    };
    use crate::input::InputSink;
    use crate::output::Renderer;

    /// Richtung des Streams.
    #[derive(Debug, Clone, Copy, PartialEq, Eq)]
    pub(crate) enum Direction {
        Render,
        Capture,
    }

    /// Was der Audio-Thread nach dem Start bedient.
    pub(crate) enum Job {
        Render(Box<Renderer>),
        Capture(Box<InputSink>),
    }

    /// So lange wartet der Aufrufer auf Einrichtung und Start des Geräts.
    const SETUP_TIMEOUT: Duration = Duration::from_secs(5);
    /// Höchste Wartezeit auf ein Pufferereignis, danach wird das Gerät geprüft.
    const EVENT_TIMEOUT_MS: u32 = 500;

    /// Eingerichteter, noch nicht gestarteter Stream. Beim Drop endet der Audio-Thread.
    pub(crate) struct Prepared {
        pub format: StreamFormat,
        direction: Direction,
        go: Option<mpsc::Sender<Job>>,
        started: mpsc::Receiver<Result<(), WasapiProblem>>,
        stop: Arc<AtomicBool>,
        thread: Option<JoinHandle<()>>,
    }

    /// Laufender Stream. Beim Drop hält der Audio-Thread an und gibt das Gerät frei.
    pub(crate) struct Running {
        stop: Arc<AtomicBool>,
        thread: Option<JoinHandle<()>>,
    }

    impl Drop for Running {
        fn drop(&mut self) {
            self.stop.store(true, Ordering::Release);
            if let Some(t) = self.thread.take() {
                let _ = t.join();
            }
        }
    }

    impl Drop for Prepared {
        fn drop(&mut self) {
            // Ohne Start-Auftrag beendet sich der Thread, sobald der Sender fehlt.
            self.go = None;
            self.stop.store(true, Ordering::Release);
            if let Some(t) = self.thread.take() {
                let _ = t.join();
            }
        }
    }

    impl Prepared {
        /// Startet den Stream mit `job` (Renderer für die Ausgabe, Senke für den Eingang).
        pub(crate) fn start(mut self, job: Job) -> Result<Running, WasapiProblem> {
            let fits = matches!(
                (&job, self.direction),
                (Job::Render(_), Direction::Render) | (Job::Capture(_), Direction::Capture)
            );
            if !fits {
                return Err(WasapiProblem::WrongDirection);
            }
            let go = self.go.take().ok_or(WasapiProblem::AlreadyRunning)?;
            go.send(job).map_err(|_| WasapiProblem::ThreadGone)?;
            let result = self
                .started
                .recv_timeout(SETUP_TIMEOUT)
                .map_err(|_| WasapiProblem::NoStartAnswer)
                .and_then(|r| r);
            match result {
                Ok(()) => Ok(Running {
                    stop: self.stop.clone(),
                    thread: self.thread.take(),
                }),
                Err(e) => Err(e),
            }
        }
    }

    /// Richtet `endpoint_id` (WASAPI-Geräte-ID) im Modus `mode` in einem eigenen Audio-Thread
    /// ein. `rate` und `block` sind Wünsche; was das Gerät bietet, steht im Ergebnis.
    pub(crate) fn prepare(
        endpoint_id: &str,
        direction: Direction,
        mode: Mode,
        rate: u32,
        block: u32,
    ) -> Result<Prepared, WasapiProblem> {
        let id: Vec<u16> = endpoint_id.encode_utf16().chain(Some(0)).collect();
        let (ready_tx, ready_rx) = mpsc::channel();
        let (go_tx, go_rx) = mpsc::channel();
        let (started_tx, started_rx) = mpsc::channel();
        let stop = Arc::new(AtomicBool::new(false));
        let stop_thread = stop.clone();
        let thread = std::thread::Builder::new()
            .name("wasapi".into())
            .spawn(move || {
                thread_main(
                    &id,
                    direction,
                    mode,
                    (rate, block),
                    ready_tx,
                    go_rx,
                    started_tx,
                    &stop_thread,
                );
            })
            .map_err(|e| WasapiProblem::Thread(e.to_string()))?;
        let format = match ready_rx.recv_timeout(SETUP_TIMEOUT) {
            Ok(Ok(f)) => f,
            Ok(Err(e)) => {
                let _ = thread.join();
                return Err(e);
            }
            // Der Thread hängt im Treiber: nicht warten, er endet mit dem fehlenden Sender.
            Err(_) => return Err(WasapiProblem::NoAnswer),
        };
        Ok(Prepared {
            format,
            direction,
            go: Some(go_tx),
            started: started_rx,
            stop,
            thread: Some(thread),
        })
    }

    #[allow(clippy::too_many_arguments)]
    fn thread_main(
        id: &[u16],
        direction: Direction,
        mode: Mode,
        (rate, block): (u32, u32),
        ready: mpsc::Sender<Result<StreamFormat, WasapiProblem>>,
        go: mpsc::Receiver<Job>,
        started: mpsc::Sender<Result<(), WasapiProblem>>,
        stop: &AtomicBool,
    ) {
        // SAFETY: COM für diesen Thread (Multithreaded Apartment); jedes erfolgreiche
        // CoInitializeEx wird unten mit CoUninitialize ausgeglichen, nachdem alle
        // COM-Objekte dieses Threads freigegeben sind (sie leben nur in `run`).
        let com = unsafe { CoInitializeEx(None, COINIT_MULTITHREADED) };
        if com.is_err() {
            let _ = ready.send(Err(WasapiProblem::Com(com.to_string())));
            return;
        }
        run(id, direction, mode, (rate, block), ready, go, started, stop);
        // SAFETY: gleicht das erfolgreiche CoInitializeEx oben aus.
        unsafe { CoUninitialize() };
    }

    /// Ein Ereignis-Handle, das beim Drop geschlossen wird.
    struct Event(HANDLE);

    impl Drop for Event {
        fn drop(&mut self) {
            // SAFETY: das Handle stammt aus CreateEventW und wird nur hier geschlossen.
            let _ = unsafe { CloseHandle(self.0) };
        }
    }

    /// Ein Mix-Format aus GetMixFormat, das beim Drop freigegeben wird.
    struct MixFormat(*mut WAVEFORMATEX);

    impl Drop for MixFormat {
        fn drop(&mut self) {
            // SAFETY: der Zeiger stammt aus GetMixFormat (CoTaskMemAlloc) und wird nur hier
            // freigegeben.
            unsafe { CoTaskMemFree(Some(self.0.cast_const().cast())) };
        }
    }

    /// Kanäle, Rate, Sampleformat und Kanalmaske eines Mix-Formats.
    struct MixInfo {
        rate: u32,
        channels: u16,
        sample: Option<SampleKind>,
        mask: u32,
    }

    impl MixFormat {
        fn info(&self) -> MixInfo {
            // SAFETY: GetMixFormat liefert ein gültiges WAVEFORMATEX; die Felder sind gepackt,
            // daher ungeausgerichtet gelesen. Die erweiterte Form wird nur gelesen, wenn Tag
            // und cbSize sie ankündigen.
            let f = unsafe { self.0.read_unaligned() };
            let tag = u32::from(f.wFormatTag);
            if tag == WAVE_FORMAT_EXTENSIBLE && f.cbSize >= 22 {
                // SAFETY: siehe oben, die erweiterte Struktur ist laut cbSize vorhanden.
                let x = unsafe { self.0.cast::<WAVEFORMATEXTENSIBLE>().read_unaligned() };
                let sub = x.SubFormat;
                // SAFETY: bei PCM/Float steht im Union-Feld die Zahl der gültigen Bits.
                let valid = unsafe { x.Samples.wValidBitsPerSample };
                let float = sub == KSDATAFORMAT_SUBTYPE_IEEE_FLOAT;
                let pcm = sub == KSDATAFORMAT_SUBTYPE_PCM;
                MixInfo {
                    rate: f.nSamplesPerSec,
                    channels: f.nChannels,
                    sample: (float || pcm)
                        .then(|| SampleKind::from_bits(f.wBitsPerSample, valid, float))
                        .flatten(),
                    mask: x.dwChannelMask,
                }
            } else {
                let float = tag == WAVE_FORMAT_IEEE_FLOAT;
                let pcm = tag == 1;
                MixInfo {
                    rate: f.nSamplesPerSec,
                    channels: f.nChannels,
                    sample: (float || pcm)
                        .then(|| SampleKind::from_bits(f.wBitsPerSample, 0, float))
                        .flatten(),
                    mask: default_mask(f.nChannels),
                }
            }
        }
    }

    /// Kanalmaske, wenn das Gerät keine nennt: Mono in der Mitte, sonst vorne links/rechts.
    fn default_mask(channels: u16) -> u32 {
        match channels {
            1 => 0x4,
            2 => 0x3,
            n => (1u32 << u32::from(n.min(18))) - 1,
        }
    }

    fn extensible(rate: u32, channels: u16, kind: SampleKind, mask: u32) -> WAVEFORMATEXTENSIBLE {
        let (bits, valid, float) = kind.bits();
        let block_align = channels * (bits / 8);
        WAVEFORMATEXTENSIBLE {
            Format: WAVEFORMATEX {
                wFormatTag: WAVE_FORMAT_EXTENSIBLE as u16,
                nChannels: channels,
                nSamplesPerSec: rate,
                nAvgBytesPerSec: rate * u32::from(block_align),
                nBlockAlign: block_align,
                wBitsPerSample: bits,
                cbSize: 22,
            },
            Samples: WAVEFORMATEXTENSIBLE_0 {
                wValidBitsPerSample: valid,
            },
            dwChannelMask: mask,
            SubFormat: if float {
                KSDATAFORMAT_SUBTYPE_IEEE_FLOAT
            } else {
                KSDATAFORMAT_SUBTYPE_PCM
            },
        }
    }

    /// Der Grund für die häufigen WASAPI-Fehler; andere Meldungen von Windows, wie sie sind.
    fn describe(e: &windows::core::Error) -> WasapiProblem {
        let code = e.code();
        if code == AUDCLNT_E_DEVICE_IN_USE {
            WasapiProblem::InUse
        } else if code == AUDCLNT_E_EXCLUSIVE_MODE_NOT_ALLOWED {
            WasapiProblem::ExclusiveNotAllowed
        } else if code == AUDCLNT_E_UNSUPPORTED_FORMAT {
            WasapiProblem::FormatUnsupported
        } else if code == AUDCLNT_E_DEVICE_INVALIDATED {
            WasapiProblem::Gone
        } else {
            WasapiProblem::Os(e.to_string())
        }
    }

    /// Eingerichteter Client samt Dienst und Ereignis.
    struct Setup {
        mode: Mode,
        client: IAudioClient,
        event: Event,
        format: StreamFormat,
        render: Option<IAudioRenderClient>,
        capture: Option<IAudioCaptureClient>,
    }

    fn open_device(id: &[u16]) -> Result<IMMDevice, WasapiProblem> {
        // SAFETY: COM ist in diesem Thread eingerichtet; `id` ist nullterminiert und lebt
        // über den Aufruf hinaus.
        unsafe {
            let enumerator: IMMDeviceEnumerator =
                CoCreateInstance(&MMDeviceEnumerator, None, CLSCTX_ALL)
                    .map_err(|e| WasapiProblem::DeviceList(describe(&e).to_string()))?;
            enumerator
                .GetDevice(PCWSTR(id.as_ptr()))
                .map_err(|e| WasapiProblem::Device(describe(&e).to_string()))
        }
    }

    fn mix_format(client: &IAudioClient) -> Result<MixFormat, WasapiProblem> {
        // SAFETY: gültiger Client; das Ergebnis gehört uns und wird von MixFormat freigegeben.
        let p = unsafe { client.GetMixFormat() }.map_err(|e| describe(&e))?;
        if p.is_null() {
            return Err(WasapiProblem::NoFormat);
        }
        Ok(MixFormat(p))
    }

    /// Exklusiver Modus, ereignisgesteuert, Periode = Puffer.
    fn setup_exclusive(
        device: &IMMDevice,
        direction: Direction,
        rate: u32,
        block: u32,
    ) -> Result<(IAudioClient, StreamFormat), WasapiProblem> {
        // SAFETY: COM-Aufrufe auf gültigen Objekten dieses Threads; Formatzeiger zeigen auf
        // lokale Strukturen, die während der Aufrufe leben.
        unsafe {
            let client: IAudioClient = device
                .Activate(CLSCTX_ALL, None)
                .map_err(|e| describe(&e))?;
            let mix = mix_format(&client)?.info();
            let channels = mix.channels.max(1);
            let mask = if mix.mask != 0 {
                mix.mask
            } else {
                default_mask(channels)
            };
            // Ausgabe darf auf die Rate des Geräts ausweichen, der Eingang muss zur Ausgabe
            // passen.
            let mut rates = vec![rate];
            if direction == Direction::Render && mix.rate != rate {
                rates.push(mix.rate);
            }
            let chosen = rates.iter().find_map(|&r| {
                SampleKind::PREFERRED.into_iter().find_map(|kind| {
                    let f = extensible(r, channels, kind, mask);
                    let p = (&raw const f).cast::<WAVEFORMATEX>();
                    (client.IsFormatSupported(AUDCLNT_SHAREMODE_EXCLUSIVE, p, None) == S_OK)
                        .then_some((f, kind, r))
                })
            });
            let Some((fmt, kind, rate)) = chosen else {
                return Err(WasapiProblem::NoExclusiveFormat { channels, rate });
            };
            let p = (&raw const fmt).cast::<WAVEFORMATEX>();
            let mut min_hns = 0i64;
            client
                .GetDevicePeriod(None, Some(&mut min_hns))
                .map_err(|e| describe(&e))?;
            let period = exclusive_period_hns(block, rate, min_hns);
            let flags = AUDCLNT_STREAMFLAGS_EVENTCALLBACK;
            let client = match client.Initialize(
                AUDCLNT_SHAREMODE_EXCLUSIVE,
                flags,
                period,
                period,
                p,
                None,
            ) {
                Ok(()) => client,
                Err(e) if e.code() == AUDCLNT_E_BUFFER_SIZE_NOT_ALIGNED => {
                    // Der Treiber will eine ausgerichtete Puffergröße: die vorgeschlagene
                    // nehmen und mit einem frischen Client neu einrichten.
                    let frames = client.GetBufferSize().map_err(|e| describe(&e))?;
                    let aligned = frames_to_hns(frames, rate);
                    let again: IAudioClient = device
                        .Activate(CLSCTX_ALL, None)
                        .map_err(|e| describe(&e))?;
                    again
                        .Initialize(
                            AUDCLNT_SHAREMODE_EXCLUSIVE,
                            flags,
                            aligned,
                            aligned,
                            p,
                            None,
                        )
                        .map_err(|e| describe(&e))?;
                    again
                }
                Err(e) => return Err(describe(&e)),
            };
            let buffer = client.GetBufferSize().map_err(|e| describe(&e))?;
            Ok((
                client,
                StreamFormat {
                    rate,
                    channels,
                    sample: kind,
                    period_frames: buffer,
                    buffer_frames: buffer,
                    latency_frames: 0,
                },
            ))
        }
    }

    /// Gemeinsamer Modus mit kleiner Periode über IAudioClient3 (Format und Rate des
    /// Windows-Mixers).
    fn setup_low_latency(
        device: &IMMDevice,
        block: u32,
    ) -> Result<(IAudioClient, StreamFormat), WasapiProblem> {
        // SAFETY: COM-Aufrufe auf gültigen Objekten dieses Threads; der Formatzeiger aus
        // GetMixFormat lebt bis zum Ende dieses Blocks (MixFormat).
        unsafe {
            let client3: IAudioClient3 = device
                .Activate(CLSCTX_ALL, None)
                .map_err(|e| WasapiProblem::NoLowLatency(describe(&e).to_string()))?;
            let mix = mix_format(&client3)?;
            let info = mix.info();
            let sample = info.sample.ok_or(WasapiProblem::MixFormatUnsupported)?;
            let (mut default, mut fundamental, mut min, mut max) = (0u32, 0u32, 0u32, 0u32);
            client3
                .GetSharedModeEnginePeriod(
                    mix.0,
                    &mut default,
                    &mut fundamental,
                    &mut min,
                    &mut max,
                )
                .map_err(|e| describe(&e))?;
            let period = shared_period(block, fundamental, min, max);
            client3
                .InitializeSharedAudioStream(AUDCLNT_STREAMFLAGS_EVENTCALLBACK, period, mix.0, None)
                .map_err(|e| describe(&e))?;
            let client: IAudioClient = client3.cast().map_err(|e| describe(&e))?;
            let buffer = client.GetBufferSize().map_err(|e| describe(&e))?;
            Ok((
                client,
                StreamFormat {
                    rate: info.rate,
                    channels: info.channels.max(1),
                    sample,
                    period_frames: period,
                    buffer_frames: buffer,
                    latency_frames: 0,
                },
            ))
        }
    }

    fn setup(
        id: &[u16],
        direction: Direction,
        mode: Mode,
        rate: u32,
        block: u32,
    ) -> Result<Setup, WasapiProblem> {
        let device = open_device(id)?;
        let (client, mut format) = match mode {
            Mode::Exclusive => setup_exclusive(&device, direction, rate, block)?,
            Mode::LowLatency => setup_low_latency(&device, block)?,
        };
        // SAFETY: COM-Aufrufe auf dem eben eingerichteten Client dieses Threads; das Ereignis
        // gehört `Event` und lebt so lange wie der Client im Setup.
        unsafe {
            let event = Event(CreateEventW(None, false, false, None).map_err(|e| describe(&e))?);
            client.SetEventHandle(event.0).map_err(|e| describe(&e))?;
            let stream_latency = client.GetStreamLatency().unwrap_or(0);
            format.latency_frames = format
                .buffer_frames
                .saturating_add(hns_to_frames(stream_latency, format.rate));
            let (render, capture) = match direction {
                Direction::Render => (
                    Some(
                        client
                            .GetService::<IAudioRenderClient>()
                            .map_err(|e| describe(&e))?,
                    ),
                    None,
                ),
                Direction::Capture => (
                    None,
                    Some(
                        client
                            .GetService::<IAudioCaptureClient>()
                            .map_err(|e| describe(&e))?,
                    ),
                ),
            };
            Ok(Setup {
                mode,
                client,
                event,
                format,
                render,
                capture,
            })
        }
    }

    #[allow(clippy::too_many_arguments)]
    fn run(
        id: &[u16],
        direction: Direction,
        mode: Mode,
        (rate, block): (u32, u32),
        ready: mpsc::Sender<Result<StreamFormat, WasapiProblem>>,
        go: mpsc::Receiver<Job>,
        started: mpsc::Sender<Result<(), WasapiProblem>>,
        stop: &AtomicBool,
    ) {
        let setup = match setup(id, direction, mode, rate, block) {
            Ok(s) => s,
            Err(e) => {
                let _ = ready.send(Err(e));
                return;
            }
        };
        if ready.send(Ok(setup.format)).is_err() {
            return;
        }
        // Ohne Start-Auftrag (Prepared verworfen) endet der Thread hier.
        let Ok(job) = go.recv() else {
            return;
        };
        // SAFETY: Thread-Priorität für Audio (MMCSS); das Handle wird unten zurückgegeben.
        // Scheitert es, läuft der Stream mit normaler Priorität weiter.
        let mut task = 0u32;
        let mmcss = unsafe { AvSetMmThreadCharacteristicsW(w!("Pro Audio"), &mut task) }.ok();
        match job {
            Job::Render(mut renderer) => {
                if let Err(e) = start_render(&setup) {
                    let _ = started.send(Err(e));
                } else {
                    let _ = started.send(Ok(()));
                    render_loop(&setup, &mut renderer, stop);
                }
            }
            Job::Capture(mut sink) => {
                if let Err(e) = start_capture(&setup) {
                    let _ = started.send(Err(e));
                } else {
                    let _ = started.send(Ok(()));
                    capture_loop(&setup, &mut sink, stop);
                }
            }
        }
        // SAFETY: gültige Clients dieses Threads; das MMCSS-Handle stammt von oben.
        unsafe {
            let _ = setup.client.Stop();
            if let Some(h) = mmcss {
                let _ = AvRevertMmThreadCharacteristics(h);
            }
        }
    }

    /// Füllt den ganzen Puffer mit Stille und startet (vermeidet ein Knacken zu Beginn).
    fn start_render(setup: &Setup) -> Result<(), WasapiProblem> {
        let Some(render) = &setup.render else {
            return Err(WasapiProblem::NoRenderService);
        };
        let frames = setup.format.buffer_frames;
        // SAFETY: GetBuffer/ReleaseBuffer paarweise auf dem eingerichteten Render-Client;
        // mit dem Silent-Flag liest WASAPI den Puffer nicht.
        unsafe {
            render.GetBuffer(frames).map_err(|e| describe(&e))?;
            render
                .ReleaseBuffer(frames, AUDCLNT_BUFFERFLAGS_SILENT.0 as u32)
                .map_err(|e| describe(&e))?;
            setup.client.Start().map_err(|e| describe(&e))
        }
    }

    fn start_capture(setup: &Setup) -> Result<(), WasapiProblem> {
        // SAFETY: Start auf dem eingerichteten Client dieses Threads.
        unsafe { setup.client.Start() }.map_err(|e| describe(&e))
    }

    fn ns_of_frames(frames: u32, rate: u32) -> u64 {
        if rate == 0 {
            return 0;
        }
        u64::from(frames) * 1_000_000_000 / u64::from(rate)
    }

    /// Ob ein Gerätezeiger für `T` passend ausgerichtet ist.
    fn aligned<T>(p: *const u8) -> bool {
        !p.is_null() && (p as usize) % std::mem::align_of::<T>() == 0
    }

    /// Echtzeit-Schleife der Ausgabe: warten auf das Ereignis, Puffer holen, rendern.
    fn render_loop(setup: &Setup, renderer: &mut Renderer, stop: &AtomicBool) {
        let Some(render) = &setup.render else {
            return;
        };
        let f = setup.format;
        let channels = usize::from(f.channels);
        let latency_ns = ns_of_frames(f.latency_frames, f.rate);
        while !stop.load(Ordering::Acquire) {
            // SAFETY: gültiges Ereignis-Handle aus dem Setup.
            let woke = unsafe { WaitForSingleObject(setup.event.0, EVENT_TIMEOUT_MS) };
            if woke != WAIT_OBJECT_0 {
                // Kein Ereignis: prüfen, ob das Gerät noch da ist.
                // SAFETY: Abfrage auf dem eingerichteten Client dieses Threads.
                if unsafe { setup.client.GetCurrentPadding() }.is_err() {
                    renderer.mark_lost();
                    return;
                }
                continue;
            }
            // Exklusiv gehört der ganze Puffer der App; gemeinsam nur der freie Teil.
            let frames = match setup.mode {
                Mode::Exclusive => f.buffer_frames,
                Mode::LowLatency => {
                    // SAFETY: Abfrage auf dem eingerichteten Client dieses Threads.
                    match unsafe { setup.client.GetCurrentPadding() } {
                        Ok(padding) => f.buffer_frames.saturating_sub(padding),
                        Err(_) => {
                            renderer.mark_lost();
                            return;
                        }
                    }
                }
            };
            if frames == 0 {
                continue;
            }
            // SAFETY: GetBuffer liefert `frames * nBlockAlign` beschreibbare Bytes, die bis
            // ReleaseBuffer uns gehören. Für die Typumdeutung wird die Ausrichtung geprüft;
            // passt sie nicht, geht der Puffer als Stille zurück.
            unsafe {
                let Ok(p) = render.GetBuffer(frames) else {
                    renderer.mark_lost();
                    return;
                };
                let n = frames as usize * channels;
                let ok = match f.sample {
                    SampleKind::F32 if aligned::<f32>(p) => {
                        renderer.render(
                            std::slice::from_raw_parts_mut(p.cast::<f32>(), n),
                            latency_ns,
                        );
                        true
                    }
                    SampleKind::I32 | SampleKind::I24In32 if aligned::<i32>(p) => {
                        renderer.render(
                            std::slice::from_raw_parts_mut(p.cast::<i32>(), n),
                            latency_ns,
                        );
                        true
                    }
                    SampleKind::I16 if aligned::<i16>(p) => {
                        renderer.render(
                            std::slice::from_raw_parts_mut(p.cast::<i16>(), n),
                            latency_ns,
                        );
                        true
                    }
                    _ => false,
                };
                let flags = if ok {
                    0
                } else {
                    AUDCLNT_BUFFERFLAGS_SILENT.0 as u32
                };
                if render.ReleaseBuffer(frames, flags).is_err() {
                    renderer.mark_lost();
                    return;
                }
            }
        }
    }

    /// Echtzeit-Schleife des Eingangs: warten, alle anstehenden Pakete in die Senke.
    fn capture_loop(setup: &Setup, sink: &mut InputSink, stop: &AtomicBool) {
        let Some(capture) = &setup.capture else {
            return;
        };
        let f = setup.format;
        let channels = usize::from(f.channels);
        // Stille-Pakete und falsch ausgerichtete Puffer: Nullen in Paketgröße (vorab angelegt).
        let zeros = vec![0.0f32; f.buffer_frames.max(1) as usize * channels * 2];
        let stream_ns = ns_of_frames(f.latency_frames.saturating_sub(f.buffer_frames), f.rate);
        while !stop.load(Ordering::Acquire) {
            // SAFETY: gültiges Ereignis-Handle aus dem Setup.
            let woke = unsafe { WaitForSingleObject(setup.event.0, EVENT_TIMEOUT_MS) };
            if woke != WAIT_OBJECT_0 {
                // SAFETY: Abfrage auf dem eingerichteten Client dieses Threads.
                if unsafe { setup.client.GetCurrentPadding() }.is_err() {
                    eprintln!("Audio-Eingang: WASAPI-Gerät nicht mehr verfügbar");
                    return;
                }
                continue;
            }
            loop {
                // SAFETY: Paketabfragen auf dem eingerichteten Capture-Client; GetBuffer und
                // ReleaseBuffer paarweise, der Puffer wird nur dazwischen gelesen
                // (`frames * nBlockAlign` Bytes, Ausrichtung geprüft).
                unsafe {
                    match capture.GetNextPacketSize() {
                        Ok(0) => break,
                        Ok(_) => {}
                        Err(_) => return,
                    }
                    let mut data = std::ptr::null_mut();
                    let mut frames = 0u32;
                    let mut flags = 0u32;
                    if capture
                        .GetBuffer(&mut data, &mut frames, &mut flags, None, None)
                        .is_err()
                    {
                        return;
                    }
                    let n = frames as usize * channels;
                    let latency = ns_of_frames(frames, f.rate) + stream_ns;
                    let silent = flags & (AUDCLNT_BUFFERFLAGS_SILENT.0 as u32) != 0;
                    let p = data.cast_const();
                    match f.sample {
                        _ if silent => sink.push(&zeros[..n.min(zeros.len())], latency),
                        SampleKind::F32 if aligned::<f32>(p) => {
                            sink.push(std::slice::from_raw_parts(p.cast::<f32>(), n), latency);
                        }
                        SampleKind::I32 | SampleKind::I24In32 if aligned::<i32>(p) => {
                            sink.push(std::slice::from_raw_parts(p.cast::<i32>(), n), latency);
                        }
                        SampleKind::I16 if aligned::<i16>(p) => {
                            sink.push(std::slice::from_raw_parts(p.cast::<i16>(), n), latency);
                        }
                        _ => sink.push(&zeros[..n.min(zeros.len())], latency),
                    }
                    if capture.ReleaseBuffer(frames).is_err() {
                        return;
                    }
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn host_names_roundtrip() {
        for m in Mode::ALL {
            assert_eq!(Mode::from_host(m.host_name()), Some(m));
        }
        assert_eq!(Mode::from_host("wasapi exclusive"), Some(Mode::Exclusive));
        assert_eq!(Mode::from_host("WASAPI"), None);
    }

    #[test]
    fn frames_and_reference_time_convert() {
        // 480 frames at 48 kHz are 10 ms = 100 000 units of 100 ns.
        assert_eq!(frames_to_hns(480, 48_000), 100_000);
        assert_eq!(hns_to_frames(100_000, 48_000), 480);
        // 128 frames at 44.1 kHz round to the nearest unit and back.
        assert_eq!(hns_to_frames(frames_to_hns(128, 44_100), 44_100), 128);
        assert_eq!(frames_to_hns(1, 0), 0);
        assert_eq!(hns_to_frames(-5, 48_000), 0);
    }

    #[test]
    fn exclusive_period_never_drops_below_the_device_minimum() {
        // 64 frames at 48 kHz = 1.33 ms; the device allows 3 ms at least.
        assert_eq!(exclusive_period_hns(64, 48_000, 30_000), 30_000);
        // 256 frames = 5.33 ms is above the minimum and kept.
        assert_eq!(exclusive_period_hns(256, 48_000, 30_000), 53_333);
    }

    #[test]
    fn shared_period_rounds_to_the_fundamental_and_clamps() {
        // A typical driver: fundamental 32 frames, 128..=480 frames.
        assert_eq!(shared_period(100, 32, 128, 480), 128);
        assert_eq!(shared_period(256, 32, 128, 480), 256);
        assert_eq!(shared_period(250, 32, 128, 480), 256);
        assert_eq!(shared_period(4096, 32, 128, 480), 480);
        // Old drivers offer only the default period.
        assert_eq!(shared_period(64, 480, 480, 480), 480);
        // No fundamental reported: only clamp.
        assert_eq!(shared_period(200, 0, 128, 480), 200);
    }

    #[test]
    fn sample_kinds_are_recognised_from_their_bits() {
        for k in SampleKind::PREFERRED {
            let (container, valid, float) = k.bits();
            assert_eq!(SampleKind::from_bits(container, valid, float), Some(k));
        }
        assert_eq!(SampleKind::from_bits(16, 0, false), Some(SampleKind::I16));
        assert_eq!(SampleKind::from_bits(24, 24, false), None);
        assert_eq!(SampleKind::from_bits(64, 64, true), None);
    }
}
