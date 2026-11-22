//! CLAP über die C-Schnittstelle (`clap-sys`), Bibliotheken über `libloading`.

pub use crate::info::{MAX_LATENCY, ParamInfo};
use std::{
    collections::HashMap,
    ffi::{CStr, CString, c_char, c_void},
    path::{Path, PathBuf},
    ptr,
    sync::{Arc, Mutex, OnceLock, Weak},
};

use clap_sys::{
    audio_buffer::clap_audio_buffer,
    entry::clap_plugin_entry,
    events::{
        CLAP_CORE_EVENT_SPACE_ID, CLAP_EVENT_MIDI, CLAP_EVENT_NOTE_OFF, CLAP_EVENT_NOTE_ON,
        CLAP_EVENT_PARAM_VALUE, clap_event_header, clap_event_midi, clap_event_note,
        clap_event_param_value, clap_input_events, clap_output_events,
    },
    ext::{
        audio_ports::{
            CLAP_AUDIO_PORT_IS_MAIN, CLAP_EXT_AUDIO_PORTS, clap_audio_port_info,
            clap_plugin_audio_ports,
        },
        params::{
            CLAP_EXT_PARAMS, CLAP_PARAM_IS_HIDDEN, CLAP_PARAM_IS_READONLY, clap_param_info,
            clap_plugin_params,
        },
        state::{CLAP_EXT_STATE, clap_plugin_state},
    },
    factory::plugin_factory::{CLAP_PLUGIN_FACTORY_ID, clap_plugin_factory},
    host::clap_host,
    plugin::{clap_plugin, clap_plugin_descriptor},
    process::{CLAP_PROCESS_ERROR, clap_process},
    stream::{clap_istream, clap_ostream},
    version::{CLAP_VERSION, clap_version_is_compatible},
};

use crate::{
    host::HostShared,
    scan::{PluginInfo, cstr},
};

/// Höchstzahl Parameteränderungen pro Block (vorab angelegt, keine Allokation im Audio-Thread).
const MAX_EVENTS: usize = 256;

/// Prozessweite Sperre für Hauptthread-Aufrufe in Plugins (Bibliothek laden und freigeben,
/// Instanz erzeugen, aktivieren, Zustand laden, zerstören).
///
/// CLAP kennt genau einen Hauptthread, und viele Plugins verlassen sich darauf: Sie teilen
/// nicht thread-sichere Bibliotheken (etwa FFT-Planer) zwischen Instanzen. Arbeiten mehrere
/// Threads als „Hauptthread“ (parallele Tests, ein Scan im Hintergrund), stürzten solche
/// Plugins ab. Die Sperre ist wiedereintrittsfähig, weil ein solcher Aufruf beim Aufräumen
/// einen weiteren auslösen kann (gescheiterte Instanz, letzte Referenz auf die Bibliothek).
/// Im Programm ist sie nie umkämpft.
pub(crate) struct MainThreadGuard {
    /// Hält die Sperre (nur der äußerste Aufruf); wird beim Drop freigegeben.
    _lock: Option<std::sync::MutexGuard<'static, ()>>,
}

thread_local! {
    static MAIN_DEPTH: std::cell::Cell<u32> = const { std::cell::Cell::new(0) };
}

/// Nur in den Tests dieses Crates: `process` läuft ebenfalls unter [`main_thread_lock`].
/// Die Tests laufen parallel, und manche Plugins teilen Zustand zwischen Instanzen (ein
/// Faltungshall lud bei abweichender Abtastrate im Hintergrund nach und stürzte ab, während eine
/// andere Instanz rechnete). Im Programm rechnet je Engine genau ein Audio-Thread, dort darf
/// nichts sperren; außerhalb der Tests ist das hier ohne Wirkung.
#[cfg(test)]
pub(crate) fn test_process_guard() -> Option<MainThreadGuard> {
    Some(main_thread_lock())
}

#[cfg(not(test))]
#[inline(always)]
pub(crate) fn test_process_guard() -> Option<MainThreadGuard> {
    None
}

pub(crate) fn main_thread_lock() -> MainThreadGuard {
    static LOCK: Mutex<()> = Mutex::new(());
    let depth = MAIN_DEPTH.with(|d| {
        let n = d.get();
        d.set(n.saturating_add(1));
        n
    });
    if depth == 0 {
        // Eine Vergiftung (Panik unter der Sperre) ändert nichts am Schutz.
        MainThreadGuard {
            _lock: Some(
                LOCK.lock()
                    .unwrap_or_else(std::sync::PoisonError::into_inner),
            ),
        }
    } else {
        MainThreadGuard { _lock: None }
    }
}

impl Drop for MainThreadGuard {
    fn drop(&mut self) {
        // Erst die Tiefe senken, danach gibt das Feld die Sperre frei.
        MAIN_DEPTH.with(|d| d.set(d.get().saturating_sub(1)));
    }
}

/// Eine geladene CLAP-Datei. Wird geteilt, solange Instanzen daraus leben;
/// beim letzten Freigeben folgt `deinit`.
pub(crate) struct Library {
    entry: *const clap_plugin_entry,
    _lib: libloading::Library,
}

// SAFETY: der Einstiegspunkt ist laut CLAP thread-sicher; Bibliothek bleibt geladen.
unsafe impl Send for Library {}
unsafe impl Sync for Library {}

impl Drop for Library {
    fn drop(&mut self) {
        let _main = main_thread_lock();
        // SAFETY: `entry` stammt aus dieser Bibliothek, `init` war erfolgreich.
        unsafe {
            if let Some(deinit) = (*self.entry).deinit {
                deinit();
            }
        }
    }
}

fn library_cache() -> &'static Mutex<HashMap<PathBuf, Weak<Library>>> {
    static CACHE: OnceLock<Mutex<HashMap<PathBuf, Weak<Library>>>> = OnceLock::new();
    CACHE.get_or_init(Default::default)
}

/// Auf macOS ist ein `.clap` ein Bundle-Ordner; die Bibliothek liegt darin.
fn binary_path(bundle: &Path) -> PathBuf {
    if bundle.is_dir() {
        let stem = bundle.file_stem().unwrap_or_default();
        bundle.join("Contents/MacOS").join(stem)
    } else {
        bundle.to_path_buf()
    }
}

impl Library {
    /// Lädt (oder teilt) die Bibliothek und ruft `init` auf. Nur im Hauptthread.
    pub(crate) fn load(bundle: &Path) -> Result<Arc<Self>, crate::error::PluginError> {
        let _main = main_thread_lock();
        let mut cache = library_cache()
            .lock()
            .map_err(|_| crate::error::PluginError::Locked(crate::error::Lock::PluginCache))?;
        if let Some(lib) = cache.get(bundle).and_then(Weak::upgrade) {
            return Ok(lib);
        }
        // SAFETY: Laden fremden Codes ist der Zweck; Initialisierer der Bibliothek laufen hier.
        let lib = unsafe { libloading::Library::new(binary_path(bundle)) }
            .map_err(|e| crate::error::PluginError::Library(e.to_string()))?;
        // SAFETY: `clap_entry` ist laut CLAP ein Datensymbol vom Typ `clap_plugin_entry`.
        let entry: *const clap_plugin_entry = unsafe {
            let sym: libloading::Symbol<*const clap_plugin_entry> =
                lib.get(b"clap_entry\0").map_err(|_| {
                    crate::error::PluginError::BadFile(crate::error::BadFile::NoClapEntry)
                })?;
            *sym
        };
        if entry.is_null() {
            return Err(crate::error::PluginError::BadFile(
                crate::error::BadFile::EmptyClapEntry,
            ));
        }
        // SAFETY: `entry` zeigt auf eine gültige Struktur der geladenen Bibliothek.
        unsafe {
            if !clap_version_is_compatible((*entry).clap_version) {
                return Err(crate::error::PluginError::BadFile(
                    crate::error::BadFile::IncompatibleClap,
                ));
            }
            let path = CString::new(bundle.to_string_lossy().as_bytes())
                .map_err(|e| crate::error::PluginError::Library(e.to_string()))?;
            let init = (*entry).init.ok_or(crate::error::PluginError::BadFile(
                crate::error::BadFile::ClapInitMissing,
            ))?;
            if !init(path.as_ptr()) {
                return Err(crate::error::PluginError::BadFile(
                    crate::error::BadFile::ClapInitFailed,
                ));
            }
        }
        let lib = Arc::new(Self { entry, _lib: lib });
        cache.insert(bundle.to_path_buf(), Arc::downgrade(&lib));
        Ok(lib)
    }

    fn factory(&self) -> Option<&clap_plugin_factory> {
        // SAFETY: `get_factory` liefert eine Fabrik, die so lange lebt wie die Bibliothek.
        unsafe {
            let get = (*self.entry).get_factory?;
            let f = get(CLAP_PLUGIN_FACTORY_ID.as_ptr()) as *const clap_plugin_factory;
            f.as_ref()
        }
    }

    /// Preset-Fabrik der Bibliothek (aktuelle oder Entwurfs-Kennung), falls vorhanden.
    pub(crate) fn preset_factory(&self) -> Option<*const crate::clap_presets::Factory> {
        // SAFETY: `get_factory` liefert null oder eine Fabrik, die so lange lebt wie die Bibliothek.
        unsafe {
            let get = (*self.entry).get_factory?;
            crate::clap_presets::FACTORY_IDS
                .iter()
                .map(|id| get(id.as_ptr()) as *const crate::clap_presets::Factory)
                .find(|f| !f.is_null())
        }
    }

    pub(crate) fn descriptors(&self) -> Vec<PluginInfo> {
        // Hauptthread-Aufruf: Manche Plugins bauen ihre Beschreibungen erst hier und nicht
        // thread-sicher, während andere Threads Instanzen erzeugen.
        let _main = main_thread_lock();
        let Some(f) = self.factory() else {
            return Vec::new();
        };
        let (Some(count), Some(get)) = (f.get_plugin_count, f.get_plugin_descriptor) else {
            return Vec::new();
        };
        // SAFETY: Index im Bereich 0..count; Beschreibungen leben so lange wie die Fabrik.
        unsafe {
            (0..count(f))
                .filter_map(|i| get(f, i).as_ref())
                .map(|d| PluginInfo {
                    format: crate::scan::PluginFormat::Clap,
                    id: cstr(d.id),
                    name: cstr(d.name),
                    vendor: cstr(d.vendor),
                    version: cstr(d.version),
                    features: features(d),
                    path: PathBuf::new(),
                })
                .collect()
        }
    }
}

/// Liest die nullterminierte Feature-Liste einer Beschreibung.
///
/// # Safety
/// `d` muss eine gültige CLAP-Beschreibung sein.
unsafe fn features(d: &clap_plugin_descriptor) -> Vec<String> {
    let mut out = Vec::new();
    let mut p = d.features;
    if p.is_null() {
        return out;
    }
    // SAFETY: laut CLAP endet die Liste mit einem Null-Zeiger.
    unsafe {
        while !(*p).is_null() {
            out.push(cstr(*p));
            p = p.add(1);
        }
    }
    out
}

// ---------------------------------------------------------------------------
// Host-Seite: was das Plugin von uns aufrufen darf.

struct HostStrings {
    name: CString,
    vendor: CString,
    url: CString,
    version: CString,
}

fn host_strings() -> &'static HostStrings {
    static S: OnceLock<HostStrings> = OnceLock::new();
    S.get_or_init(|| HostStrings {
        name: CString::new("StudioCraft").unwrap_or_default(),
        vendor: CString::new("StudioCraft contributors").unwrap_or_default(),
        url: CString::new("").unwrap_or_default(),
        version: CString::new(env!("CARGO_PKG_VERSION")).unwrap_or_default(),
    })
}

fn new_host(shared: &Arc<HostShared>) -> Box<clap_host> {
    let s = host_strings();
    Box::new(clap_host {
        clap_version: CLAP_VERSION,
        host_data: Arc::as_ptr(shared) as *mut c_void,
        name: s.name.as_ptr(),
        vendor: s.vendor.as_ptr(),
        url: s.url.as_ptr(),
        version: s.version.as_ptr(),
        get_extension: Some(crate::host::get_extension),
        request_restart: Some(crate::host::host_request_nothing),
        request_process: Some(crate::host::host_request_nothing),
        request_callback: Some(crate::host::host_request_callback),
    })
}

// ---------------------------------------------------------------------------
// Ereignislisten für `process`.

/// Ein Ereignis beliebigen Typs; alle beginnen mit `clap_event_header`.
#[repr(C)]
#[derive(Clone, Copy)]
union AnyEvent {
    header: clap_event_header,
    param: clap_event_param_value,
    note: clap_event_note,
    midi: clap_event_midi,
}

impl AnyEvent {
    fn header(&self) -> &clap_event_header {
        // SAFETY: jede Variante beginnt mit dem Kopf (repr(C)).
        unsafe { &self.header }
    }
}

struct EventList {
    events: Vec<AnyEvent>,
}

unsafe extern "C" fn in_size(list: *const clap_input_events) -> u32 {
    // SAFETY: `ctx` zeigt während `process` auf unsere `EventList`.
    unsafe { (*((*list).ctx as *const EventList)).events.len() as u32 }
}
unsafe extern "C" fn in_get(list: *const clap_input_events, i: u32) -> *const clap_event_header {
    // SAFETY: wie oben; Index wird geprüft.
    unsafe {
        let l = &*((*list).ctx as *const EventList);
        l.events
            .get(i as usize)
            .map_or(ptr::null(), |e| e.header() as *const _)
    }
}
unsafe extern "C" fn out_push(_: *const clap_output_events, _: *const clap_event_header) -> bool {
    // Ausgabe-Ereignisse (z. B. Parameteränderungen aus der Plugin-GUI) später.
    true
}

// ---------------------------------------------------------------------------
// Zugriff aus dem Hauptthread, während die Instanz im Audio-Thread läuft.

/// Hauptthread-Zugriff auf eine laufende Instanz (Zustand, später Fenster).
///
/// CLAP erlaubt Hauptthread-Funktionen, während der Audio-Thread verarbeitet.
/// Vertrag: Der Griff muss freigegeben sein, bevor die Instanz zerstört wird
/// (StudioCraft gibt Griffe frei, bevor es eine Kette austauscht).
pub struct ClapHandle {
    pub(crate) plugin: *const clap_plugin,
    _lib: Arc<Library>,
    /// Id des Plugins (für die Zuordnung von Presets).
    plugin_id: String,
    pub(crate) shared: Arc<HostShared>,
    pub(crate) gui: crate::gui::GuiState,
}

impl Drop for ClapHandle {
    fn drop(&mut self) {
        // Fenster vor der Instanz schließen (Vertrag: Griff stirbt zuerst).
        self.close_gui();
    }
}

unsafe extern "C" fn ostream_write(
    stream: *const clap_ostream,
    buf: *const c_void,
    size: u64,
) -> i64 {
    // SAFETY: `ctx` zeigt während `save` auf unseren Vec; `buf` hat `size` Bytes.
    unsafe {
        let out = &mut *((*stream).ctx as *mut Vec<u8>);
        out.extend_from_slice(std::slice::from_raw_parts(buf as *const u8, size as usize));
    }
    size as i64
}

struct ReadCursor<'a> {
    data: &'a [u8],
    pos: usize,
}

unsafe extern "C" fn istream_read(stream: *const clap_istream, buf: *mut c_void, size: u64) -> i64 {
    // SAFETY: `ctx` zeigt während `load` auf unseren Cursor; `buf` fasst `size` Bytes.
    unsafe {
        let cur = &mut *((*stream).ctx as *mut ReadCursor);
        let n = (size as usize).min(cur.data.len() - cur.pos);
        ptr::copy_nonoverlapping(cur.data.as_ptr().add(cur.pos), buf as *mut u8, n);
        cur.pos += n;
        n as i64
    }
}

impl ClapHandle {
    /// Presets des Plugins aus der Preset-Fabrik seiner Datei (leer ohne Fabrik). Hauptthread.
    pub fn presets(&self) -> Vec<crate::info::Preset> {
        let _main = main_thread_lock();
        crate::clap_presets::discover(&self._lib, &self.plugin_id)
            .iter()
            .map(|f| f.preset(f.name.clone()))
            .collect()
    }

    /// Ein Preset aus [`Self::presets`] laden (Erweiterung `preset-load`). Hauptthread; laut
    /// CLAP parallel zur Verarbeitung erlaubt.
    pub fn load_preset(&self, uri: &str) -> Result<(), crate::error::PluginError> {
        use clap_sys::ext::preset_load::{
            CLAP_EXT_PRESET_LOAD, CLAP_EXT_PRESET_LOAD_COMPAT, clap_plugin_preset_load,
        };
        let (kind, location, key) = crate::clap_presets::parse_uri(uri).ok_or(
            crate::error::PluginError::Preset(crate::error::PresetProblem::NotClapPreset),
        )?;
        let _main = main_thread_lock();
        let mut ext: *const clap_plugin_preset_load = self.extension(CLAP_EXT_PRESET_LOAD);
        if ext.is_null() {
            ext = self.extension(CLAP_EXT_PRESET_LOAD_COMPAT);
        }
        // SAFETY: Erweiterung null oder gültig; Zeichenketten leben während des Aufrufs.
        let ok = unsafe {
            ext.as_ref().and_then(|e| e.from_location).is_some_and(|f| {
                f(
                    self.plugin,
                    kind,
                    location.as_ref().map_or(ptr::null(), |c| c.as_ptr()),
                    key.as_ref().map_or(ptr::null(), |c| c.as_ptr()),
                )
            })
        };
        if ok {
            Ok(())
        } else {
            Err(crate::error::PluginError::Preset(
                crate::error::PresetProblem::NotLoaded,
            ))
        }
    }

    /// Aktuelle Werte aller Parameter (Id, Wert), etwa nach dem Laden eines Presets. Hauptthread.
    pub fn param_values(&self) -> Vec<(u32, f64)> {
        let ext: *const clap_plugin_params = self.extension(CLAP_EXT_PARAMS);
        let mut out = Vec::new();
        // SAFETY: Erweiterung null oder gültig; Hauptthread; Strukturen in voller Größe.
        unsafe {
            let Some(e) = ext.as_ref() else { return out };
            let (Some(count), Some(info), Some(value)) = (e.count, e.get_info, e.get_value) else {
                return out;
            };
            for i in 0..count(self.plugin).min(100_000) {
                let mut pi: clap_param_info = std::mem::zeroed();
                if !info(self.plugin, i, &mut pi) {
                    continue;
                }
                let mut v = 0.0;
                if value(self.plugin, pi.id, &mut v) && v.is_finite() {
                    out.push((pi.id, v));
                }
            }
        }
        out
    }

    /// Erweiterung des Plugins oder null.
    pub(crate) fn extension<T>(&self, id: &CStr) -> *const T {
        // SAFETY: `plugin` lebt (Vertrag oben).
        unsafe {
            (*self.plugin)
                .get_extension
                .map_or(ptr::null(), |f| f(self.plugin, id.as_ptr()) as *const T)
        }
    }

    fn state_ext(&self) -> Option<&clap_plugin_state> {
        // SAFETY: `plugin` lebt (Vertrag oben); Ergebnis ist null oder gültig.
        unsafe {
            let f = (*self.plugin).get_extension?;
            (f(self.plugin, CLAP_EXT_STATE.as_ptr()) as *const clap_plugin_state).as_ref()
        }
    }

    /// Kompletter Plugin-Zustand (Preset, interne Daten). Nur im Hauptthread.
    pub fn save_state(&self) -> Option<Vec<u8>> {
        let save = self.state_ext()?.save?;
        let mut out = Vec::new();
        let stream = clap_ostream {
            ctx: &mut out as *mut Vec<u8> as *mut c_void,
            write: Some(ostream_write),
        };
        // SAFETY: Strom und Plugin gültig; Aufruf im Hauptthread.
        unsafe { save(self.plugin, &stream) }.then_some(out)
    }

    /// Latenz des Plugins in Frames (`clap.latency`; ohne Erweiterung 0). Nur im
    /// Hauptthread, solange die Instanz aktiviert ist.
    pub fn latency(&self) -> u32 {
        let ext = self.extension::<clap_sys::ext::latency::clap_plugin_latency>(
            clap_sys::ext::latency::CLAP_EXT_LATENCY,
        );
        // SAFETY: `ext` ist null oder gültig, solange das Plugin lebt (Vertrag oben); Aufruf
        // im Hauptthread.
        let get = unsafe { ext.as_ref() }.and_then(|e| e.get);
        // SAFETY: wie oben.
        get.map_or(0, |f| unsafe { f(self.plugin) })
            .min(MAX_LATENCY)
    }

    /// Hat das Plugin seit dem letzten Aufruf eine geänderte Latenz gemeldet?
    pub fn take_latency_changed(&self) -> bool {
        self.shared
            .latency_changed
            .swap(false, std::sync::atomic::Ordering::AcqRel)
    }

    /// Zustand laden. Nur im Hauptthread.
    pub fn load_state(&self, data: &[u8]) -> bool {
        let _main = main_thread_lock();
        let Some(load) = self.state_ext().and_then(|s| s.load) else {
            return false;
        };
        let mut cur = ReadCursor { data, pos: 0 };
        let stream = clap_istream {
            ctx: &mut cur as *mut ReadCursor as *mut c_void,
            read: Some(istream_read),
        };
        // SAFETY: Strom und Plugin gültig; Aufruf im Hauptthread.
        unsafe { load(self.plugin, &stream) }
    }
}

// ---------------------------------------------------------------------------
// Instanz.

/// Höchstens so viele Anschlüsse je Richtung und Kanäle je Anschluss (wie bei VST3): Die
/// Zahlen kommen vom Plugin, und für jeden Kanal wird ein Puffer angelegt.
const MAX_PORTS: u32 = 16;
const MAX_PORT_CHANNELS: u32 = 32;

/// Kanäle je Anschluss und der Hauptanschluss aus dem, was das Plugin meldet: `count`
/// Anschlüsse, `info(i)` mit (Kanäle, Flags) oder `None`, wenn das Plugin den Anschluss nicht
/// beschreibt (dann ohne Kanäle). Hauptanschluss ist der erste mit dem Main-Flag, sonst der
/// erste. Anzahl und Kanäle sind begrenzt.
fn port_layout(
    count: u32,
    mut info: impl FnMut(u32) -> Option<(u32, u32)>,
) -> (Vec<u32>, Option<usize>) {
    let mut channels = Vec::new();
    let mut main = None;
    for i in 0..count.min(MAX_PORTS) {
        let (ch, flags) = info(i).unwrap_or((0, 0));
        if main.is_none() && flags & CLAP_AUDIO_PORT_IS_MAIN != 0 {
            main = Some(channels.len());
        }
        channels.push(ch.min(MAX_PORT_CHANNELS));
    }
    let main = main.or((!channels.is_empty()).then_some(0));
    (channels, main)
}

/// Puffer für alle Audio-Anschlüsse einer Richtung. Laut CLAP muss der Host
/// für jeden gemeldeten Anschluss Puffer liefern (auch Sidechains). Alles wird
/// beim Erzeugen angelegt; die Zeiger bleiben gültig, weil die Daten auf dem
/// Heap liegen und nicht mehr wachsen.
struct PortSet {
    bufs: Vec<Vec<Vec<f32>>>,
    /// Kanalzeiger-Tabellen, auf die `clap[i].data32` zeigt; müssen so lange leben.
    _ptrs: Vec<Vec<*mut f32>>,
    clap: Vec<clap_audio_buffer>,
    /// Index des Hauptanschlusses.
    main: Option<usize>,
}

impl PortSet {
    fn new(channels: &[u32], main: Option<usize>, max_frames: usize) -> Self {
        let mut bufs: Vec<Vec<Vec<f32>>> = channels
            .iter()
            .map(|&c| (0..c).map(|_| vec![0.0; max_frames]).collect())
            .collect();
        let mut ptrs: Vec<Vec<*mut f32>> = bufs
            .iter_mut()
            .map(|port| port.iter_mut().map(|ch| ch.as_mut_ptr()).collect())
            .collect();
        let clap = ptrs
            .iter_mut()
            .map(|p| clap_audio_buffer {
                data32: p.as_mut_ptr(),
                data64: ptr::null_mut(),
                channel_count: p.len() as u32,
                latency: 0,
                constant_mask: 0,
            })
            .collect();
        Self {
            bufs,
            _ptrs: ptrs,
            clap,
            main,
        }
    }

    fn main_channels(&self) -> usize {
        self.main.and_then(|m| self.bufs.get(m)).map_or(0, Vec::len)
    }
}

/// Eine laufende Plugin-Instanz mit Stereo-Ein- und -Ausgang.
pub struct ClapInstance {
    plugin: *const clap_plugin,
    _host: Box<clap_host>,
    _lib: Arc<Library>,
    /// Daten für Host-Rückrufe; `_host.host_data` zeigt darauf.
    shared: Arc<HostShared>,
    pub info: PluginInfo,
    pub params: Vec<ParamInfo>,
    params_ext: *const clap_plugin_params,
    processing: bool,
    max_frames: usize,
    steady_time: i64,
    // Vorab angelegte Puffer für den Audio-Thread.
    inputs: PortSet,
    outputs: PortSet,
    pending: EventList,
}

// SAFETY: CLAP erlaubt, eine Instanz zwischen Threads weiterzugeben, solange
// die Thread-Regeln je Funktion eingehalten werden (siehe Crate-Doku).
unsafe impl Send for ClapInstance {}

impl ClapInstance {
    /// Erzeugt, initialisiert und aktiviert das Plugin. Nur im Hauptthread.
    pub fn new(
        info: &PluginInfo,
        sample_rate: u32,
        max_frames: u32,
    ) -> Result<Self, crate::error::PluginError> {
        let _main = main_thread_lock();
        let lib = Library::load(&info.path)?;
        let factory = lib.factory().ok_or(crate::error::PluginError::BadFile(
            crate::error::BadFile::NoFactory,
        ))?;
        let create = factory
            .create_plugin
            .ok_or(crate::error::PluginError::BadFile(
                crate::error::BadFile::NoCreate,
            ))?;
        let shared = Arc::new(HostShared::default());
        let host = new_host(&shared);
        let id = CString::new(info.id.as_bytes())
            .map_err(|e| crate::error::PluginError::Library(e.to_string()))?;
        // SAFETY: Fabrik, Host und Id sind gültig; der Host lebt so lange wie die Instanz.
        let plugin = unsafe { create(factory, &*host, id.as_ptr()) };
        if plugin.is_null() {
            return Err(crate::error::PluginError::Create {
                plugin: info.name.clone(),
                step: crate::error::Step::Create,
            });
        }
        let mut inst = Self {
            plugin,
            _host: host,
            _lib: lib,
            shared,
            info: info.clone(),
            params: Vec::new(),
            params_ext: ptr::null(),
            processing: false,
            max_frames: max_frames as usize,
            steady_time: 0,
            inputs: PortSet::new(&[], None, 0),
            outputs: PortSet::new(&[], None, 0),
            pending: EventList {
                events: Vec::with_capacity(MAX_EVENTS),
            },
        };
        // SAFETY: `plugin` ist gültig; Aufrufe im Hauptthread in der CLAP-Reihenfolge.
        unsafe {
            let p = &*plugin;
            if !p.init.is_some_and(|f| f(plugin)) {
                // Ohne erfolgreiches init nur destroy (Drop würde deactivate versuchen).
                if let Some(d) = p.destroy {
                    d(plugin);
                }
                inst.plugin = ptr::null();
                return Err(crate::error::PluginError::Create {
                    plugin: info.name.clone(),
                    step: crate::error::Step::Init,
                });
            }
            inst.read_ports();
            inst.read_params();
            if !p
                .activate
                .is_some_and(|f| f(plugin, f64::from(sample_rate), 1, max_frames))
            {
                return Err(crate::error::PluginError::Create {
                    plugin: info.name.clone(),
                    step: crate::error::Step::Activate,
                });
            }
        }
        if inst.outputs.main_channels() == 0 {
            return Err(crate::error::PluginError::Create {
                plugin: info.name.clone(),
                step: crate::error::Step::NoAudioOutput,
            });
        }
        Ok(inst)
    }

    /// Griff für Hauptthread-Aufrufe (siehe [`ClapHandle`]).
    pub fn handle(&self) -> ClapHandle {
        ClapHandle {
            plugin: self.plugin,
            _lib: self._lib.clone(),
            plugin_id: self.info.id.clone(),
            shared: self.shared.clone(),
            gui: Default::default(),
        }
    }

    fn extension<T>(&self, id: &CStr) -> *const T {
        // SAFETY: `plugin` ist gültig; das Ergebnis ist null oder eine Struktur vom Typ T.
        unsafe {
            (*self.plugin)
                .get_extension
                .map_or(ptr::null(), |f| f(self.plugin, id.as_ptr()) as *const T)
        }
    }

    /// Alle Audio-Anschlüsse lesen und Puffer dafür anlegen
    /// (ohne Erweiterung: ein Stereo-Ein- und -Ausgang).
    fn read_ports(&mut self) {
        let ports: *const clap_plugin_audio_ports = self.extension(CLAP_EXT_AUDIO_PORTS);
        // SAFETY: Erweiterung gültig, Aufruf im Hauptthread.
        let list = |is_input: bool| unsafe {
            let Some(ports) = ports.as_ref() else {
                return (vec![2], Some(0));
            };
            let (Some(count), Some(get)) = (ports.count, ports.get) else {
                return (vec![2], Some(0));
            };
            port_layout(count(self.plugin, is_input), |i| {
                let mut info: clap_audio_port_info = std::mem::zeroed();
                get(self.plugin, i, is_input, &mut info).then_some((info.channel_count, info.flags))
            })
        };
        let (ic, im) = list(true);
        let (oc, om) = list(false);
        self.inputs = PortSet::new(&ic, im, self.max_frames);
        self.outputs = PortSet::new(&oc, om, self.max_frames);
    }

    fn read_params(&mut self) {
        self.params_ext = self.extension(CLAP_EXT_PARAMS);
        // SAFETY: Erweiterung gültig, Aufruf im Hauptthread.
        unsafe {
            let Some(ext) = self.params_ext.as_ref() else {
                return;
            };
            let (Some(count), Some(get_info)) = (ext.count, ext.get_info) else {
                return;
            };
            for i in 0..count(self.plugin) {
                let mut info: clap_param_info = std::mem::zeroed();
                if !get_info(self.plugin, i, &mut info) {
                    continue;
                }
                if info.flags & (CLAP_PARAM_IS_HIDDEN | CLAP_PARAM_IS_READONLY) != 0 {
                    continue;
                }
                let mut value = info.default_value;
                if let Some(get_value) = ext.get_value {
                    get_value(self.plugin, info.id, &mut value);
                }
                self.params.push(ParamInfo {
                    id: info.id,
                    name: cstr(info.name.as_ptr()),
                    module: cstr(info.module.as_ptr()),
                    min: info.min_value,
                    max: info.max_value,
                    default: info.default_value,
                    value,
                    stepped: info.flags & clap_sys::ext::params::CLAP_PARAM_IS_STEPPED != 0,
                });
            }
        }
    }

    /// Text zu einem Parameterwert, wie das Plugin ihn anzeigt. Nur im Hauptthread.
    pub fn value_text(&self, id: u32, value: f64) -> Option<String> {
        // SAFETY: Erweiterung gültig; Puffer groß genug und nullterminiert.
        unsafe {
            let ext = self.params_ext.as_ref()?;
            let f = ext.value_to_text?;
            let mut buf = [0 as c_char; 64];
            f(self.plugin, id, value, buf.as_mut_ptr(), buf.len() as u32)
                .then(|| cstr(buf.as_ptr()))
        }
    }

    /// Parameteränderung für den nächsten Block vormerken. Echtzeitsicher;
    /// ist der Vorrat voll, gewinnt beim nächsten Block der neueste Wert nicht.
    pub fn set_param(&mut self, id: u32, value: f64) {
        let existing = self.pending.events.iter_mut().find(|e| {
            // SAFETY: Typ wird über den Kopf geprüft, bevor die Variante gelesen wird.
            e.header().type_ == CLAP_EVENT_PARAM_VALUE && unsafe { e.param.param_id } == id
        });
        if let Some(e) = existing {
            // SAFETY: oben als Parameter-Ereignis erkannt.
            e.param.value = value;
            return;
        }
        if self.pending.events.len() < MAX_EVENTS {
            self.pending.events.push(AnyEvent {
                param: clap_event_param_value {
                    header: clap_event_header {
                        size: std::mem::size_of::<clap_event_param_value>() as u32,
                        time: 0,
                        space_id: CLAP_CORE_EVENT_SPACE_ID,
                        type_: CLAP_EVENT_PARAM_VALUE,
                        flags: 0,
                    },
                    param_id: id,
                    cookie: ptr::null_mut(),
                    note_id: -1,
                    port_index: -1,
                    channel: -1,
                    key: -1,
                    value,
                },
            });
        }
    }

    /// Parameteränderung an Frame `time` des nächsten Blocks (Automation). Anders als
    /// [`Self::set_param`] bleiben frühere Werte stehen, damit eine Kurve im Block mehrere
    /// Stützpunkte hat. Echtzeitsicher; ist der Vorrat voll, fällt das Ereignis weg.
    pub fn set_param_at(&mut self, time: u32, id: u32, value: f64) {
        if self.pending.events.len() >= MAX_EVENTS || !value.is_finite() {
            return;
        }
        self.pending.events.push(AnyEvent {
            param: clap_event_param_value {
                header: clap_event_header {
                    size: std::mem::size_of::<clap_event_param_value>() as u32,
                    time,
                    space_id: CLAP_CORE_EVENT_SPACE_ID,
                    type_: CLAP_EVENT_PARAM_VALUE,
                    flags: 0,
                },
                param_id: id,
                cookie: ptr::null_mut(),
                note_id: -1,
                port_index: -1,
                channel: -1,
                key: -1,
                value,
            },
        });
    }

    /// Rohe MIDI-Nachricht (Controller, Pitchbend, Aftertouch). Echtzeitsicher.
    pub fn midi(&mut self, time: u32, data: [u8; 3]) {
        if self.pending.events.len() >= MAX_EVENTS {
            return;
        }
        self.pending.events.push(AnyEvent {
            midi: clap_event_midi {
                header: clap_event_header {
                    size: std::mem::size_of::<clap_event_midi>() as u32,
                    time,
                    space_id: CLAP_CORE_EVENT_SPACE_ID,
                    type_: CLAP_EVENT_MIDI,
                    flags: 0,
                },
                port_index: 0,
                data,
            },
        });
    }

    /// Note an oder aus, `time` = Frame im nächsten Block. Echtzeitsicher.
    pub fn note(&mut self, time: u32, key: u8, velocity: f32, on: bool) {
        if self.pending.events.len() >= MAX_EVENTS {
            return;
        }
        self.pending.events.push(AnyEvent {
            note: clap_event_note {
                header: clap_event_header {
                    size: std::mem::size_of::<clap_event_note>() as u32,
                    time,
                    space_id: CLAP_CORE_EVENT_SPACE_ID,
                    type_: if on {
                        CLAP_EVENT_NOTE_ON
                    } else {
                        CLAP_EVENT_NOTE_OFF
                    },
                    flags: 0,
                },
                note_id: -1,
                port_index: 0,
                channel: 0,
                key: i16::from(key),
                velocity: f64::from(velocity),
            },
        });
    }

    /// Bearbeitet Stereo an Ort und Stelle. Nur im Audio-Thread, echtzeitsicher.
    pub fn process(&mut self, left: &mut [f32], right: &mut [f32]) {
        let _tests = crate::clap::test_process_guard();
        let n = left.len().min(right.len());
        for (l, r) in left[..n]
            .chunks_mut(self.max_frames)
            .zip(right[..n].chunks_mut(self.max_frames))
        {
            self.process_block(l, r);
        }
    }

    fn process_block(&mut self, left: &mut [f32], right: &mut [f32]) {
        let n = left.len();
        // SAFETY: `plugin` ist aktiviert; Aufrufe im Audio-Thread.
        unsafe {
            let p = &*self.plugin;
            if !self.processing {
                self.processing = p.start_processing.is_none_or(|f| f(self.plugin));
                if !self.processing {
                    return;
                }
            }
            // Haupteingang füllen (Mono-Plugins bekommen die Summe); andere Eingänge bleiben still.
            if let Some(main) = self.inputs.main.and_then(|m| self.inputs.bufs.get_mut(m)) {
                match main.len() {
                    0 => {}
                    1 => {
                        for i in 0..n {
                            main[0][i] = (left[i] + right[i]) * 0.5;
                        }
                    }
                    _ => {
                        main[0][..n].copy_from_slice(left);
                        main[1][..n].copy_from_slice(right);
                    }
                }
            }
            let in_events = clap_input_events {
                ctx: &mut self.pending as *mut EventList as *mut c_void,
                size: Some(in_size),
                get: Some(in_get),
            };
            let out_events = clap_output_events {
                ctx: ptr::null_mut(),
                try_push: Some(out_push),
            };
            let process = clap_process {
                steady_time: self.steady_time,
                frames_count: n as u32,
                transport: ptr::null(),
                audio_inputs: self.inputs.clap.as_ptr(),
                audio_outputs: self.outputs.clap.as_mut_ptr(),
                audio_inputs_count: self.inputs.clap.len() as u32,
                audio_outputs_count: self.outputs.clap.len() as u32,
                in_events: &in_events,
                out_events: &out_events,
            };
            // CLAP verlangt zeitlich sortierte Ereignisse (sortiert an Ort und Stelle,
            // ohne Allokation). Bei gleicher Zeit kommt Note-Off vor Note-On, damit
            // eine direkt anschließende gleiche Note nicht abgeschnitten wird.
            self.pending.events.sort_unstable_by_key(|e| {
                let h = e.header();
                let order = match h.type_ {
                    CLAP_EVENT_NOTE_OFF => 0u8,
                    CLAP_EVENT_NOTE_ON => 1,
                    _ => 2,
                };
                (h.time, order)
            });
            let status = p
                .process
                .map_or(CLAP_PROCESS_ERROR, |f| f(self.plugin, &process));
            self.pending.events.clear();
            self.steady_time += n as i64;
            if status == CLAP_PROCESS_ERROR {
                return; // Signal unverändert durchlassen.
            }
        }
        let Some(out) = self.outputs.main.and_then(|m| self.outputs.bufs.get(m)) else {
            return;
        };
        if let Some(l) = out.first() {
            left.copy_from_slice(&l[..n]);
            right.copy_from_slice(&out.get(1).unwrap_or(l)[..n]);
        }
    }

    /// Verarbeitung beenden. Im Audio-Thread aufrufen, bevor die Instanz zurück
    /// an den Hauptthread geht.
    pub fn stop_processing(&mut self) {
        if self.processing {
            // SAFETY: `plugin` gültig und in Verarbeitung.
            unsafe {
                if let Some(f) = (*self.plugin).stop_processing {
                    f(self.plugin);
                }
            }
            self.processing = false;
        }
    }
}

impl Drop for ClapInstance {
    /// Deaktivieren und zerstören. Gehört in den Hauptthread.
    fn drop(&mut self) {
        if self.plugin.is_null() {
            return;
        }
        let _main = main_thread_lock();
        self.stop_processing();
        // SAFETY: `plugin` gültig und aktiviert; danach wird es nicht mehr benutzt.
        unsafe {
            let p = &*self.plugin;
            if let Some(f) = p.deactivate {
                f(self.plugin);
            }
            if let Some(f) = p.destroy {
                f(self.plugin);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const MAIN: u32 = CLAP_AUDIO_PORT_IS_MAIN;

    /// Ein Effekt mit Sidechain: Jeder Anschluss bekommt eigene Puffer mit seiner Kanalzahl
    /// (CLAP verlangt Puffer auch für Sidechains), der Haupteingang ist der mit dem Flag.
    #[test]
    fn every_port_gets_buffers_including_side_chains() {
        let ports = [(1, 0), (2, MAIN), (2, 0)]; // Mono-Sidechain vorn, Main, Stereo-Sidechain
        let (channels, main) = port_layout(3, |i| ports.get(i as usize).copied());
        assert_eq!((channels.as_slice(), main), (&[1, 2, 2][..], Some(1)));
        let set = PortSet::new(&channels, main, 512);
        assert_eq!(set.clap.len(), 3);
        assert_eq!(set.main_channels(), 2);
        let mut seen = std::collections::HashSet::new();
        for (k, buf) in set.clap.iter().enumerate() {
            assert_eq!(buf.channel_count, channels[k]);
            assert!(buf.data64.is_null());
            assert_eq!(set.bufs[k].len(), buf.channel_count as usize);
            for (c, data) in set.bufs[k].iter().enumerate() {
                // SAFETY: `data32` zeigt auf `channel_count` Kanalzeiger (PortSet::new).
                let ch = unsafe { *buf.data32.add(c) };
                assert!(!ch.is_null() && seen.insert(ch), "eigener Puffer je Kanal");
                assert_eq!(ch.cast_const(), data.as_ptr());
                assert_eq!(data.len(), 512);
            }
        }
    }

    #[test]
    fn the_main_port_falls_back_to_the_first() {
        assert_eq!(port_layout(2, |_| Some((2, 0))), (vec![2, 2], Some(0)));
        assert_eq!(port_layout(0, |_| Some((2, MAIN))), (vec![], None));
        // Ein Anschluss, den das Plugin nicht beschreibt, hat keine Kanäle.
        assert_eq!(
            port_layout(2, |i| (i == 1).then_some((2, MAIN))),
            (vec![0, 2], Some(1))
        );
    }

    /// Unsinnige Zahlen eines Plugins legen keine riesigen Puffer an (sonst Absturz beim
    /// Anlegen): höchstens 16 Anschlüsse mit je 32 Kanälen, wie bei VST3.
    #[test]
    fn port_and_channel_counts_from_the_plugin_are_capped() {
        let mut asked = 0;
        let (channels, main) = port_layout(u32::MAX, |_| {
            asked += 1;
            Some((u32::MAX, MAIN))
        });
        assert_eq!(asked, MAX_PORTS);
        assert_eq!(channels, vec![MAX_PORT_CHANNELS; MAX_PORTS as usize]);
        assert_eq!(main, Some(0));
        let set = PortSet::new(&channels, main, 64);
        assert_eq!(set.bufs.iter().map(Vec::len).sum::<usize>(), 16 * 32);
    }
    use crate::scan::{default_paths, scan};

    /// Testet gegen echte, auf dem System installierte Plugins (falls vorhanden).
    fn installed() -> Vec<PluginInfo> {
        // Nur CLAP: dieselben Plugins gibt es oft auch als LV2 mit gleichem Namen.
        scan(&default_paths())
            .0
            .into_iter()
            .filter(|p| p.format == crate::scan::PluginFormat::Clap)
            .collect()
    }

    #[test]
    fn surge_presets_are_found_and_change_the_sound() {
        let Some(info) = installed().into_iter().find(|p| p.name == "Surge XT") else {
            eprintln!("Surge XT (CLAP) nicht installiert – übersprungen");
            return;
        };
        let inst = ClapInstance::new(&info, 48_000, 256);
        assert!(inst.is_ok());
        let Ok(inst) = inst else { return };
        let h = inst.handle();
        let presets = h.presets();
        eprintln!(
            "Surge XT: {} Presets, z. B. {:?}",
            presets.len(),
            presets.get(10).map(|p| &p.name)
        );
        assert!(presets.len() > 50, "Surge ships many factory patches");
        let before = h.param_values();
        assert!(!before.is_empty());
        // Ein Preset aus der Mitte (nicht das Init-Patch) laden.
        let p = &presets[presets.len() / 2];
        assert!(h.load_preset(&p.uri).is_ok(), "{}", p.name);
        let after = h.param_values();
        assert_ne!(before, after, "loading {} changes parameters", p.name);
        assert!(h.load_preset("urn:kein-clap").is_err());
    }

    #[test]
    fn state_roundtrip_with_installed_plugin() {
        let Some(info) = installed().into_iter().find(|p| p.name == "ZamComp") else {
            return;
        };
        let mut a = ClapInstance::new(&info, 48_000, 256).unwrap();
        let p = a.params[0].clone();
        let target = (p.min + p.max) / 2.0 + 0.123;
        a.set_param(p.id, target);
        let (mut l, mut r) = (vec![0.0; 64], vec![0.0; 64]);
        a.process(&mut l, &mut r);
        a.stop_processing();
        let state = a
            .handle()
            .save_state()
            .expect("ZamComp kann seinen Zustand speichern");
        let b = ClapInstance::new(&info, 48_000, 256).unwrap();
        assert!(b.handle().load_state(&state));
        let again = b.handle().save_state().unwrap();
        assert_eq!(again, state, "geladener Zustand ergibt denselben Zustand");
    }

    /// Mehrere Threads als „Hauptthread“ (wie parallele Tests): Plugins mit gemeinsamem,
    /// nicht thread-sicherem FFT-Planer (Faltung) stürzten beim gleichzeitigen Erzeugen ab.
    #[test]
    fn plugins_can_be_created_from_several_threads_at_once() {
        let plugins: Vec<PluginInfo> = installed()
            .into_iter()
            .filter(|p| ["ZamVerb", "ZamHeadX2"].contains(&p.name.as_str()))
            .collect();
        if plugins.is_empty() {
            eprintln!("keine Faltungs-Plugins installiert – übersprungen");
            return;
        }
        let threads: Vec<_> = (0..8)
            .map(|t| {
                let plugins = plugins.clone();
                std::thread::spawn(move || {
                    for k in 0..6 {
                        let info = &plugins[(t + k) % plugins.len()];
                        let inst = ClapInstance::new(info, 48_000, 256);
                        assert!(inst.is_ok(), "{}", info.name);
                    }
                })
            })
            .collect();
        for t in threads {
            assert!(t.join().is_ok(), "no thread panicked");
        }
    }

    #[test]
    fn gui_exists_but_only_opens_on_main_thread() {
        let Some(info) = installed().into_iter().find(|p| p.name == "ZamComp") else {
            return;
        };
        let inst = ClapInstance::new(&info, 48_000, 256).unwrap();
        let mut h = inst.handle();
        assert!(h.has_gui());
        // Testthreads heißen nicht „main“: CLAP verlangt den Hauptthread.
        assert!(
            h.open_gui("x")
                .unwrap_err()
                .to_string()
                .contains("Hauptthread")
        );
        assert!(!h.idle(), "ohne Fenster nichts zu bedienen");
    }

    #[test]
    fn scan_and_process_installed_effects() {
        let plugins = installed();
        if plugins.is_empty() {
            eprintln!("keine CLAP-Plugins installiert – übersprungen");
            return;
        }
        eprintln!("{} CLAP-Plugins gefunden", plugins.len());
        // Ein paar Effekte laden und ein Signal durchschicken.
        let effects: Vec<&PluginInfo> = plugins
            .iter()
            .filter(|p| p.features.iter().any(|f| f == "audio-effect"))
            .take(5)
            .collect();
        for info in effects {
            let mut inst = match ClapInstance::new(info, 48_000, 512) {
                Ok(i) => i,
                Err(e) => {
                    eprintln!("{e}");
                    continue;
                }
            };
            let mut l = vec![0.0f32; 1024];
            let mut r = vec![0.0f32; 1024];
            l[0] = 1.0;
            r[0] = 1.0;
            if let Some(p) = inst.params.first().cloned() {
                inst.set_param(p.id, p.default);
            }
            inst.process(&mut l, &mut r);
            assert!(
                l.iter().chain(&r).all(|s| s.is_finite()),
                "{}: Ausgabe nicht endlich",
                info.name
            );
            inst.stop_processing();
            eprintln!(
                "ok: {} ({} Parameter, {}→{} Kanäle)",
                info.name,
                inst.params.len(),
                inst.inputs.main_channels(),
                inst.outputs.main_channels()
            );
        }
    }
}
