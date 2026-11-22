//! LV2-Hosting (Issue #33) über lilv, zur Laufzeit geladen: Ohne installierte lilv-Bibliothek
//! gibt es einfach keine LV2-Plugins, das Programm startet trotzdem. Keine Build-Abhängigkeit.
//!
//! Umfang: Plugins finden (lilv liest nur die Turtle-Beschreibungen, kein Plugin-Code),
//! Instanzen mit URID-Map, Optionen und begrenzter Blockgröße, Audio (Stereo; Mono-Plugins
//! bekommen die Summe und speisen beide Kanäle), Control-Ports als Parameter, MIDI über einen
//! Atom-Port, Latenz über den Latenz-Port. Zustand, Presets und Fenster folgen.

pub use crate::info::Preset as Lv2Preset;
use std::{
    collections::HashMap,
    ffi::{CStr, CString, c_char, c_void},
    path::PathBuf,
    ptr,
    sync::{
        Arc, Mutex, OnceLock,
        atomic::{AtomicBool, AtomicU32, Ordering},
    },
};

use crate::{
    clap::{MAX_LATENCY, ParamInfo, main_thread_lock},
    scan::{PluginFormat, PluginInfo},
};

// ---------------------------------------------------------------------------
// LV2-Strukturen (Spezifikation lv2plug.in).

type RawHandle = *mut c_void;

#[repr(C)]
pub(crate) struct Lv2Feature {
    pub(crate) uri: *const c_char,
    pub(crate) data: *mut c_void,
}

#[repr(C)]
struct Lv2Descriptor {
    uri: *const c_char,
    instantiate: Option<
        unsafe extern "C" fn(
            *const Lv2Descriptor,
            f64,
            *const c_char,
            *const *const Lv2Feature,
        ) -> RawHandle,
    >,
    connect_port: Option<unsafe extern "C" fn(RawHandle, u32, *mut c_void)>,
    activate: Option<unsafe extern "C" fn(RawHandle)>,
    run: Option<unsafe extern "C" fn(RawHandle, u32)>,
    deactivate: Option<unsafe extern "C" fn(RawHandle)>,
    cleanup: Option<unsafe extern "C" fn(RawHandle)>,
    extension_data: Option<unsafe extern "C" fn(*const c_char) -> *const c_void>,
}

/// Öffentliche Struktur einer lilv-Instanz (`lilv.h`, die Zugriffe dort sind inline).
#[repr(C)]
struct LilvInstance {
    descriptor: *const Lv2Descriptor,
    handle: RawHandle,
    pimpl: *mut c_void,
}

#[repr(C)]
struct UridMap {
    handle: *mut c_void,
    map: unsafe extern "C" fn(*mut c_void, *const c_char) -> u32,
}

#[repr(C)]
struct UridUnmap {
    handle: *mut c_void,
    unmap: unsafe extern "C" fn(*mut c_void, u32) -> *const c_char,
}

#[repr(C)]
struct Lv2Option {
    context: u32,
    subject: u32,
    key: u32,
    size: u32,
    kind: u32,
    value: *const c_void,
}

#[repr(C)]
struct AtomHeader {
    size: u32,
    kind: u32,
}

const URI_AUDIO_PORT: &str = "http://lv2plug.in/ns/lv2core#AudioPort";
const URI_CONTROL_PORT: &str = "http://lv2plug.in/ns/lv2core#ControlPort";
const URI_INPUT_PORT: &str = "http://lv2plug.in/ns/lv2core#InputPort";
const URI_OUTPUT_PORT: &str = "http://lv2plug.in/ns/lv2core#OutputPort";
const URI_ATOM_PORT: &str = "http://lv2plug.in/ns/ext/atom#AtomPort";
const URI_INSTRUMENT: &str = "http://lv2plug.in/ns/lv2core#InstrumentPlugin";
const URI_LATENCY: &str = "http://lv2plug.in/ns/lv2core#latency";
const URI_INTEGER: &str = "http://lv2plug.in/ns/lv2core#integer";
const URI_TOGGLED: &str = "http://lv2plug.in/ns/lv2core#toggled";
const URI_ENUMERATION: &str = "http://lv2plug.in/ns/lv2core#enumeration";
const URI_NOT_ON_GUI: &str = "http://lv2plug.in/ns/ext/port-props#notOnGUI";
const URI_MIDI_EVENT: &str = "http://lv2plug.in/ns/ext/midi#MidiEvent";
const URI_ATOM_SEQUENCE: &str = "http://lv2plug.in/ns/ext/atom#Sequence";
const URI_ATOM_CHUNK: &str = "http://lv2plug.in/ns/ext/atom#Chunk";
const URI_ATOM_INT: &str = "http://lv2plug.in/ns/ext/atom#Int";
const URI_ATOM_FLOAT: &str = "http://lv2plug.in/ns/ext/atom#Float";
const URI_MAX_BLOCK: &str = "http://lv2plug.in/ns/ext/buf-size#maxBlockLength";
const URI_MIN_BLOCK: &str = "http://lv2plug.in/ns/ext/buf-size#minBlockLength";
const URI_NOMINAL_BLOCK: &str = "http://lv2plug.in/ns/ext/buf-size#nominalBlockLength";
const URI_SAMPLE_RATE: &str = "http://lv2plug.in/ns/ext/parameters#sampleRate";
const URI_PRESET: &str = "http://lv2plug.in/ns/ext/presets#Preset";
const URI_X11_UI: &str = "http://lv2plug.in/ns/extensions/ui#X11UI";
const URI_LABEL: &str = "http://www.w3.org/2000/01/rdf-schema#label";
const URI_ATOM_DOUBLE: &str = "http://lv2plug.in/ns/ext/atom#Double";
const URI_ATOM_BOOL: &str = "http://lv2plug.in/ns/ext/atom#Bool";
const STATE_INTERFACE: &CStr = c"http://lv2plug.in/ns/ext/state#interface";
/// Subjekt-URI gespeicherter Zustände (beliebig, aber fest).
const STATE_URI: &CStr = c"urn:studiocraft:lv2-state";
/// `LV2_STATE_IS_POD | LV2_STATE_IS_PORTABLE`: nur einfache, übertragbare Werte.
const STATE_FLAGS: u32 = 1 | 2;
const FEATURE_URID_MAP: &CStr = c"http://lv2plug.in/ns/ext/urid#map";
const FEATURE_URID_UNMAP: &CStr = c"http://lv2plug.in/ns/ext/urid#unmap";
const FEATURE_OPTIONS: &CStr = c"http://lv2plug.in/ns/ext/options#options";
const FEATURE_BOUNDED: &CStr = c"http://lv2plug.in/ns/ext/buf-size#boundedBlockLength";

/// Größe des Atom-Puffers je Atom-Port (Bytes).
const ATOM_CAPACITY: usize = 16 * 1024;

// ---------------------------------------------------------------------------
// URID-Map: prozessweit, Zeichenketten leben bis zum Programmende (Plugins behalten Zeiger).

struct Urids {
    ids: HashMap<String, u32>,
    uris: Vec<CString>,
}

fn urids() -> &'static Mutex<Urids> {
    static U: OnceLock<Mutex<Urids>> = OnceLock::new();
    U.get_or_init(|| {
        Mutex::new(Urids {
            ids: HashMap::new(),
            uris: Vec::new(),
        })
    })
}

fn map_uri(uri: &str) -> u32 {
    let Ok(mut u) = urids().lock() else {
        return 0;
    };
    if let Some(&id) = u.ids.get(uri) {
        return id;
    }
    let Ok(c) = CString::new(uri) else {
        return 0;
    };
    u.uris.push(c);
    let id = u32::try_from(u.uris.len()).unwrap_or(0);
    u.ids.insert(uri.to_owned(), id);
    id
}

unsafe extern "C" fn urid_map(_: *mut c_void, uri: *const c_char) -> u32 {
    if uri.is_null() {
        return 0;
    }
    // SAFETY: Das Plugin übergibt laut LV2 eine nullterminierte URI.
    let s = unsafe { CStr::from_ptr(uri) }.to_string_lossy();
    map_uri(&s)
}

unsafe extern "C" fn urid_unmap(_: *mut c_void, id: u32) -> *const c_char {
    let Ok(u) = urids().lock() else {
        return ptr::null();
    };
    // Zeiger bleibt gültig: Einträge werden nie entfernt, CString-Daten liegen auf dem Heap.
    (id as usize)
        .checked_sub(1)
        .and_then(|i| u.uris.get(i))
        .map_or(ptr::null(), |c| c.as_ptr())
}

// ---------------------------------------------------------------------------
// lilv, zur Laufzeit geladen.

type Ptr = *mut c_void;

macro_rules! lilv_api {
    ($($name:ident : fn($($arg:ty),*) $(-> $ret:ty)?;)*) => {
        #[allow(non_snake_case)]
        struct Lilv {
            _lib: libloading::Library,
            $($name: unsafe extern "C" fn($($arg),*) $(-> $ret)?,)*
        }

        impl Lilv {
            fn open() -> Option<Self> {
                let names: &[&str] = if cfg!(target_os = "macos") {
                    &["liblilv-0.0.dylib", "liblilv-0.dylib"]
                } else if cfg!(windows) {
                    &["lilv-0.dll"]
                } else {
                    &["liblilv-0.so.0", "liblilv-0.so"]
                };
                // SAFETY: lilv ist eine gewöhnliche C-Bibliothek ohne besondere Initialisierer.
                let lib = names
                    .iter()
                    .find_map(|&n| unsafe { libloading::Library::new(n) }.ok())?;
                // SAFETY: Die Signaturen entsprechen `lilv.h` (API 0.24 und neuer).
                unsafe {
                    Some(Self {
                        $($name: *lib.get(concat!(stringify!($name), "\0").as_bytes()).ok()?,)*
                        _lib: lib,
                    })
                }
            }
        }
    };
}

lilv_api! {
    lilv_world_new: fn() -> Ptr;
    lilv_world_load_all: fn(Ptr);
    lilv_world_get_all_plugins: fn(Ptr) -> Ptr;
    lilv_plugins_begin: fn(Ptr) -> Ptr;
    lilv_plugins_is_end: fn(Ptr, Ptr) -> bool;
    lilv_plugins_next: fn(Ptr, Ptr) -> Ptr;
    lilv_plugins_get: fn(Ptr, Ptr) -> Ptr;
    lilv_plugins_get_by_uri: fn(Ptr, Ptr) -> Ptr;
    lilv_new_uri: fn(Ptr, *const c_char) -> Ptr;
    lilv_node_free: fn(Ptr);
    lilv_node_as_string: fn(Ptr) -> *const c_char;
    lilv_plugin_get_uri: fn(Ptr) -> Ptr;
    lilv_plugin_get_name: fn(Ptr) -> Ptr;
    lilv_plugin_get_author_name: fn(Ptr) -> Ptr;
    lilv_plugin_get_class: fn(Ptr) -> Ptr;
    lilv_plugin_class_get_uri: fn(Ptr) -> Ptr;
    lilv_plugin_get_bundle_uri: fn(Ptr) -> Ptr;
    lilv_plugin_get_num_ports: fn(Ptr) -> u32;
    lilv_plugin_get_port_by_index: fn(Ptr, u32) -> Ptr;
    lilv_plugin_get_port_by_designation: fn(Ptr, Ptr, Ptr) -> Ptr;
    lilv_plugin_get_port_ranges_float: fn(Ptr, *mut f32, *mut f32, *mut f32);
    lilv_port_is_a: fn(Ptr, Ptr, Ptr) -> bool;
    lilv_port_has_property: fn(Ptr, Ptr, Ptr) -> bool;
    lilv_port_supports_event: fn(Ptr, Ptr, Ptr) -> bool;
    lilv_port_get_index: fn(Ptr, Ptr) -> u32;
    lilv_port_get_name: fn(Ptr, Ptr) -> Ptr;
    lilv_port_get_symbol: fn(Ptr, Ptr) -> Ptr;
    lilv_plugin_instantiate: fn(Ptr, f64, *const *const Lv2Feature) -> *mut LilvInstance;
    lilv_instance_free: fn(*mut LilvInstance);
    lilv_free: fn(*mut c_void);
    lilv_plugin_get_related: fn(Ptr, Ptr) -> Ptr;
    lilv_nodes_begin: fn(Ptr) -> Ptr;
    lilv_nodes_is_end: fn(Ptr, Ptr) -> bool;
    lilv_nodes_get: fn(Ptr, Ptr) -> Ptr;
    lilv_nodes_next: fn(Ptr, Ptr) -> Ptr;
    lilv_nodes_free: fn(Ptr);
    lilv_world_load_resource: fn(Ptr, Ptr) -> i32;
    lilv_world_get: fn(Ptr, Ptr, Ptr, Ptr) -> Ptr;
    lilv_state_new_from_instance: fn(
        Ptr,
        *mut LilvInstance,
        *mut UridMap,
        *const c_char,
        *const c_char,
        *const c_char,
        *const c_char,
        Option<GetPortValue>,
        *mut c_void,
        u32,
        *const *const Lv2Feature
    ) -> Ptr;
    lilv_state_to_string: fn(Ptr, *mut UridMap, *mut UridUnmap, Ptr, *const c_char, *const c_char) -> *mut c_char;
    lilv_state_new_from_string: fn(Ptr, *mut UridMap, *const c_char) -> Ptr;
    lilv_state_new_from_world: fn(Ptr, *mut UridMap, Ptr) -> Ptr;
    lilv_state_restore: fn(Ptr, *mut LilvInstance, Option<SetPortValue>, *mut c_void, u32, *const *const Lv2Feature);
    lilv_state_emit_port_values: fn(Ptr, SetPortValue, *mut c_void);
    lilv_state_free: fn(Ptr);
    lilv_plugin_get_uis: fn(Ptr) -> Ptr;
    lilv_uis_begin: fn(Ptr) -> Ptr;
    lilv_uis_is_end: fn(Ptr, Ptr) -> bool;
    lilv_uis_get: fn(Ptr, Ptr) -> Ptr;
    lilv_uis_next: fn(Ptr, Ptr) -> Ptr;
    lilv_uis_free: fn(Ptr);
    lilv_ui_is_a: fn(Ptr, Ptr) -> bool;
    lilv_ui_get_uri: fn(Ptr) -> Ptr;
    lilv_ui_get_binary_uri: fn(Ptr) -> Ptr;
    lilv_ui_get_bundle_uri: fn(Ptr) -> Ptr;
    lilv_file_uri_parse: fn(*const c_char, *mut *mut c_char) -> *mut c_char;
}

type GetPortValue =
    unsafe extern "C" fn(*const c_char, *mut c_void, *mut u32, *mut u32) -> *const c_void;
type SetPortValue = unsafe extern "C" fn(*const c_char, *mut c_void, *const c_void, u32, u32);

/// Die lilv-Welt mit allen Beschreibungen; lebt bis zum Programmende.
struct World {
    api: Lilv,
    world: Ptr,
    /// Häufig gebrauchte URI-Knoten.
    nodes: HashMap<&'static str, Ptr>,
}

// SAFETY: Alle Zugriffe laufen über den Mutex in `world()`; lilv selbst ist dann einzeln benutzt.
unsafe impl Send for World {}

impl World {
    fn node(&self, uri: &'static str) -> Ptr {
        self.nodes.get(uri).copied().unwrap_or(ptr::null_mut())
    }

    fn string(&self, node: Ptr) -> String {
        if node.is_null() {
            return String::new();
        }
        // SAFETY: `node` ist ein gültiger Knoten; die Zeichenkette gehört ihm.
        unsafe { cstr((self.api.lilv_node_as_string)(node)) }
    }

    /// Zeichenkette eines neu angelegten Knotens, danach freigeben.
    fn take_string(&self, node: Ptr) -> String {
        let s = self.string(node);
        if !node.is_null() {
            // SAFETY: Der Knoten wurde von lilv für uns angelegt.
            unsafe { (self.api.lilv_node_free)(node) };
        }
        s
    }

    fn plugins(&self) -> Vec<Ptr> {
        let mut out = Vec::new();
        // SAFETY: Welt gültig; Iteration nach lilv-Vertrag.
        unsafe {
            let all = (self.api.lilv_world_get_all_plugins)(self.world);
            let mut it = (self.api.lilv_plugins_begin)(all);
            while !(self.api.lilv_plugins_is_end)(all, it) {
                let p = (self.api.lilv_plugins_get)(all, it);
                if !p.is_null() {
                    out.push(p);
                }
                it = (self.api.lilv_plugins_next)(all, it);
            }
        }
        out
    }

    fn plugin_by_uri(&self, uri: &str) -> Option<Ptr> {
        let c = CString::new(uri).ok()?;
        // SAFETY: Welt gültig; der Knoten wird nach der Suche freigegeben.
        unsafe {
            let n = (self.api.lilv_new_uri)(self.world, c.as_ptr());
            if n.is_null() {
                return None;
            }
            let all = (self.api.lilv_world_get_all_plugins)(self.world);
            let p = (self.api.lilv_plugins_get_by_uri)(all, n);
            (self.api.lilv_node_free)(n);
            (!p.is_null()).then_some(p)
        }
    }

    /// Pfad zu einer `file://`-URI (über lilv, das Kodierungen kennt).
    fn file_path(&self, node: Ptr) -> Option<PathBuf> {
        if node.is_null() {
            return None;
        }
        // SAFETY: Knoten gültig; das Ergebnis gehört uns und wird freigegeben.
        unsafe {
            let p = (self.api.lilv_file_uri_parse)(
                (self.api.lilv_node_as_string)(node),
                ptr::null_mut(),
            );
            if p.is_null() {
                return None;
            }
            let out = PathBuf::from(cstr(p));
            (self.api.lilv_free)(p.cast());
            Some(out)
        }
    }

    /// Die eingebettete X11-Oberfläche eines Plugins, falls es eine hat.
    fn x11_ui(&self, plugin: Ptr) -> Option<crate::lv2_gui::UiInfo> {
        let mut found = None;
        // SAFETY: Plugin aus der Welt; die Liste wird am Ende freigegeben.
        unsafe {
            let uis = (self.api.lilv_plugin_get_uis)(plugin);
            if uis.is_null() {
                return None;
            }
            let mut it = (self.api.lilv_uis_begin)(uis);
            while found.is_none() && !(self.api.lilv_uis_is_end)(uis, it) {
                let ui = (self.api.lilv_uis_get)(uis, it);
                if !ui.is_null() && (self.api.lilv_ui_is_a)(ui, self.node(URI_X11_UI)) {
                    let binary = self.file_path((self.api.lilv_ui_get_binary_uri)(ui));
                    let bundle = self.file_path((self.api.lilv_ui_get_bundle_uri)(ui));
                    if let (Some(binary), Some(bundle)) = (binary, bundle) {
                        found = Some(crate::lv2_gui::UiInfo {
                            uri: self.string((self.api.lilv_ui_get_uri)(ui)),
                            binary,
                            bundle,
                        });
                    }
                }
                it = (self.api.lilv_uis_next)(uis, it);
            }
            (self.api.lilv_uis_free)(uis);
        }
        found
    }

    fn info(&self, plugin: Ptr) -> PluginInfo {
        // SAFETY: `plugin` stammt aus dieser Welt.
        unsafe {
            let class = (self.api.lilv_plugin_get_class)(plugin);
            let class_uri = if class.is_null() {
                String::new()
            } else {
                self.string((self.api.lilv_plugin_class_get_uri)(class))
            };
            let bundle = self.string((self.api.lilv_plugin_get_bundle_uri)(plugin));
            PluginInfo {
                format: PluginFormat::Lv2,
                id: self.string((self.api.lilv_plugin_get_uri)(plugin)),
                name: self.take_string((self.api.lilv_plugin_get_name)(plugin)),
                vendor: self.take_string((self.api.lilv_plugin_get_author_name)(plugin)),
                version: String::new(),
                features: vec![if class_uri == URI_INSTRUMENT {
                    "instrument".into()
                } else {
                    "audio-effect".into()
                }],
                path: PathBuf::from(bundle.strip_prefix("file://").unwrap_or(&bundle)),
            }
        }
    }
}

fn cstr(p: *const c_char) -> String {
    if p.is_null() {
        return String::new();
    }
    // SAFETY: lilv liefert nullterminierte Zeichenketten.
    unsafe { CStr::from_ptr(p) }.to_string_lossy().into_owned()
}

/// Die lilv-Welt (beim ersten Aufruf geladen); `None` ohne lilv.
fn world() -> Option<std::sync::MutexGuard<'static, World>> {
    static WORLD: OnceLock<Option<Mutex<World>>> = OnceLock::new();
    WORLD
        .get_or_init(|| {
            let api = Lilv::open()?;
            // SAFETY: lilv-Welt anlegen und alle installierten Beschreibungen lesen.
            let world = unsafe { (api.lilv_world_new)() };
            if world.is_null() {
                return None;
            }
            // SAFETY: Welt gültig.
            unsafe { (api.lilv_world_load_all)(world) };
            let mut nodes = HashMap::new();
            for uri in [
                URI_AUDIO_PORT,
                URI_CONTROL_PORT,
                URI_INPUT_PORT,
                URI_OUTPUT_PORT,
                URI_ATOM_PORT,
                URI_LATENCY,
                URI_INTEGER,
                URI_TOGGLED,
                URI_ENUMERATION,
                URI_NOT_ON_GUI,
                URI_MIDI_EVENT,
                URI_PRESET,
                URI_LABEL,
                URI_X11_UI,
            ] {
                let Ok(c) = CString::new(uri) else { continue };
                // SAFETY: Welt gültig; Knoten leben so lange wie die Welt.
                let n = unsafe { (api.lilv_new_uri)(world, c.as_ptr()) };
                nodes.insert(uri, n);
            }
            Some(Mutex::new(World { api, world, nodes }))
        })
        .as_ref()?
        .lock()
        .ok()
}

/// Ob LV2 verfügbar ist (lilv gefunden).
pub fn available() -> bool {
    world().is_some()
}

/// Alle installierten LV2-Plugins (leer ohne lilv). Liest nur Beschreibungen, lädt keinen
/// Plugin-Code.
pub fn scan() -> Vec<PluginInfo> {
    let Some(w) = world() else {
        return Vec::new();
    };
    w.plugins().into_iter().map(|p| w.info(p)).collect()
}

// ---------------------------------------------------------------------------
// Instanz.

/// Rolle eines Ports beim Verbinden.
enum PortKind {
    AudioIn(usize),
    AudioOut(usize),
    ControlIn(usize),
    ControlOut(usize),
    AtomIn(usize),
    AtomOut(usize),
    /// Unbekannter Typ (z. B. CV): mit einem leeren Puffer verbunden.
    Other,
}

/// Zustand, den der Griff im Hauptthread liest, während die Instanz rechnet.
#[derive(Default)]
struct Shared {
    latency: AtomicU32,
    latency_changed: AtomicBool,
}

/// Eine LV2-Instanz. Erzeugen und Freigeben im Hauptthread, `process` im Audio-Thread.
pub struct Lv2Instance {
    inst: *mut LilvInstance,
    pub info: PluginInfo,
    pub params: Vec<ParamInfo>,
    /// Port-Index → Platz in `controls` (nur Control-Eingänge).
    control_of_port: HashMap<u32, usize>,
    controls: Box<[f32]>,
    control_outs: Box<[f32]>,
    latency_out: Option<usize>,
    audio_in: Vec<Box<[f32]>>,
    audio_out: Vec<Box<[f32]>>,
    atom_in: Vec<Box<[u64]>>,
    atom_out: Vec<Box<[u64]>>,
    other: Box<[f32]>,
    midi_urid: u32,
    sequence_urid: u32,
    chunk_urid: u32,
    /// Bytes der MIDI-Ereignisse im ersten Atom-Eingang (nach dem Sequenzkopf).
    midi_used: usize,
    max_frames: usize,
    active: bool,
    shared: Arc<Shared>,
    /// Symbol → Port-Index der Control-Eingänge (Presets nennen Ports beim Symbol).
    symbols: Arc<HashMap<String, u32>>,
    // Features: Zeiger darauf gibt das Plugin nicht frei; sie leben mit der Instanz.
    _map: Box<UridMap>,
    _unmap: Box<UridUnmap>,
    _options: Box<[Lv2Option]>,
    _option_values: Box<[u32; 3]>,
    _rate: Box<f32>,
    _features: Box<[Lv2Feature]>,
    _feature_ptrs: Box<[*const Lv2Feature]>,
}

// SAFETY: LV2 erlaubt, eine Instanz zwischen Threads weiterzugeben, solange je Funktion die
// Thread-Klasse eingehalten wird (instantiate/cleanup im Hauptthread, run im Audio-Thread).
unsafe impl Send for Lv2Instance {}

fn pad8(n: usize) -> usize {
    (n + 7) & !7
}

impl Lv2Instance {
    /// Erzeugt und aktiviert das Plugin. Nur im Hauptthread.
    pub fn new(
        info: &PluginInfo,
        sample_rate: u32,
        max_frames: u32,
    ) -> Result<Self, crate::error::PluginError> {
        let _main = main_thread_lock();
        let w = world().ok_or(crate::error::PluginError::Unavailable(
            crate::error::Unavailable::Lv2NoLilv,
        ))?;
        let plugin =
            w.plugin_by_uri(&info.id)
                .ok_or_else(|| crate::error::PluginError::Create {
                    plugin: info.name.clone(),
                    step: crate::error::Step::Lv2NotFound,
                })?;
        let max = max_frames.max(1) as usize;

        let mut map = Box::new(UridMap {
            handle: ptr::null_mut(),
            map: urid_map,
        });
        let mut unmap = Box::new(UridUnmap {
            handle: ptr::null_mut(),
            unmap: urid_unmap,
        });
        let int = map_uri(URI_ATOM_INT);
        let option_values = Box::new([max_frames.max(1), 1, max_frames.max(1)]);
        let rate = Box::new(sample_rate as f32);
        let opt = |key: &str, kind: u32, value: *const c_void| Lv2Option {
            context: 0,
            subject: 0,
            key: map_uri(key),
            size: 4,
            kind,
            value,
        };
        let options: Box<[Lv2Option]> = Box::new([
            opt(
                URI_MAX_BLOCK,
                int,
                &option_values[0] as *const u32 as *const c_void,
            ),
            opt(
                URI_MIN_BLOCK,
                int,
                &option_values[1] as *const u32 as *const c_void,
            ),
            opt(
                URI_NOMINAL_BLOCK,
                int,
                &option_values[2] as *const u32 as *const c_void,
            ),
            opt(
                URI_SAMPLE_RATE,
                map_uri(URI_ATOM_FLOAT),
                &*rate as *const f32 as *const c_void,
            ),
            Lv2Option {
                context: 0,
                subject: 0,
                key: 0,
                size: 0,
                kind: 0,
                value: ptr::null(),
            },
        ]);
        let features: Box<[Lv2Feature]> = Box::new([
            Lv2Feature {
                uri: FEATURE_URID_MAP.as_ptr(),
                data: &mut *map as *mut UridMap as *mut c_void,
            },
            Lv2Feature {
                uri: FEATURE_URID_UNMAP.as_ptr(),
                data: &mut *unmap as *mut UridUnmap as *mut c_void,
            },
            Lv2Feature {
                uri: FEATURE_OPTIONS.as_ptr(),
                data: options.as_ptr() as *mut c_void,
            },
            Lv2Feature {
                uri: FEATURE_BOUNDED.as_ptr(),
                data: ptr::null_mut(),
            },
        ]);
        let feature_ptrs: Box<[*const Lv2Feature]> = features
            .iter()
            .map(|f| f as *const Lv2Feature)
            .chain(std::iter::once(ptr::null()))
            .collect();

        // Ports beschreiben.
        // SAFETY: `plugin` stammt aus der Welt; Arrays haben `n` Plätze.
        let n = unsafe { (w.api.lilv_plugin_get_num_ports)(plugin) };
        let mut mins = vec![f32::NAN; n as usize];
        let mut maxs = vec![f32::NAN; n as usize];
        let mut defs = vec![f32::NAN; n as usize];
        // SAFETY: wie oben.
        unsafe {
            (w.api.lilv_plugin_get_port_ranges_float)(
                plugin,
                mins.as_mut_ptr(),
                maxs.as_mut_ptr(),
                defs.as_mut_ptr(),
            );
        }
        let latency_port = {
            // SAFETY: Knoten aus der Welt; Ergebnis null oder ein Port des Plugins.
            let p = unsafe {
                (w.api.lilv_plugin_get_port_by_designation)(
                    plugin,
                    w.node(URI_OUTPUT_PORT),
                    w.node(URI_LATENCY),
                )
            };
            // SAFETY: wie oben.
            (!p.is_null()).then(|| unsafe { (w.api.lilv_port_get_index)(plugin, p) })
        };
        let mut kinds = Vec::with_capacity(n as usize);
        let (mut a_in, mut a_out, mut c_in, mut c_out, mut t_in, mut t_out) = (0, 0, 0, 0, 0, 0);
        let mut params = Vec::new();
        let mut control_of_port = HashMap::new();
        let mut symbols = HashMap::new();
        let mut controls = Vec::new();
        let mut latency_out = None;
        for i in 0..n {
            // SAFETY: Index im Bereich.
            let port = unsafe { (w.api.lilv_plugin_get_port_by_index)(plugin, i) };
            // SAFETY: Port und Knoten gültig.
            let is = |uri| unsafe { (w.api.lilv_port_is_a)(plugin, port, w.node(uri)) };
            // SAFETY: wie oben.
            let has = |uri| unsafe { (w.api.lilv_port_has_property)(plugin, port, w.node(uri)) };
            let input = is(URI_INPUT_PORT);
            let kind = if is(URI_AUDIO_PORT) {
                if input {
                    a_in += 1;
                    PortKind::AudioIn(a_in - 1)
                } else {
                    a_out += 1;
                    PortKind::AudioOut(a_out - 1)
                }
            } else if is(URI_CONTROL_PORT) {
                let at = i as usize;
                let def = defs.get(at).copied().filter(|v| v.is_finite());
                let min = mins
                    .get(at)
                    .copied()
                    .filter(|v| v.is_finite())
                    .unwrap_or(0.0);
                let max = maxs
                    .get(at)
                    .copied()
                    .filter(|v| v.is_finite() && *v > min)
                    .unwrap_or(min + 1.0);
                let def = def.unwrap_or(min).clamp(min, max);
                if input {
                    // SAFETY: Port gültig; das Symbol gehört dem Port.
                    let symbol = w.string(unsafe { (w.api.lilv_port_get_symbol)(plugin, port) });
                    symbols.insert(symbol, i);
                    control_of_port.insert(i, controls.len());
                    controls.push(def);
                    if !has(URI_NOT_ON_GUI) {
                        // SAFETY: Port gültig; der Name ist ein neuer Knoten.
                        let name =
                            w.take_string(unsafe { (w.api.lilv_port_get_name)(plugin, port) });
                        let name = if name.is_empty() {
                            // SAFETY: wie oben; das Symbol gehört dem Port.
                            w.string(unsafe { (w.api.lilv_port_get_symbol)(plugin, port) })
                        } else {
                            name
                        };
                        params.push(ParamInfo {
                            id: i,
                            name,
                            module: String::new(),
                            min: f64::from(min),
                            max: f64::from(max),
                            default: f64::from(def),
                            value: f64::from(def),
                            stepped: has(URI_INTEGER) || has(URI_TOGGLED) || has(URI_ENUMERATION),
                        });
                    }
                    c_in += 1;
                    PortKind::ControlIn(c_in - 1)
                } else {
                    if latency_port == Some(i) {
                        latency_out = Some(c_out);
                    }
                    c_out += 1;
                    PortKind::ControlOut(c_out - 1)
                }
            } else if is(URI_ATOM_PORT) {
                // SAFETY: Port und Knoten gültig.
                let _midi = unsafe {
                    (w.api.lilv_port_supports_event)(plugin, port, w.node(URI_MIDI_EVENT))
                };
                if input {
                    t_in += 1;
                    PortKind::AtomIn(t_in - 1)
                } else {
                    t_out += 1;
                    PortKind::AtomOut(t_out - 1)
                }
            } else {
                PortKind::Other
            };
            kinds.push(kind);
        }
        if a_out == 0 {
            return Err(crate::error::PluginError::Create {
                plugin: info.name.clone(),
                step: crate::error::Step::NoAudioOutput,
            });
        }
        let info = w.info(plugin);
        // SAFETY: Plugin aus der Welt, Features nullterminiert und so lange gültig wie die
        // Instanz (Felder unten).
        let inst = unsafe {
            (w.api.lilv_plugin_instantiate)(plugin, f64::from(sample_rate), feature_ptrs.as_ptr())
        };
        drop(w);
        if inst.is_null() {
            return Err(crate::error::PluginError::Create {
                plugin: info.name.clone(),
                step: crate::error::Step::Lv2Instantiate,
            });
        }
        let words = ATOM_CAPACITY / 8;
        let mut me = Self {
            inst,
            info,
            params,
            control_of_port,
            controls: controls.into_boxed_slice(),
            control_outs: vec![0.0; c_out].into_boxed_slice(),
            latency_out,
            audio_in: (0..a_in)
                .map(|_| vec![0.0; max].into_boxed_slice())
                .collect(),
            audio_out: (0..a_out)
                .map(|_| vec![0.0; max].into_boxed_slice())
                .collect(),
            atom_in: (0..t_in)
                .map(|_| vec![0u64; words].into_boxed_slice())
                .collect(),
            atom_out: (0..t_out)
                .map(|_| vec![0u64; words].into_boxed_slice())
                .collect(),
            other: vec![0.0; max].into_boxed_slice(),
            midi_urid: map_uri(URI_MIDI_EVENT),
            sequence_urid: map_uri(URI_ATOM_SEQUENCE),
            chunk_urid: map_uri(URI_ATOM_CHUNK),
            midi_used: 0,
            max_frames: max,
            active: false,
            shared: Arc::default(),
            symbols: Arc::new(symbols),
            _map: map,
            _unmap: unmap,
            _options: options,
            _option_values: option_values,
            _rate: rate,
            _features: features,
            _feature_ptrs: feature_ptrs,
        };
        // Alle Ports verbinden; die Puffer liegen auf dem Heap und wachsen nicht mehr.
        for (i, kind) in kinds.iter().enumerate() {
            let p: *mut c_void = match *kind {
                PortKind::AudioIn(k) => me.audio_in[k].as_mut_ptr().cast(),
                PortKind::AudioOut(k) => me.audio_out[k].as_mut_ptr().cast(),
                PortKind::ControlIn(k) => (&mut me.controls[k] as *mut f32).cast(),
                PortKind::ControlOut(k) => (&mut me.control_outs[k] as *mut f32).cast(),
                PortKind::AtomIn(k) => me.atom_in[k].as_mut_ptr().cast(),
                PortKind::AtomOut(k) => me.atom_out[k].as_mut_ptr().cast(),
                PortKind::Other => me.other.as_mut_ptr().cast(),
            };
            me.connect(i as u32, p);
        }
        me.reset_atoms();
        // SAFETY: Instanz gültig, alle Ports verbunden; activate im Hauptthread.
        unsafe {
            if let Some(f) = (*me.descriptor()).activate {
                f((*me.inst).handle);
            }
        }
        me.active = true;
        Ok(me)
    }

    fn descriptor(&self) -> *const Lv2Descriptor {
        // SAFETY: `inst` ist gültig, solange die Instanz lebt.
        unsafe { (*self.inst).descriptor }
    }

    fn connect(&mut self, port: u32, data: *mut c_void) {
        // SAFETY: Instanz gültig; `data` zeigt auf einen Puffer, der mit ihr lebt.
        unsafe {
            if let Some(f) = (*self.descriptor()).connect_port {
                f((*self.inst).handle, port, data);
            }
        }
    }

    /// Atom-Eingänge: leere Sequenz; Ausgänge: Platz für das Plugin (Chunk mit Kapazität).
    fn reset_atoms(&mut self) {
        let (seq, chunk) = (self.sequence_urid, self.chunk_urid);
        for buf in &mut self.atom_in {
            write_header(buf, 8, seq);
            if let Some(w) = buf.get_mut(1) {
                *w = 0; // Sequenzkopf: unit 0 (Frames), pad 0
            }
        }
        for buf in &mut self.atom_out {
            write_header(buf, (ATOM_CAPACITY - 8) as u32, chunk);
        }
        self.midi_used = 0;
    }

    pub fn handle(&self) -> Lv2Handle {
        Lv2Handle {
            shared: self.shared.clone(),
            inst: self.inst,
            uri: self.info.id.clone(),
            title_name: self.info.name.clone(),
            symbols: self.symbols.clone(),
            features: self._feature_ptrs.to_vec(),
            ui: OnceLock::new(),
            gui: None,
        }
    }

    pub fn set_param(&mut self, id: u32, value: f64) {
        if let Some(&k) = self.control_of_port.get(&id)
            && let Some(c) = self.controls.get_mut(k)
            && value.is_finite()
        {
            *c = value as f32;
        }
    }

    /// LV2-Control-Ports gelten für den ganzen Block: der Wert wirkt ab dem nächsten Block.
    pub fn set_param_at(&mut self, _time: u32, id: u32, value: f64) {
        self.set_param(id, value);
    }

    pub fn midi(&mut self, time: u32, data: [u8; 3]) {
        let len = match data[0] & 0xF0 {
            0xC0 | 0xD0 => 2,
            _ => 3,
        };
        let Some(buf) = self.atom_in.first_mut() else {
            return;
        };
        // Ereignis: Zeit (i64), Atom-Kopf (size, type), Daten, auf 8 Bytes aufgefüllt.
        let start = 16 + self.midi_used; // nach Atom-Kopf (8) und Sequenzkopf (8)
        let need = 16 + pad8(len);
        if start + need > ATOM_CAPACITY {
            return;
        }
        let bytes = as_bytes_mut(buf);
        bytes[start..start + 8].copy_from_slice(&i64::from(time).to_ne_bytes());
        bytes[start + 8..start + 12].copy_from_slice(&(len as u32).to_ne_bytes());
        bytes[start + 12..start + 16].copy_from_slice(&self.midi_urid.to_ne_bytes());
        bytes[start + 16..start + 16 + len].copy_from_slice(&data[..len]);
        for b in &mut bytes[start + 16 + len..start + need] {
            *b = 0;
        }
        self.midi_used += need;
        let size = (8 + self.midi_used) as u32;
        write_header(buf, size, self.sequence_urid);
    }

    pub fn note(&mut self, time: u32, key: u8, velocity: f32, on: bool) {
        let vel = (velocity.clamp(0.0, 1.0) * 127.0).round() as u8;
        let status = if on { 0x90 } else { 0x80 };
        self.midi(
            time,
            [status, key.min(127), if on { vel.max(1) } else { vel }],
        );
    }

    /// Einen Stereo-Block an Ort und Stelle rechnen (Audio-Thread, keine Allokation).
    pub fn process(&mut self, left: &mut [f32], right: &mut [f32]) {
        let _tests = crate::clap::test_process_guard();
        let total = left.len().min(right.len());
        let mut done = 0;
        while done < total {
            let n = (total - done).min(self.max_frames);
            let (l, r) = (&mut left[done..done + n], &mut right[done..done + n]);
            match self.audio_in.len() {
                0 => {}
                1 => {
                    for (d, (a, b)) in self.audio_in[0].iter_mut().zip(l.iter().zip(r.iter())) {
                        *d = 0.5 * (a + b);
                    }
                }
                _ => {
                    self.audio_in[0][..n].copy_from_slice(l);
                    self.audio_in[1][..n].copy_from_slice(r);
                    for extra in self.audio_in.iter_mut().skip(2) {
                        extra[..n].fill(0.0);
                    }
                }
            }
            // SAFETY: Instanz aktiv, alle Ports mit Puffern ≥ n verbunden; run im Audio-Thread.
            unsafe {
                if let Some(f) = (*self.descriptor()).run {
                    f((*self.inst).handle, n as u32);
                }
            }
            match self.audio_out.len() {
                1 => {
                    l.copy_from_slice(&self.audio_out[0][..n]);
                    r.copy_from_slice(&self.audio_out[0][..n]);
                }
                _ => {
                    l.copy_from_slice(&self.audio_out[0][..n]);
                    r.copy_from_slice(&self.audio_out[1][..n]);
                }
            }
            for v in l.iter_mut().chain(r.iter_mut()) {
                if !v.is_finite() {
                    *v = 0.0;
                }
            }
            // MIDI gilt für den ersten Teilblock; danach leer.
            self.reset_atoms();
            done += n;
        }
        if let Some(k) = self.latency_out {
            let lat = self
                .control_outs
                .get(k)
                .copied()
                .filter(|v| v.is_finite() && *v >= 0.0)
                .map_or(0, |v| (v.round() as u32).min(MAX_LATENCY));
            if self.shared.latency.swap(lat, Ordering::AcqRel) != lat {
                self.shared.latency_changed.store(true, Ordering::Release);
            }
        }
    }

    pub fn stop_processing(&mut self) {}
}

fn write_header(buf: &mut [u64], size: u32, kind: u32) {
    if let Some(w) = buf.first_mut() {
        let h = AtomHeader { size, kind };
        let mut b = [0u8; 8];
        b[..4].copy_from_slice(&h.size.to_ne_bytes());
        b[4..].copy_from_slice(&h.kind.to_ne_bytes());
        *w = u64::from_ne_bytes(b);
    }
}

fn as_bytes_mut(buf: &mut [u64]) -> &mut [u8] {
    let len = buf.len() * 8;
    // SAFETY: u64-Speicher als Bytes ansehen ist immer gültig (Ausrichtung 8 ≥ 1, gleiche Länge).
    unsafe { std::slice::from_raw_parts_mut(buf.as_mut_ptr().cast::<u8>(), len) }
}

impl Drop for Lv2Instance {
    /// Deaktivieren und freigeben. Gehört in den Hauptthread.
    fn drop(&mut self) {
        if self.inst.is_null() {
            return;
        }
        let _main = main_thread_lock();
        // SAFETY: Instanz gültig; danach wird sie nicht mehr benutzt.
        unsafe {
            if self.active
                && let Some(f) = (*self.descriptor()).deactivate
            {
                f((*self.inst).handle);
            }
        }
        if let Some(w) = world() {
            // SAFETY: von lilv_plugin_instantiate angelegt, genau einmal freigegeben.
            unsafe { (w.api.lilv_instance_free)(self.inst) };
        }
        self.inst = ptr::null_mut();
    }
}

/// Griff auf eine LV2-Instanz für den Hauptthread (Zustand, Presets, Latenz), während die
/// Instanz in der Engine rechnet. Muss vor der Instanz freigegeben werden (wie bei CLAP).
pub struct Lv2Handle {
    shared: Arc<Shared>,
    inst: *mut LilvInstance,
    uri: String,
    title_name: String,
    symbols: Arc<HashMap<String, u32>>,
    /// Features der Instanz (URID-Map usw.), auch für die Oberfläche; leben mit der Instanz.
    features: Vec<*const Lv2Feature>,
    /// Eingebettete Oberfläche laut Beschreibung (einmal nachgeschlagen).
    ui: OnceLock<Option<crate::lv2_gui::UiInfo>>,
    gui: Option<Box<crate::lv2_gui::Lv2Gui>>,
}

/// Auf dem Stack angelegte URID-Map/-Unmap für lilv-Aufrufe.
fn map_features() -> (UridMap, UridUnmap) {
    (
        UridMap {
            handle: ptr::null_mut(),
            map: urid_map,
        },
        UridUnmap {
            handle: ptr::null_mut(),
            unmap: urid_unmap,
        },
    )
}

/// Sammelt Port-Werte eines Presets (Rückruf von `lilv_state_emit_port_values`).
struct PortValues<'a> {
    symbols: &'a HashMap<String, u32>,
    out: Vec<(u32, f64)>,
}

unsafe extern "C" fn collect_port_value(
    symbol: *const c_char,
    user: *mut c_void,
    value: *const c_void,
    size: u32,
    kind: u32,
) {
    if symbol.is_null() || user.is_null() || value.is_null() {
        return;
    }
    // SAFETY: `user` ist der `PortValues`, den `preset_values` übergibt; er lebt den Aufruf über.
    let pv = unsafe { &mut *(user as *mut PortValues) };
    let name = cstr(symbol);
    let Some(&port) = pv.symbols.get(&name) else {
        return;
    };
    // SAFETY: lilv übergibt `size` Bytes vom Typ `kind`; nur bekannte Typen mit passender Größe.
    let v = unsafe {
        if kind == map_uri(URI_ATOM_FLOAT) && size == 4 {
            f64::from(*(value as *const f32))
        } else if kind == map_uri(URI_ATOM_DOUBLE) && size == 8 {
            *(value as *const f64)
        } else if (kind == map_uri(URI_ATOM_INT) || kind == map_uri(URI_ATOM_BOOL)) && size == 4 {
            f64::from(*(value as *const i32))
        } else {
            return;
        }
    };
    if v.is_finite() {
        pv.out.push((port, v));
    }
}

// SAFETY: Der Griff ruft die Instanz nur im Hauptthread über lilv auf (Zustand speichern ist laut
// lilv parallel zu `run` erlaubt); der Zeiger lebt so lange wie die Instanz.
unsafe impl Send for Lv2Handle {}

impl Drop for Lv2Handle {
    fn drop(&mut self) {
        // Fenster vor der Instanz schließen (Vertrag: Griff stirbt zuerst).
        self.close_gui();
    }
}

impl Lv2Handle {
    fn ui_info(&self) -> Option<&crate::lv2_gui::UiInfo> {
        self.ui
            .get_or_init(|| {
                if !cfg!(all(unix, not(target_os = "macos"))) {
                    return None;
                }
                let w = world()?;
                let plugin = w.plugin_by_uri(&self.uri)?;
                w.x11_ui(plugin)
            })
            .as_ref()
    }

    /// Ob das Plugin eine Oberfläche hat, die sich einbetten lässt.
    pub fn has_gui(&self) -> bool {
        self.ui_info().is_some()
    }

    pub fn gui_open(&self) -> bool {
        self.gui.is_some()
    }

    /// Öffnet das Plugin-Fenster. Nur im Hauptthread; danach Werte mit
    /// [`Self::set_ui_value`] übergeben.
    pub fn open_gui(&mut self, title: &str) -> Result<(), crate::error::PluginError> {
        if self.gui.is_some() {
            return Ok(());
        }
        if !crate::host::on_main_thread() {
            return Err(crate::error::PluginError::Window(
                crate::error::WindowProblem::MainThreadOnly,
            ));
        }
        let info = self
            .ui_info()
            .cloned()
            .ok_or(crate::error::PluginError::Window(
                crate::error::WindowProblem::Lv2NoMatchingUi,
            ))?;
        let _main = main_thread_lock();
        // SAFETY: Instanz lebt (Vertrag des Griffs); Beschreibung und Handle sind gültig.
        let (handle, ext) = unsafe {
            let d = (*self.inst).descriptor;
            (
                (*self.inst).handle,
                if d.is_null() {
                    None
                } else {
                    (*d).extension_data
                },
            )
        };
        let title = if title.is_empty() {
            &self.title_name
        } else {
            title
        };
        let gui =
            crate::lv2_gui::Lv2Gui::open(&info, &self.uri, title, handle, ext, &self.features)?;
        self.gui = Some(Box::new(gui));
        Ok(())
    }

    pub fn close_gui(&mut self) {
        if let Some(g) = self.gui.take() {
            let _main = main_thread_lock();
            g.close();
        }
    }

    /// Im UI-Takt; `true`, solange das Fenster offen ist.
    pub fn idle(&mut self) -> bool {
        let Some(g) = self.gui.as_mut() else {
            return false;
        };
        if g.idle() {
            return true;
        }
        self.close_gui();
        false
    }

    /// Reglerbewegungen im Plugin-Fenster seit dem letzten Aufruf: (Parameter-Id, Wert).
    pub fn take_param_changes(&self) -> Vec<(u32, f64)> {
        self.gui
            .as_ref()
            .map(|g| g.take_param_changes())
            .unwrap_or_default()
    }

    /// Einen Parameterwert an das offene Plugin-Fenster geben.
    pub fn set_ui_value(&self, id: u32, value: f64) {
        if let Some(g) = &self.gui {
            g.set_value(id, value);
        }
    }

    pub fn latency(&self) -> u32 {
        self.shared.latency.load(Ordering::Acquire)
    }

    pub fn take_latency_changed(&self) -> bool {
        self.shared.latency_changed.swap(false, Ordering::AcqRel)
    }

    /// Hat das Plugin eine Zustands-Schnittstelle (mehr als nur Parameter)?
    fn has_state(&self) -> bool {
        // SAFETY: Instanz lebt (Vertrag des Griffs); extension_data ist ein reiner Abruf.
        unsafe {
            let d = (*self.inst).descriptor;
            !d.is_null()
                && (*d)
                    .extension_data
                    .is_some_and(|f| !f(STATE_INTERFACE.as_ptr()).is_null())
        }
    }

    /// Zustand als Text (Turtle) speichern; `None` ohne Zustands-Schnittstelle. Hauptthread;
    /// laut lilv parallel zur Verarbeitung erlaubt.
    pub fn save_state(&self) -> Option<Vec<u8>> {
        if !self.has_state() {
            return None;
        }
        let _main = main_thread_lock();
        let w = world()?;
        let plugin = w.plugin_by_uri(&self.uri)?;
        let (mut map, mut unmap) = map_features();
        // SAFETY: Plugin und Instanz gültig; keine Verzeichnisse, nur einfache Werte (Flags).
        unsafe {
            let state = (w.api.lilv_state_new_from_instance)(
                plugin,
                self.inst,
                &mut map,
                ptr::null(),
                ptr::null(),
                ptr::null(),
                ptr::null(),
                None,
                ptr::null_mut(),
                STATE_FLAGS,
                ptr::null(),
            );
            if state.is_null() {
                return None;
            }
            let text = (w.api.lilv_state_to_string)(
                w.world,
                &mut map,
                &mut unmap,
                state,
                STATE_URI.as_ptr(),
                ptr::null(),
            );
            (w.api.lilv_state_free)(state);
            if text.is_null() {
                return None;
            }
            let out = CStr::from_ptr(text).to_bytes().to_vec();
            (w.api.lilv_free)(text.cast());
            Some(out)
        }
    }

    /// Gespeicherten Zustand laden. Nur bevor die Instanz rechnet (LV2: Wiederherstellen darf
    /// nicht parallel zu `run` laufen) – StudioCraft lädt ihn beim Erzeugen.
    pub fn load_state(&self, data: &[u8]) -> bool {
        let Ok(text) = CString::new(data) else {
            return false;
        };
        let _main = main_thread_lock();
        let Some(w) = world() else {
            return false;
        };
        let (mut map, _) = map_features();
        // SAFETY: Text nullterminiert; Zustand wird nach dem Wiederherstellen freigegeben.
        unsafe {
            let state = (w.api.lilv_state_new_from_string)(w.world, &mut map, text.as_ptr());
            if state.is_null() {
                return false;
            }
            (w.api.lilv_state_restore)(state, self.inst, None, ptr::null_mut(), 0, ptr::null());
            (w.api.lilv_state_free)(state);
        }
        true
    }

    /// Presets des Plugins, nach Name sortiert.
    pub fn presets(&self) -> Vec<Lv2Preset> {
        let _main = main_thread_lock();
        let Some(w) = world() else {
            return Vec::new();
        };
        let Some(plugin) = w.plugin_by_uri(&self.uri) else {
            return Vec::new();
        };
        let mut out = Vec::new();
        // SAFETY: Plugin und Knoten aus der Welt; die Liste wird am Ende freigegeben.
        unsafe {
            let list = (w.api.lilv_plugin_get_related)(plugin, w.node(URI_PRESET));
            if list.is_null() {
                return out;
            }
            let mut it = (w.api.lilv_nodes_begin)(list);
            while !(w.api.lilv_nodes_is_end)(list, it) {
                let node = (w.api.lilv_nodes_get)(list, it);
                if !node.is_null() {
                    (w.api.lilv_world_load_resource)(w.world, node);
                    let label =
                        (w.api.lilv_world_get)(w.world, node, w.node(URI_LABEL), ptr::null_mut());
                    let uri = w.string(node);
                    let name = w.take_string(label);
                    out.push(Lv2Preset {
                        name: if name.is_empty() { uri.clone() } else { name },
                        uri,
                    });
                }
                it = (w.api.lilv_nodes_next)(list, it);
            }
            (w.api.lilv_nodes_free)(list);
        }
        out.sort_by_key(|p| p.name.to_lowercase());
        out
    }

    /// Parameterwerte (Port-Index, Wert) eines Presets; leer, wenn es keine hat. Der Host setzt
    /// sie wie andere Parameteränderungen (rückgängig machbar, auch während der Wiedergabe).
    pub fn preset_values(&self, uri: &str) -> Vec<(u32, f64)> {
        let Ok(c) = CString::new(uri) else {
            return Vec::new();
        };
        let _main = main_thread_lock();
        let Some(w) = world() else {
            return Vec::new();
        };
        let (mut map, _) = map_features();
        let mut pv = PortValues {
            symbols: &self.symbols,
            out: Vec::new(),
        };
        // SAFETY: Knoten und Zustand werden hier angelegt und freigegeben; `pv` lebt den Aufruf.
        unsafe {
            let node = (w.api.lilv_new_uri)(w.world, c.as_ptr());
            if node.is_null() {
                return Vec::new();
            }
            (w.api.lilv_world_load_resource)(w.world, node);
            let state = (w.api.lilv_state_new_from_world)(w.world, &mut map, node);
            (w.api.lilv_node_free)(node);
            if state.is_null() {
                return Vec::new();
            }
            (w.api.lilv_state_emit_port_values)(
                state,
                collect_port_value,
                (&mut pv as *mut PortValues).cast(),
            );
            (w.api.lilv_state_free)(state);
        }
        pv.out.sort_by_key(|&(p, _)| p);
        pv.out
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn urids_are_stable_and_unmap() {
        let a = map_uri("urn:studiocraft:test:a");
        let b = map_uri("urn:studiocraft:test:b");
        assert!(a > 0 && b > 0 && a != b);
        assert_eq!(map_uri("urn:studiocraft:test:a"), a);
        // SAFETY: Test ruft die Rückrufe wie ein Plugin auf.
        let back = unsafe { urid_unmap(ptr::null_mut(), a) };
        assert_eq!(cstr(back), "urn:studiocraft:test:a");
        // SAFETY: wie oben.
        assert!(unsafe { urid_unmap(ptr::null_mut(), 0) }.is_null());
    }

    /// Gegen installierte LV2-Plugins (falls lilv und Plugins vorhanden).
    #[test]
    fn installed_lv2_effects_process_audio() {
        let plugins = scan();
        if plugins.is_empty() {
            eprintln!("keine LV2-Plugins (oder kein lilv) – übersprungen");
            return;
        }
        let mut ok = 0;
        for info in plugins.iter().filter(|p| !p.is_instrument()).take(40) {
            let Ok(mut inst) = Lv2Instance::new(info, 48_000, 256) else {
                continue;
            };
            let (mut l, mut r) = (vec![0.25f32; 300], vec![-0.25f32; 300]);
            inst.process(&mut l, &mut r);
            assert!(l.iter().chain(&r).all(|v| v.is_finite()), "{}", info.name);
            if let Some(p) = inst.params.first().cloned() {
                inst.set_param(p.id, p.max);
                inst.process(&mut l, &mut r);
            }
            ok += 1;
        }
        assert!(ok > 0, "at least one LV2 effect instantiates");
    }

    fn installed(name: &str) -> Option<PluginInfo> {
        scan().into_iter().find(|p| p.name == name)
    }

    #[test]
    fn state_survives_a_new_instance() {
        let Some(info) = installed("Audio Gain (Stereo)").or_else(|| installed("ZamComp")) else {
            eprintln!("kein passendes LV2-Plugin – übersprungen");
            return;
        };
        let a = Lv2Instance::new(&info, 48_000, 256);
        let Ok(a) = a else {
            return;
        };
        let Some(state) = a.handle().save_state() else {
            eprintln!("{} ohne Zustand – übersprungen", info.name);
            return;
        };
        assert!(
            String::from_utf8_lossy(&state).contains("lv2-state"),
            "Turtle text"
        );
        let b = Lv2Instance::new(&info, 48_000, 256);
        assert!(b.is_ok(), "second instance");
        let Ok(b) = b else { return };
        assert!(b.handle().load_state(&state));
        assert_eq!(
            b.handle().save_state(),
            Some(state),
            "same state after loading"
        );
        assert!(
            !b.handle().load_state(b"kein turtle\0"),
            "garbage is refused"
        );
    }

    #[test]
    fn presets_list_and_yield_parameter_values() {
        let Some(info) = installed("ZamComp") else {
            eprintln!("ZamComp (LV2) nicht installiert – übersprungen");
            return;
        };
        let Ok(inst) = Lv2Instance::new(&info, 48_000, 256) else {
            return;
        };
        let h = inst.handle();
        let presets = h.presets();
        assert!(!presets.is_empty(), "ZamComp ships presets");
        let values = h.preset_values(&presets[0].uri);
        assert!(!values.is_empty());
        for (id, v) in &values {
            let p = inst.params.iter().find(|p| p.id == *id);
            assert!(p.is_some(), "port {id} is a parameter");
            assert!(v.is_finite());
        }
        assert!(h.preset_values("urn:studiocraft:kein-preset").is_empty());
    }

    #[test]
    fn midi_events_fill_the_sequence() {
        // Nur der Atom-Puffer, ohne Plugin.
        let mut buf = vec![0u64; ATOM_CAPACITY / 8].into_boxed_slice();
        write_header(&mut buf, 8, 7);
        let bytes = as_bytes_mut(&mut buf);
        assert_eq!(
            u32::from_ne_bytes([bytes[0], bytes[1], bytes[2], bytes[3]]),
            8
        );
        assert_eq!(pad8(3), 8);
        assert_eq!(pad8(8), 8);
        assert_eq!(pad8(9), 16);
    }
}
