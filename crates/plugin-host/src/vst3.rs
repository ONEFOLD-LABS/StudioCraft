//! VST3-Hosting (Issue #32) mit eigenen, schlanken Bindungen: nur die Schnittstellen, die ein
//! Host zum Laden, Rechnen und für Parameter braucht. Kennungen und Speicherlayout folgen der
//! öffentlichen Formatbeschreibung (COM-artige Schnittstellen mit Vtabellen).
//!
//! Umfang: Bundles finden und Klassen lesen, Komponente + Audioprozessor + Controller anlegen,
//! Stereo-Audio (Mono-Busse bekommen die Summe), Parameter (normiert; gestufte als Stufen),
//! Noten als Ereignisse, Zustand (Komponente + Controller) und Latenzänderungen. Fenster folgen.

use std::{
    collections::HashMap,
    ffi::{c_char, c_void},
    path::{Path, PathBuf},
    ptr,
    sync::{
        Arc, Mutex, OnceLock,
        atomic::{AtomicU32, Ordering},
    },
};

use crate::{
    clap::{MAX_LATENCY, ParamInfo, main_thread_lock},
    scan::{PluginFormat, PluginInfo},
};

// ---------------------------------------------------------------------------
// Grundtypen.

pub(crate) type Tresult = i32;
pub(crate) const RESULT_OK: Tresult = 0;
pub(crate) type Tuid = [u8; 16];
pub(crate) type Obj = *mut c_void;

/// Kennung aus vier 32-Bit-Teilen wie in der Formatbeschreibung. Unter Windows im
/// COM-Layout (erste drei Felder little-endian), sonst durchgehend big-endian.
pub(crate) const fn uid(a: u32, b: u32, c: u32, d: u32) -> Tuid {
    let (a, b, c, d) = (
        a.to_be_bytes(),
        b.to_be_bytes(),
        c.to_be_bytes(),
        d.to_be_bytes(),
    );
    if cfg!(windows) {
        [
            a[3], a[2], a[1], a[0], b[1], b[0], b[3], b[2], c[0], c[1], c[2], c[3], d[0], d[1],
            d[2], d[3],
        ]
    } else {
        [
            a[0], a[1], a[2], a[3], b[0], b[1], b[2], b[3], c[0], c[1], c[2], c[3], d[0], d[1],
            d[2], d[3],
        ]
    }
}

const IID_UNKNOWN: Tuid = uid(0x0000_0000, 0x0000_0000, 0xC000_0000, 0x0000_0046);
const IID_FACTORY2: Tuid = uid(0x0007_B650, 0xF24B_4C0B, 0xA464_EDB9, 0xF00B_2ABB);
const IID_COMPONENT: Tuid = uid(0xE831_FF31, 0xF2D5_4301, 0x928E_BBEE, 0x2569_7802);
const IID_AUDIO_PROCESSOR: Tuid = uid(0x4204_3F99, 0xB7DA_453C, 0xA569_E79D, 0x9AAE_C33D);
const IID_EDIT_CONTROLLER: Tuid = uid(0xDCD7_BBE3, 0x7742_448D, 0xA874_AACC, 0x979C_759E);
const IID_CONNECTION_POINT: Tuid = uid(0x70A4_156F, 0x6E6E_4026, 0x9891_48BF, 0xAA60_D8D1);
const IID_HOST_APPLICATION: Tuid = uid(0x58E5_95CC, 0xDB2D_4969, 0x8B6A_AF8C, 0x36A6_64E5);
const IID_PARAMETER_CHANGES: Tuid = uid(0xA477_9663, 0x0BB6_4A56, 0xB443_84A8, 0x466F_EB9D);
const IID_PARAM_VALUE_QUEUE: Tuid = uid(0x0126_3A18, 0xED07_4F6F, 0x98C9_D356, 0x4686_F9BA);
const IID_EVENT_LIST: Tuid = uid(0x3A2C_4214, 0x3463_49FE, 0xB2C4_F397, 0xB969_5A44);
const IID_COMPONENT_HANDLER: Tuid = uid(0x93A0_BEA3, 0x0BD0_45DB, 0x8E89_0B0C, 0xC1E4_6AC6);
const IID_STREAM: Tuid = uid(0xC3BF_6EA2, 0x3099_4752, 0x9B6B_F990, 0x1EE3_3E9B);

/// Kennung gespeicherter VST3-Zustände in StudioCraft-Projekten (Komponente + Controller).
const STATE_MAGIC: &[u8; 4] = b"SCV3";
/// Höchstgröße eines Zustands (Schutz gegen entgleiste Plugins und kaputte Dateien).
const MAX_STATE: usize = 256 << 20;

const MEDIA_AUDIO: i32 = 0;
const MEDIA_EVENT: i32 = 1;
const DIR_IN: i32 = 0;
const DIR_OUT: i32 = 1;
const SPEAKER_STEREO: u64 = 3;
const SPEAKER_MONO: u64 = 1 << 19;
const PARAM_READ_ONLY: i32 = 1 << 1;
const PARAM_HIDDEN: i32 = 1 << 4;
const EVENT_NOTE_ON: u16 = 0;
const EVENT_NOTE_OFF: u16 = 1;
const AUDIO_CLASS: &str = "Audio Module Class";

/// Höchstzahl Parameteränderungen und Ereignisse je Block (vorab angelegt).
const MAX_QUEUES: usize = 128;
const MAX_POINTS: usize = 16;
const MAX_EVENTS: usize = 512;

fn utf16(s: &[u16]) -> String {
    let end = s.iter().position(|&c| c == 0).unwrap_or(s.len());
    String::from_utf16_lossy(&s[..end])
}

fn text(s: &[c_char]) -> String {
    let bytes: Vec<u8> = s
        .iter()
        .take_while(|&&c| c != 0)
        .map(|&c| c as u8)
        .collect();
    String::from_utf8_lossy(&bytes).into_owned()
}

fn hex(id: &Tuid) -> String {
    id.iter().map(|b| format!("{b:02X}")).collect()
}

fn unhex(s: &str) -> Option<Tuid> {
    if s.len() != 32 {
        return None;
    }
    let mut out = [0u8; 16];
    for (i, o) in out.iter_mut().enumerate() {
        *o = u8::from_str_radix(s.get(2 * i..2 * i + 2)?, 16).ok()?;
    }
    Some(out)
}

// ---------------------------------------------------------------------------
// Vtabellen (Reihenfolge der Methoden wie in der Formatbeschreibung).

#[repr(C)]
pub(crate) struct UnknownVt {
    pub(crate) query_interface: unsafe extern "system" fn(Obj, *const Tuid, *mut Obj) -> Tresult,
    pub(crate) add_ref: unsafe extern "system" fn(Obj) -> u32,
    pub(crate) release: unsafe extern "system" fn(Obj) -> u32,
}

#[repr(C)]
struct ClassInfo2 {
    cid: Tuid,
    cardinality: i32,
    category: [c_char; 32],
    name: [c_char; 64],
    class_flags: u32,
    sub_categories: [c_char; 128],
    vendor: [c_char; 64],
    version: [c_char; 64],
    sdk_version: [c_char; 64],
}

#[repr(C)]
struct ClassInfo {
    cid: Tuid,
    cardinality: i32,
    category: [c_char; 32],
    name: [c_char; 64],
}

#[repr(C)]
struct FactoryVt {
    unknown: UnknownVt,
    get_factory_info: unsafe extern "system" fn(Obj, *mut c_void) -> Tresult,
    count_classes: unsafe extern "system" fn(Obj) -> i32,
    get_class_info: unsafe extern "system" fn(Obj, i32, *mut ClassInfo) -> Tresult,
    create_instance: unsafe extern "system" fn(Obj, *const Tuid, *const Tuid, *mut Obj) -> Tresult,
    // Factory2
    get_class_info2: unsafe extern "system" fn(Obj, i32, *mut ClassInfo2) -> Tresult,
}

#[repr(C)]
struct PluginBaseVt {
    unknown: UnknownVt,
    initialize: unsafe extern "system" fn(Obj, Obj) -> Tresult,
    terminate: unsafe extern "system" fn(Obj) -> Tresult,
}

#[repr(C)]
struct BusInfo {
    media_type: i32,
    direction: i32,
    channel_count: i32,
    name: [u16; 128],
    bus_type: i32,
    flags: u32,
}

#[repr(C)]
struct ComponentVt {
    base: PluginBaseVt,
    get_controller_class_id: unsafe extern "system" fn(Obj, *mut Tuid) -> Tresult,
    set_io_mode: unsafe extern "system" fn(Obj, i32) -> Tresult,
    get_bus_count: unsafe extern "system" fn(Obj, i32, i32) -> i32,
    get_bus_info: unsafe extern "system" fn(Obj, i32, i32, i32, *mut BusInfo) -> Tresult,
    get_routing_info: unsafe extern "system" fn(Obj, *mut c_void, *mut c_void) -> Tresult,
    activate_bus: unsafe extern "system" fn(Obj, i32, i32, i32, u8) -> Tresult,
    set_active: unsafe extern "system" fn(Obj, u8) -> Tresult,
    set_state: unsafe extern "system" fn(Obj, Obj) -> Tresult,
    get_state: unsafe extern "system" fn(Obj, Obj) -> Tresult,
}

#[repr(C)]
struct ProcessSetup {
    process_mode: i32,
    symbolic_sample_size: i32,
    max_samples_per_block: i32,
    sample_rate: f64,
}

#[repr(C)]
struct AudioBusBuffers {
    num_channels: i32,
    silence_flags: u64,
    channel_buffers32: *mut *mut f32,
}

#[repr(C)]
struct ProcessData {
    process_mode: i32,
    symbolic_sample_size: i32,
    num_samples: i32,
    num_inputs: i32,
    num_outputs: i32,
    inputs: *mut AudioBusBuffers,
    outputs: *mut AudioBusBuffers,
    input_parameter_changes: Obj,
    output_parameter_changes: Obj,
    input_events: Obj,
    output_events: Obj,
    process_context: *mut c_void,
}

#[repr(C)]
struct AudioProcessorVt {
    unknown: UnknownVt,
    set_bus_arrangements: unsafe extern "system" fn(Obj, *mut u64, i32, *mut u64, i32) -> Tresult,
    get_bus_arrangement: unsafe extern "system" fn(Obj, i32, i32, *mut u64) -> Tresult,
    can_process_sample_size: unsafe extern "system" fn(Obj, i32) -> Tresult,
    get_latency_samples: unsafe extern "system" fn(Obj) -> u32,
    setup_processing: unsafe extern "system" fn(Obj, *mut ProcessSetup) -> Tresult,
    set_processing: unsafe extern "system" fn(Obj, u8) -> Tresult,
    process: unsafe extern "system" fn(Obj, *mut ProcessData) -> Tresult,
    get_tail_samples: unsafe extern "system" fn(Obj) -> u32,
}

#[repr(C)]
struct ParameterInfo {
    id: u32,
    title: [u16; 128],
    short_title: [u16; 128],
    units: [u16; 128],
    step_count: i32,
    default_normalized_value: f64,
    unit_id: i32,
    flags: i32,
}

#[repr(C)]
pub(crate) struct EditControllerVt {
    base: PluginBaseVt,
    set_component_state: unsafe extern "system" fn(Obj, Obj) -> Tresult,
    set_state: unsafe extern "system" fn(Obj, Obj) -> Tresult,
    get_state: unsafe extern "system" fn(Obj, Obj) -> Tresult,
    get_parameter_count: unsafe extern "system" fn(Obj) -> i32,
    get_parameter_info: unsafe extern "system" fn(Obj, i32, *mut ParameterInfo) -> Tresult,
    get_param_string_by_value: unsafe extern "system" fn(Obj, u32, f64, *mut [u16; 128]) -> Tresult,
    get_param_value_by_string: unsafe extern "system" fn(Obj, u32, *const u16, *mut f64) -> Tresult,
    normalized_param_to_plain: unsafe extern "system" fn(Obj, u32, f64) -> f64,
    plain_param_to_normalized: unsafe extern "system" fn(Obj, u32, f64) -> f64,
    get_param_normalized: unsafe extern "system" fn(Obj, u32) -> f64,
    set_param_normalized: unsafe extern "system" fn(Obj, u32, f64) -> Tresult,
    set_component_handler: unsafe extern "system" fn(Obj, Obj) -> Tresult,
    pub(crate) create_view: unsafe extern "system" fn(Obj, *const c_char) -> Obj,
}

#[repr(C)]
struct ConnectionPointVt {
    unknown: UnknownVt,
    connect: unsafe extern "system" fn(Obj, Obj) -> Tresult,
    disconnect: unsafe extern "system" fn(Obj, Obj) -> Tresult,
    notify: unsafe extern "system" fn(Obj, Obj) -> Tresult,
}

/// Vtabelle eines Objekts lesen.
///
/// # Safety
/// `obj` muss ein lebendes Objekt mit einer Vtabelle vom Typ `T` sein.
pub(crate) unsafe fn vt<'a, T>(obj: Obj) -> &'a T {
    // SAFETY: laut Vertrag zeigt das erste Feld des Objekts auf die Vtabelle.
    unsafe { &**(obj as *const *const T) }
}

/// Schnittstelle `iid` eines Objekts erfragen (`None`, wenn es sie nicht hat).
///
/// # Safety
/// `obj` muss ein lebendes Objekt sein.
pub(crate) unsafe fn query(obj: Obj, iid: &Tuid) -> Option<Obj> {
    let mut out: Obj = ptr::null_mut();
    // SAFETY: laut Vertrag; Ausgabezeiger gültig.
    let r = unsafe { (vt::<UnknownVt>(obj).query_interface)(obj, iid, &mut out) };
    (r == RESULT_OK && !out.is_null()).then_some(out)
}

/// # Safety
/// `obj` muss ein lebendes Objekt sein, dessen Referenz wir halten.
pub(crate) unsafe fn release(obj: Obj) {
    if !obj.is_null() {
        // SAFETY: laut Vertrag.
        unsafe { (vt::<UnknownVt>(obj).release)(obj) };
    }
}

// ---------------------------------------------------------------------------
// Host-Objekte. Sie gehören dem Host und leben länger als jede Nutzung durch das Plugin;
// addRef/release zählen deshalb nicht wirklich.

pub(crate) unsafe extern "system" fn host_add_ref(_: Obj) -> u32 {
    1
}

pub(crate) unsafe extern "system" fn host_release(_: Obj) -> u32 {
    1
}

/// Allgemeines `queryInterface` für Host-Objekte: das Objekt selbst für FUnknown und `own`.
///
/// # Safety
/// `iid` und `out` müssen gültig sein (Vertrag von queryInterface).
pub(crate) unsafe fn host_query(this: Obj, iid: *const Tuid, out: *mut Obj, own: &Tuid) -> Tresult {
    if iid.is_null() || out.is_null() {
        return -1;
    }
    // SAFETY: laut Vertrag gültige Zeiger.
    unsafe {
        if *iid == IID_UNKNOWN || *iid == *own {
            *out = this;
            return RESULT_OK;
        }
        *out = ptr::null_mut();
    }
    -1
}

#[repr(C)]
struct HostAppVt {
    unknown: UnknownVt,
    get_name: unsafe extern "system" fn(Obj, *mut [u16; 128]) -> Tresult,
    create_instance: unsafe extern "system" fn(Obj, *const Tuid, *const Tuid, *mut Obj) -> Tresult,
}

#[repr(C)]
struct HostApp {
    vt: *const HostAppVt,
}

unsafe extern "system" fn app_query(this: Obj, iid: *const Tuid, out: *mut Obj) -> Tresult {
    // SAFETY: Aufruf durch das Plugin mit gültigen Zeigern.
    unsafe { host_query(this, iid, out, &IID_HOST_APPLICATION) }
}

unsafe extern "system" fn app_get_name(_: Obj, name: *mut [u16; 128]) -> Tresult {
    if name.is_null() {
        return -1;
    }
    // SAFETY: `name` zeigt laut Vertrag auf 128 Zeichen.
    let out = unsafe { &mut *name };
    out.fill(0);
    for (o, c) in out.iter_mut().zip("StudioCraft".encode_utf16()) {
        *o = c;
    }
    RESULT_OK
}

unsafe extern "system" fn app_create_instance(
    _: Obj,
    cid: *const Tuid,
    iid: *const Tuid,
    out: *mut Obj,
) -> Tresult {
    if out.is_null() {
        return -1;
    }
    // SAFETY: Zeiger laut Vertrag gültig oder null.
    unsafe {
        *out = ptr::null_mut();
        let wants = |id: &Tuid| (!cid.is_null() && *cid == *id) || (!iid.is_null() && *iid == *id);
        if wants(&IID_MESSAGE) {
            *out = Message::create();
            return RESULT_OK;
        }
        if wants(&IID_ATTRIBUTE_LIST) {
            *out = AttrList::create();
            return RESULT_OK;
        }
    }
    -1
}

// Nachrichten zwischen Controller und Komponente (manche Plugins verlangen sie vom Host).
// Echte Referenzzählung: das Plugin gibt sie frei, wenn es sie nicht mehr braucht.

const IID_MESSAGE: Tuid = uid(0x936F_033B, 0xC6C0_47DB, 0xBB08_82F8, 0x13C1_E613);
const IID_ATTRIBUTE_LIST: Tuid = uid(0x1E5F_0AEB, 0xCC7F_4533, 0xA254_4011, 0x38AD_5EE4);

#[repr(C)]
struct MessageVt {
    unknown: UnknownVt,
    get_message_id: unsafe extern "system" fn(Obj) -> *const c_char,
    set_message_id: unsafe extern "system" fn(Obj, *const c_char),
    get_attributes: unsafe extern "system" fn(Obj) -> Obj,
}

#[repr(C)]
struct Message {
    vt: *const MessageVt,
    refs: AtomicU32,
    id: std::ffi::CString,
    attrs: Obj,
}

impl Message {
    fn create() -> Obj {
        Box::into_raw(Box::new(Self {
            vt: &MESSAGE_VT,
            refs: AtomicU32::new(1),
            id: std::ffi::CString::default(),
            attrs: AttrList::create(),
        }))
        .cast()
    }
}

unsafe extern "system" fn msg_query(this: Obj, iid: *const Tuid, out: *mut Obj) -> Tresult {
    // SAFETY: Aufruf durch das Plugin mit gültigen Zeigern; Treffer zählen als neue Referenz.
    let r = unsafe { host_query(this, iid, out, &IID_MESSAGE) };
    if r == RESULT_OK {
        // SAFETY: `this` ist eine unserer Nachrichten.
        unsafe { msg_add_ref(this) };
    }
    r
}

unsafe extern "system" fn msg_add_ref(this: Obj) -> u32 {
    // SAFETY: `this` ist eine unserer Nachrichten.
    unsafe {
        (*(this as *const Message))
            .refs
            .fetch_add(1, Ordering::AcqRel)
            + 1
    }
}

unsafe extern "system" fn msg_release(this: Obj) -> u32 {
    // SAFETY: `this` ist eine unserer Nachrichten; bei 0 gibt es keine Nutzer mehr.
    unsafe {
        let n = (*(this as *const Message))
            .refs
            .fetch_sub(1, Ordering::AcqRel)
            - 1;
        if n == 0 {
            let m = Box::from_raw(this as *mut Message);
            release(m.attrs);
        }
        n
    }
}

unsafe extern "system" fn msg_get_id(this: Obj) -> *const c_char {
    // SAFETY: `this` ist eine unserer Nachrichten; die Kennung lebt mit ihr.
    unsafe { (*(this as *const Message)).id.as_ptr() }
}

unsafe extern "system" fn msg_set_id(this: Obj, id: *const c_char) {
    // SAFETY: wie oben; `id` nullterminiert oder null.
    unsafe {
        let m = &mut *(this as *mut Message);
        m.id = if id.is_null() {
            std::ffi::CString::default()
        } else {
            std::ffi::CStr::from_ptr(id).to_owned()
        };
    }
}

unsafe extern "system" fn msg_attributes(this: Obj) -> Obj {
    // SAFETY: wie oben. Laut Formatbeschreibung ohne zusätzliche Referenz.
    unsafe { (*(this as *const Message)).attrs }
}

static MESSAGE_VT: MessageVt = MessageVt {
    unknown: UnknownVt {
        query_interface: msg_query,
        add_ref: msg_add_ref,
        release: msg_release,
    },
    get_message_id: msg_get_id,
    set_message_id: msg_set_id,
    get_attributes: msg_attributes,
};

#[repr(C)]
struct AttrListVt {
    unknown: UnknownVt,
    set_int: unsafe extern "system" fn(Obj, *const c_char, i64) -> Tresult,
    get_int: unsafe extern "system" fn(Obj, *const c_char, *mut i64) -> Tresult,
    set_float: unsafe extern "system" fn(Obj, *const c_char, f64) -> Tresult,
    get_float: unsafe extern "system" fn(Obj, *const c_char, *mut f64) -> Tresult,
    set_string: unsafe extern "system" fn(Obj, *const c_char, *const u16) -> Tresult,
    get_string: unsafe extern "system" fn(Obj, *const c_char, *mut u16, u32) -> Tresult,
    set_binary: unsafe extern "system" fn(Obj, *const c_char, *const c_void, u32) -> Tresult,
    get_binary:
        unsafe extern "system" fn(Obj, *const c_char, *mut *const c_void, *mut u32) -> Tresult,
}

enum Attr {
    Int(i64),
    Float(f64),
    Str(Vec<u16>),
    Bin(Vec<u8>),
}

#[repr(C)]
struct AttrList {
    vt: *const AttrListVt,
    refs: AtomicU32,
    map: Mutex<HashMap<String, Attr>>,
}

impl AttrList {
    fn create() -> Obj {
        Box::into_raw(Box::new(Self {
            vt: &ATTR_VT,
            refs: AtomicU32::new(1),
            map: Mutex::new(HashMap::new()),
        }))
        .cast()
    }
}

/// # Safety
/// `this` muss eine unserer Attributlisten sein, `key` nullterminiert oder null.
unsafe fn attr_access<R>(
    this: Obj,
    key: *const c_char,
    f: impl FnOnce(&mut HashMap<String, Attr>, String) -> R,
) -> Option<R> {
    if this.is_null() || key.is_null() {
        return None;
    }
    // SAFETY: laut Vertrag.
    let (list, key) = unsafe {
        (
            &*(this as *const AttrList),
            std::ffi::CStr::from_ptr(key).to_string_lossy().into_owned(),
        )
    };
    let mut map = list.map.lock().ok()?;
    Some(f(&mut map, key))
}

unsafe extern "system" fn attr_query(this: Obj, iid: *const Tuid, out: *mut Obj) -> Tresult {
    // SAFETY: Aufruf durch das Plugin mit gültigen Zeigern; Treffer zählen als neue Referenz.
    let r = unsafe { host_query(this, iid, out, &IID_ATTRIBUTE_LIST) };
    if r == RESULT_OK {
        // SAFETY: `this` ist eine unserer Listen.
        unsafe { attr_add_ref(this) };
    }
    r
}

unsafe extern "system" fn attr_add_ref(this: Obj) -> u32 {
    // SAFETY: `this` ist eine unserer Listen.
    unsafe {
        (*(this as *const AttrList))
            .refs
            .fetch_add(1, Ordering::AcqRel)
            + 1
    }
}

unsafe extern "system" fn attr_release(this: Obj) -> u32 {
    // SAFETY: `this` ist eine unserer Listen; bei 0 gibt es keine Nutzer mehr.
    unsafe {
        let n = (*(this as *const AttrList))
            .refs
            .fetch_sub(1, Ordering::AcqRel)
            - 1;
        if n == 0 {
            drop(Box::from_raw(this as *mut AttrList));
        }
        n
    }
}

unsafe extern "system" fn attr_set_int(this: Obj, key: *const c_char, v: i64) -> Tresult {
    // SAFETY: Aufruf durch das Plugin.
    unsafe { attr_access(this, key, |m, k| m.insert(k, Attr::Int(v))) }.map_or(-1, |_| RESULT_OK)
}

unsafe extern "system" fn attr_get_int(this: Obj, key: *const c_char, out: *mut i64) -> Tresult {
    // SAFETY: Aufruf durch das Plugin; `out` gültig oder null.
    unsafe {
        match attr_access(this, key, |m, k| match m.get(&k) {
            Some(Attr::Int(v)) => Some(*v),
            _ => None,
        })
        .flatten()
        {
            Some(v) if !out.is_null() => {
                *out = v;
                RESULT_OK
            }
            _ => -1,
        }
    }
}

unsafe extern "system" fn attr_set_float(this: Obj, key: *const c_char, v: f64) -> Tresult {
    // SAFETY: Aufruf durch das Plugin.
    unsafe { attr_access(this, key, |m, k| m.insert(k, Attr::Float(v))) }.map_or(-1, |_| RESULT_OK)
}

unsafe extern "system" fn attr_get_float(this: Obj, key: *const c_char, out: *mut f64) -> Tresult {
    // SAFETY: Aufruf durch das Plugin; `out` gültig oder null.
    unsafe {
        match attr_access(this, key, |m, k| match m.get(&k) {
            Some(Attr::Float(v)) => Some(*v),
            _ => None,
        })
        .flatten()
        {
            Some(v) if !out.is_null() => {
                *out = v;
                RESULT_OK
            }
            _ => -1,
        }
    }
}

unsafe extern "system" fn attr_set_string(this: Obj, key: *const c_char, s: *const u16) -> Tresult {
    if s.is_null() {
        return -1;
    }
    // SAFETY: `s` ist laut Vertrag nullterminiert; höchstens 64 Ki Zeichen gelesen.
    let text: Vec<u16> = unsafe {
        let mut v = Vec::new();
        let mut i = 0;
        while i < 65_536 && *s.add(i) != 0 {
            v.push(*s.add(i));
            i += 1;
        }
        v
    };
    // SAFETY: Aufruf durch das Plugin.
    unsafe { attr_access(this, key, |m, k| m.insert(k, Attr::Str(text))) }.map_or(-1, |_| RESULT_OK)
}

unsafe extern "system" fn attr_get_string(
    this: Obj,
    key: *const c_char,
    out: *mut u16,
    size_bytes: u32,
) -> Tresult {
    let cap = (size_bytes / 2) as usize;
    if out.is_null() || cap == 0 {
        return -1;
    }
    // SAFETY: `out` hat laut Vertrag `size_bytes` Bytes Platz.
    unsafe {
        attr_access(this, key, |m, k| match m.get(&k) {
            Some(Attr::Str(t)) => {
                let n = t.len().min(cap - 1);
                ptr::copy_nonoverlapping(t.as_ptr(), out, n);
                *out.add(n) = 0;
                RESULT_OK
            }
            _ => -1,
        })
        .unwrap_or(-1)
    }
}

unsafe extern "system" fn attr_set_binary(
    this: Obj,
    key: *const c_char,
    data: *const c_void,
    size: u32,
) -> Tresult {
    if (data.is_null() && size > 0) || size as usize > MAX_STATE {
        return -1;
    }
    // SAFETY: `data` hat laut Vertrag `size` Bytes.
    let bytes = unsafe {
        if size == 0 {
            Vec::new()
        } else {
            std::slice::from_raw_parts(data.cast::<u8>(), size as usize).to_vec()
        }
    };
    // SAFETY: Aufruf durch das Plugin.
    unsafe { attr_access(this, key, |m, k| m.insert(k, Attr::Bin(bytes))) }
        .map_or(-1, |_| RESULT_OK)
}

unsafe extern "system" fn attr_get_binary(
    this: Obj,
    key: *const c_char,
    data: *mut *const c_void,
    size: *mut u32,
) -> Tresult {
    if data.is_null() || size.is_null() {
        return -1;
    }
    // SAFETY: Ausgabezeiger gültig; der Zeiger zeigt in die Liste und lebt mit ihr.
    unsafe {
        attr_access(this, key, |m, k| match m.get(&k) {
            Some(Attr::Bin(b)) => {
                *data = b.as_ptr().cast();
                *size = b.len() as u32;
                RESULT_OK
            }
            _ => -1,
        })
        .unwrap_or(-1)
    }
}

static ATTR_VT: AttrListVt = AttrListVt {
    unknown: UnknownVt {
        query_interface: attr_query,
        add_ref: attr_add_ref,
        release: attr_release,
    },
    set_int: attr_set_int,
    get_int: attr_get_int,
    set_float: attr_set_float,
    get_float: attr_get_float,
    set_string: attr_set_string,
    get_string: attr_get_string,
    set_binary: attr_set_binary,
    get_binary: attr_get_binary,
};

static HOST_APP_VT: HostAppVt = HostAppVt {
    unknown: UnknownVt {
        query_interface: app_query,
        add_ref: host_add_ref,
        release: host_release,
    },
    get_name: app_get_name,
    create_instance: app_create_instance,
};

struct SyncPtr<T>(T);
// SAFETY: Die Host-Anwendung ist unveränderlich (nur eine Vtabelle); Plugins dürfen sie aus
// jedem Thread benutzen.
unsafe impl<T> Sync for SyncPtr<T> {}
unsafe impl<T> Send for SyncPtr<T> {}

fn host_app() -> Obj {
    static APP: OnceLock<SyncPtr<Box<HostApp>>> = OnceLock::new();
    let app = APP.get_or_init(|| SyncPtr(Box::new(HostApp { vt: &HOST_APP_VT })));
    (&*app.0 as *const HostApp).cast_mut().cast()
}

#[repr(C)]
struct ComponentHandlerVt {
    unknown: UnknownVt,
    begin_edit: unsafe extern "system" fn(Obj, u32) -> Tresult,
    perform_edit: unsafe extern "system" fn(Obj, u32, f64) -> Tresult,
    end_edit: unsafe extern "system" fn(Obj, u32) -> Tresult,
    restart_component: unsafe extern "system" fn(Obj, i32) -> Tresult,
}

/// Rückmeldungen des Controllers (Latenz geändert, Bearbeitungen im Plugin).
#[repr(C)]
struct ComponentHandler {
    vt: *const ComponentHandlerVt,
    shared: Arc<Shared>,
}

unsafe extern "system" fn handler_query(this: Obj, iid: *const Tuid, out: *mut Obj) -> Tresult {
    // SAFETY: Aufruf durch das Plugin mit gültigen Zeigern.
    unsafe { host_query(this, iid, out, &IID_COMPONENT_HANDLER) }
}

unsafe extern "system" fn handler_edit(_: Obj, _: u32) -> Tresult {
    RESULT_OK
}

unsafe extern "system" fn handler_perform(this: Obj, id: u32, value: f64) -> Tresult {
    if this.is_null() || !value.is_finite() {
        return -1;
    }
    // SAFETY: `this` ist unser ComponentHandler (lebt mit der Instanz).
    let h = unsafe { &*(this as *const ComponentHandler) };
    // Manche Plugins melden auch aus `process` (Audio-Thread): ohne Lock und ohne Allokation,
    // nur der letzte Wert je Parameter (Ziehen erzeugt viele Schritte).
    h.shared.edits.record(id, value.clamp(0.0, 1.0));
    RESULT_OK
}

unsafe extern "system" fn handler_restart(this: Obj, flags: i32) -> Tresult {
    // kLatencyChanged = 1 << 3
    if flags & (1 << 3) != 0 && !this.is_null() {
        // SAFETY: `this` ist unser ComponentHandler (lebt mit der Instanz).
        let h = unsafe { &*(this as *const ComponentHandler) };
        h.shared.latency_dirty.store(1, Ordering::Release);
    }
    RESULT_OK
}

static HANDLER_VT: ComponentHandlerVt = ComponentHandlerVt {
    unknown: UnknownVt {
        query_interface: handler_query,
        add_ref: host_add_ref,
        release: host_release,
    },
    begin_edit: handler_edit,
    perform_edit: handler_perform,
    end_edit: handler_edit,
    restart_component: handler_restart,
};

// Parameteränderungen für einen Block: feste Anzahl Warteschlangen mit festen Punkten.

#[repr(C)]
struct QueueVt {
    unknown: UnknownVt,
    get_parameter_id: unsafe extern "system" fn(Obj) -> u32,
    get_point_count: unsafe extern "system" fn(Obj) -> i32,
    get_point: unsafe extern "system" fn(Obj, i32, *mut i32, *mut f64) -> Tresult,
    add_point: unsafe extern "system" fn(Obj, i32, f64, *mut i32) -> Tresult,
}

#[repr(C)]
struct Queue {
    vt: *const QueueVt,
    id: u32,
    len: usize,
    points: [(i32, f64); MAX_POINTS],
}

unsafe extern "system" fn queue_query(this: Obj, iid: *const Tuid, out: *mut Obj) -> Tresult {
    // SAFETY: Aufruf durch das Plugin mit gültigen Zeigern.
    unsafe { host_query(this, iid, out, &IID_PARAM_VALUE_QUEUE) }
}

unsafe extern "system" fn queue_id(this: Obj) -> u32 {
    // SAFETY: `this` ist eine unserer Warteschlangen.
    unsafe { (*(this as *const Queue)).id }
}

unsafe extern "system" fn queue_count(this: Obj) -> i32 {
    // SAFETY: wie oben.
    unsafe { (*(this as *const Queue)).len as i32 }
}

unsafe extern "system" fn queue_point(this: Obj, i: i32, offset: *mut i32, v: *mut f64) -> Tresult {
    // SAFETY: wie oben; Ausgabezeiger laut Vertrag gültig.
    unsafe {
        let q = &*(this as *const Queue);
        let Some(&(o, val)) = usize::try_from(i)
            .ok()
            .filter(|&i| i < q.len)
            .and_then(|i| q.points.get(i))
        else {
            return -1;
        };
        if !offset.is_null() {
            *offset = o;
        }
        if !v.is_null() {
            *v = val;
        }
    }
    RESULT_OK
}

unsafe extern "system" fn queue_add(this: Obj, offset: i32, v: f64, index: *mut i32) -> Tresult {
    // SAFETY: `this` ist eine unserer Warteschlangen; `index` laut Vertrag gültig oder null.
    unsafe {
        let q = &mut *(this as *mut Queue);
        if q.len >= MAX_POINTS {
            return -1;
        }
        q.points[q.len] = (offset, if v.is_finite() { v } else { 0.0 });
        if !index.is_null() {
            *index = q.len as i32;
        }
        q.len += 1;
    }
    RESULT_OK
}

static QUEUE_VT: QueueVt = QueueVt {
    unknown: UnknownVt {
        query_interface: queue_query,
        add_ref: host_add_ref,
        release: host_release,
    },
    get_parameter_id: queue_id,
    get_point_count: queue_count,
    get_point: queue_point,
    add_point: queue_add,
};

#[repr(C)]
struct ChangesVt {
    unknown: UnknownVt,
    get_parameter_count: unsafe extern "system" fn(Obj) -> i32,
    get_parameter_data: unsafe extern "system" fn(Obj, i32) -> Obj,
    add_parameter_data: unsafe extern "system" fn(Obj, *const u32, *mut i32) -> Obj,
}

#[repr(C)]
struct Changes {
    vt: *const ChangesVt,
    used: usize,
    queues: Box<[Queue]>,
}

unsafe extern "system" fn changes_query(this: Obj, iid: *const Tuid, out: *mut Obj) -> Tresult {
    // SAFETY: Aufruf durch das Plugin mit gültigen Zeigern.
    unsafe { host_query(this, iid, out, &IID_PARAMETER_CHANGES) }
}

unsafe extern "system" fn changes_count(this: Obj) -> i32 {
    // SAFETY: `this` ist unsere Änderungsliste.
    unsafe { (*(this as *const Changes)).used as i32 }
}

unsafe extern "system" fn changes_data(this: Obj, i: i32) -> Obj {
    // SAFETY: wie oben.
    unsafe {
        let c = &mut *(this as *mut Changes);
        match usize::try_from(i).ok().filter(|&i| i < c.used) {
            Some(i) => (&mut c.queues[i] as *mut Queue).cast(),
            None => ptr::null_mut(),
        }
    }
}

unsafe extern "system" fn changes_add(this: Obj, id: *const u32, index: *mut i32) -> Obj {
    if id.is_null() {
        return ptr::null_mut();
    }
    // SAFETY: `this` ist unsere Änderungsliste; `id` gültig, `index` gültig oder null.
    unsafe {
        let c = &mut *(this as *mut Changes);
        let id = *id;
        let k = match c.queues[..c.used].iter().position(|q| q.id == id) {
            Some(k) => k,
            None if c.used < c.queues.len() => {
                c.queues[c.used].id = id;
                c.queues[c.used].len = 0;
                c.used += 1;
                c.used - 1
            }
            None => return ptr::null_mut(),
        };
        if !index.is_null() {
            *index = k as i32;
        }
        (&mut c.queues[k] as *mut Queue).cast()
    }
}

static CHANGES_VT: ChangesVt = ChangesVt {
    unknown: UnknownVt {
        query_interface: changes_query,
        add_ref: host_add_ref,
        release: host_release,
    },
    get_parameter_count: changes_count,
    get_parameter_data: changes_data,
    add_parameter_data: changes_add,
};

impl Changes {
    fn new() -> Box<Self> {
        Box::new(Self {
            vt: &CHANGES_VT,
            used: 0,
            queues: (0..MAX_QUEUES)
                .map(|_| Queue {
                    vt: &QUEUE_VT,
                    id: 0,
                    len: 0,
                    points: [(0, 0.0); MAX_POINTS],
                })
                .collect(),
        })
    }

    /// Änderung an Frame `offset` aufnehmen (keine Allokation; voll = verworfen).
    fn push(&mut self, id: u32, offset: i32, value: f64) {
        let used = self.used;
        let k = match self.queues[..used].iter().position(|q| q.id == id) {
            Some(k) => k,
            None if used < self.queues.len() => {
                self.queues[used].id = id;
                self.queues[used].len = 0;
                self.used += 1;
                used
            }
            None => return,
        };
        let q = &mut self.queues[k];
        if q.len < MAX_POINTS {
            q.points[q.len] = (offset, value.clamp(0.0, 1.0));
            q.len += 1;
        } else if let Some(last) = q.points.last_mut() {
            *last = (offset, value.clamp(0.0, 1.0));
        }
    }

    fn clear(&mut self) {
        self.used = 0;
    }
}

// Ereignisse (Noten) für einen Block.

#[repr(C)]
#[derive(Clone, Copy)]
struct NoteOn {
    channel: i16,
    pitch: i16,
    tuning: f32,
    velocity: f32,
    length: i32,
    note_id: i32,
}

#[repr(C)]
#[derive(Clone, Copy)]
struct NoteOff {
    channel: i16,
    pitch: i16,
    velocity: f32,
    note_id: i32,
    tuning: f32,
}

#[repr(C)]
#[derive(Clone, Copy)]
union EventBody {
    on: NoteOn,
    off: NoteOff,
    /// Größe der größten Ereignisart (Platz für alles, was ein Plugin erwartet).
    raw: [u64; 3],
}

#[repr(C)]
#[derive(Clone, Copy)]
struct Event {
    bus_index: i32,
    sample_offset: i32,
    ppq_position: f64,
    flags: u16,
    kind: u16,
    body: EventBody,
}

#[repr(C)]
struct EventListVt {
    unknown: UnknownVt,
    get_event_count: unsafe extern "system" fn(Obj) -> i32,
    get_event: unsafe extern "system" fn(Obj, i32, *mut Event) -> Tresult,
    add_event: unsafe extern "system" fn(Obj, *mut Event) -> Tresult,
}

#[repr(C)]
struct EventList {
    vt: *const EventListVt,
    len: usize,
    events: Box<[Event]>,
}

unsafe extern "system" fn events_query(this: Obj, iid: *const Tuid, out: *mut Obj) -> Tresult {
    // SAFETY: Aufruf durch das Plugin mit gültigen Zeigern.
    unsafe { host_query(this, iid, out, &IID_EVENT_LIST) }
}

unsafe extern "system" fn events_count(this: Obj) -> i32 {
    // SAFETY: `this` ist unsere Ereignisliste.
    unsafe { (*(this as *const EventList)).len as i32 }
}

unsafe extern "system" fn events_get(this: Obj, i: i32, out: *mut Event) -> Tresult {
    // SAFETY: wie oben; `out` laut Vertrag gültig.
    unsafe {
        let l = &*(this as *const EventList);
        match usize::try_from(i).ok().filter(|&i| i < l.len) {
            Some(i) if !out.is_null() => {
                *out = l.events[i];
                RESULT_OK
            }
            _ => -1,
        }
    }
}

unsafe extern "system" fn events_add(_: Obj, _: *mut Event) -> Tresult {
    -1
}

static EVENTS_VT: EventListVt = EventListVt {
    unknown: UnknownVt {
        query_interface: events_query,
        add_ref: host_add_ref,
        release: host_release,
    },
    get_event_count: events_count,
    get_event: events_get,
    add_event: events_add,
};

impl EventList {
    fn new() -> Box<Self> {
        let empty = Event {
            bus_index: 0,
            sample_offset: 0,
            ppq_position: 0.0,
            flags: 0,
            kind: 0,
            body: EventBody { raw: [0; 3] },
        };
        Box::new(Self {
            vt: &EVENTS_VT,
            len: 0,
            events: vec![empty; MAX_EVENTS].into_boxed_slice(),
        })
    }

    fn push(&mut self, e: Event) {
        if let Some(slot) = self.events.get_mut(self.len) {
            *slot = e;
            self.len += 1;
        }
    }
}

// Datenstrom für Zustände: liest aus bzw. schreibt in einen Puffer (Hauptthread).

#[repr(C)]
struct StreamVt {
    unknown: UnknownVt,
    read: unsafe extern "system" fn(Obj, *mut c_void, i32, *mut i32) -> Tresult,
    write: unsafe extern "system" fn(Obj, *const c_void, i32, *mut i32) -> Tresult,
    seek: unsafe extern "system" fn(Obj, i64, i32, *mut i64) -> Tresult,
    tell: unsafe extern "system" fn(Obj, *mut i64) -> Tresult,
}

#[repr(C)]
struct Stream {
    vt: *const StreamVt,
    data: Vec<u8>,
    pos: usize,
}

unsafe extern "system" fn stream_query(this: Obj, iid: *const Tuid, out: *mut Obj) -> Tresult {
    // SAFETY: Aufruf durch das Plugin mit gültigen Zeigern.
    unsafe { host_query(this, iid, out, &IID_STREAM) }
}

unsafe extern "system" fn stream_read(
    this: Obj,
    buf: *mut c_void,
    n: i32,
    read: *mut i32,
) -> Tresult {
    // SAFETY: `this` ist unser Strom; `buf` hat laut Vertrag `n` Bytes Platz.
    unsafe {
        let s = &mut *(this as *mut Stream);
        let want = usize::try_from(n).unwrap_or(0);
        let k = want.min(s.data.len().saturating_sub(s.pos));
        if k > 0 && !buf.is_null() {
            ptr::copy_nonoverlapping(s.data.as_ptr().add(s.pos), buf.cast::<u8>(), k);
        }
        s.pos += k;
        if !read.is_null() {
            *read = k as i32;
        }
    }
    RESULT_OK
}

unsafe extern "system" fn stream_write(
    this: Obj,
    buf: *const c_void,
    n: i32,
    written: *mut i32,
) -> Tresult {
    // SAFETY: `this` ist unser Strom; `buf` hat laut Vertrag `n` Bytes.
    unsafe {
        let s = &mut *(this as *mut Stream);
        let n = usize::try_from(n).unwrap_or(0);
        if buf.is_null() || s.pos.saturating_add(n) > MAX_STATE {
            return -1;
        }
        let src = std::slice::from_raw_parts(buf.cast::<u8>(), n);
        let end = s.pos + n;
        if s.data.len() < end {
            s.data.resize(end, 0);
        }
        s.data[s.pos..end].copy_from_slice(src);
        s.pos = end;
        if !written.is_null() {
            *written = n as i32;
        }
    }
    RESULT_OK
}

unsafe extern "system" fn stream_seek(this: Obj, pos: i64, mode: i32, result: *mut i64) -> Tresult {
    // SAFETY: `this` ist unser Strom; `result` gültig oder null.
    unsafe {
        let s = &mut *(this as *mut Stream);
        let base = match mode {
            0 => 0i64,
            1 => s.pos as i64,
            2 => s.data.len() as i64,
            _ => return -1,
        };
        let Some(new) = base
            .checked_add(pos)
            .filter(|&p| (0..=MAX_STATE as i64).contains(&p))
        else {
            return -1;
        };
        s.pos = new as usize;
        if !result.is_null() {
            *result = new;
        }
    }
    RESULT_OK
}

unsafe extern "system" fn stream_tell(this: Obj, pos: *mut i64) -> Tresult {
    if pos.is_null() {
        return -1;
    }
    // SAFETY: `this` ist unser Strom; `pos` gültig.
    unsafe { *pos = (*(this as *const Stream)).pos as i64 };
    RESULT_OK
}

static STREAM_VT: StreamVt = StreamVt {
    unknown: UnknownVt {
        query_interface: stream_query,
        add_ref: host_add_ref,
        release: host_release,
    },
    read: stream_read,
    write: stream_write,
    seek: stream_seek,
    tell: stream_tell,
};

impl Stream {
    fn new(data: Vec<u8>) -> Box<Self> {
        Box::new(Self {
            vt: &STREAM_VT,
            data,
            pos: 0,
        })
    }

    fn obj(&mut self) -> Obj {
        (self as *mut Self).cast()
    }
}

/// Zustand in einen Container: Kennung, dann Länge + Bytes von Komponente und Controller.
fn pack_state(component: &[u8], controller: &[u8]) -> Vec<u8> {
    let mut out = Vec::with_capacity(12 + component.len() + controller.len());
    out.extend_from_slice(STATE_MAGIC);
    for part in [component, controller] {
        out.extend_from_slice(&(part.len() as u32).to_le_bytes());
        out.extend_from_slice(part);
    }
    out
}

/// Gegenstück zu [`pack_state`]; `None` bei fremden oder kaputten Daten.
fn unpack_state(data: &[u8]) -> Option<(&[u8], &[u8])> {
    let rest = data.strip_prefix(STATE_MAGIC.as_slice())?;
    let take = |d: &[u8]| -> Option<(usize, usize)> {
        let len = u32::from_le_bytes(d.get(..4)?.try_into().ok()?) as usize;
        (len <= MAX_STATE && d.len() >= 4 + len).then_some((4, len))
    };
    let (o, a) = take(rest)?;
    let component = &rest[o..o + a];
    let rest = &rest[o + a..];
    let (o, b) = take(rest)?;
    let controller = &rest[o..o + b];
    (rest.len() == o + b).then_some((component, controller))
}

// ---------------------------------------------------------------------------
// Module (Bibliotheken) und Fabriken. Module bleiben bis zum Programmende geladen.

struct Module {
    factory: Obj,
}

// SAFETY: Die Fabrik ist laut Formatbeschreibung aus dem Hauptthread zu benutzen; alle Zugriffe
// laufen unter `main_thread_lock`.
unsafe impl Send for Module {}

/// Bibliothek innerhalb eines `.vst3`-Bundles für dieses System.
fn binary_path(bundle: &Path) -> Option<PathBuf> {
    let stem = bundle.file_stem()?.to_owned();
    if bundle.is_file() {
        return Some(bundle.to_path_buf());
    }
    let arch = std::env::consts::ARCH;
    let dir = if cfg!(windows) {
        let a = if arch == "aarch64" { "arm64" } else { arch };
        format!("{a}-win")
    } else if cfg!(target_os = "macos") {
        return None;
    } else {
        format!("{arch}-linux")
    };
    let mut name = stem;
    name.push(if cfg!(windows) { ".vst3" } else { ".so" });
    Some(bundle.join("Contents").join(dir).join(name))
}

fn module(bundle: &Path) -> Result<Obj, String> {
    static MODULES: OnceLock<Mutex<HashMap<PathBuf, Module>>> = OnceLock::new();
    let mut mods = MODULES
        .get_or_init(Default::default)
        .lock()
        .map_err(|_| "VST3-Module gesperrt".to_string())?;
    if let Some(m) = mods.get(bundle) {
        return Ok(m.factory);
    }
    let path = binary_path(bundle).ok_or("VST3 auf diesem System noch nicht unterstützt")?;
    // SAFETY: Laden fremden Codes ist der Zweck; Initialisierer der Bibliothek laufen hier.
    let lib = unsafe { libloading::Library::new(&path) }.map_err(|e| e.to_string())?;
    // Einstieg laut Formatbeschreibung: Linux `ModuleEntry(handle)`, Windows `InitDll()`.
    #[cfg(unix)]
    {
        // SAFETY: Signatur laut Formatbeschreibung; der Griff ist der der geöffneten Bibliothek.
        let entry: Option<libloading::Symbol<unsafe extern "C" fn(*mut c_void) -> bool>> =
            unsafe { lib.get(b"ModuleEntry\0") }.ok();
        if let Some(entry) = entry {
            // SAFETY: wie oben. Ein Griff ist nicht nötig; Plugins nutzen ihn nur für Pfade.
            if !unsafe { entry(ptr::null_mut()) } {
                return Err("ModuleEntry fehlgeschlagen".into());
            }
        }
    }
    #[cfg(windows)]
    {
        // SAFETY: Signatur laut Formatbeschreibung.
        let entry: Option<libloading::Symbol<unsafe extern "system" fn() -> bool>> =
            unsafe { lib.get(b"InitDll\0") }.ok();
        if let Some(entry) = entry {
            // SAFETY: wie oben.
            if !unsafe { entry() } {
                return Err("InitDll fehlgeschlagen".into());
            }
        }
    }
    // SAFETY: `GetPluginFactory` liefert laut Formatbeschreibung eine Fabrik (mit Referenz).
    let factory = unsafe {
        let get: libloading::Symbol<unsafe extern "system" fn() -> Obj> = lib
            .get(b"GetPluginFactory\0")
            .map_err(|_| "kein GetPluginFactory – keine VST3-Datei".to_string())?;
        get()
    };
    if factory.is_null() {
        return Err("VST3-Fabrik ist leer".into());
    }
    // Bibliothek nie entladen (Plugins mit eigenen Threads stürzen sonst ab).
    std::mem::forget(lib);
    mods.insert(bundle.to_path_buf(), Module { factory });
    Ok(factory)
}

/// Klassen eines Bundles als Plugin-Beschreibungen (nur Audio-Klassen).
pub(crate) fn descriptors(bundle: &Path) -> Result<Vec<PluginInfo>, String> {
    let _main = main_thread_lock();
    let f = module(bundle)?;
    let mut out = Vec::new();
    // SAFETY: Fabrik lebt (Modul bleibt geladen); Hauptthread; Strukturen in voller Größe.
    unsafe {
        let fvt = vt::<FactoryVt>(f);
        let f2 = query(f, &IID_FACTORY2);
        let n = (fvt.count_classes)(f);
        for i in 0..n.clamp(0, 1024) {
            let mut c2: ClassInfo2 = std::mem::zeroed();
            let have2 = f2.is_some_and(|f2| {
                (vt::<FactoryVt>(f2).get_class_info2)(f2, i, &mut c2) == RESULT_OK
            });
            let (cid, category, name, sub, vendor, version) = if have2 {
                (
                    c2.cid,
                    text(&c2.category),
                    text(&c2.name),
                    text(&c2.sub_categories),
                    text(&c2.vendor),
                    text(&c2.version),
                )
            } else {
                let mut c: ClassInfo = std::mem::zeroed();
                if (fvt.get_class_info)(f, i, &mut c) != RESULT_OK {
                    continue;
                }
                (
                    c.cid,
                    text(&c.category),
                    text(&c.name),
                    String::new(),
                    String::new(),
                    String::new(),
                )
            };
            if category != AUDIO_CLASS {
                continue;
            }
            out.push(PluginInfo {
                format: PluginFormat::Vst3,
                id: hex(&cid),
                name,
                vendor,
                version,
                features: vec![if sub.split('|').any(|s| s == "Instrument") {
                    "instrument".into()
                } else {
                    "audio-effect".into()
                }],
                path: bundle.to_path_buf(),
            });
        }
        if let Some(f2) = f2 {
            release(f2);
        }
    }
    Ok(out)
}

// ---------------------------------------------------------------------------
// Instanz.

#[derive(Default)]
struct Shared {
    latency: AtomicU32,
    /// 1, wenn das Plugin eine geänderte Latenz gemeldet hat (noch abzuholen).
    latency_dirty: AtomicU32,
    /// Bearbeitungen im Plugin (Id, normierter Wert), noch abzuholen; aus jedem Thread.
    edits: crate::edits::Edits,
}

/// Puffer eines Audio-Busses: Kanäle und die Zeigertabelle, auf die das Plugin schaut.
struct Bus {
    chans: Vec<Box<[f32]>>,
    ptrs: Box<[*mut f32]>,
}

impl Bus {
    fn new(channels: usize, frames: usize) -> Self {
        let mut chans: Vec<Box<[f32]>> = (0..channels)
            .map(|_| vec![0.0; frames].into_boxed_slice())
            .collect();
        let ptrs = chans.iter_mut().map(|c| c.as_mut_ptr()).collect();
        Self { chans, ptrs }
    }
}

/// Eine VST3-Instanz. Erzeugen und Freigeben im Hauptthread, `process` im Audio-Thread.
pub struct Vst3Instance {
    component: Obj,
    processor: Obj,
    controller: Obj,
    /// Controller ist ein eigenes Objekt (nicht die Komponente selbst).
    separate_controller: bool,
    connections: Option<(Obj, Obj)>,
    pub info: PluginInfo,
    pub params: Vec<ParamInfo>,
    /// Stufen je Parameter (0 = stufenlos); Werte werden in Stufen angezeigt.
    steps: Arc<HashMap<u32, u32>>,
    ins: Vec<Bus>,
    outs: Vec<Bus>,
    in_bufs: Box<[AudioBusBuffers]>,
    out_bufs: Box<[AudioBusBuffers]>,
    changes: Box<Changes>,
    /// Vom Plugin gemeldete Änderungen (manche Plugins verlangen die Liste); bisher verworfen.
    out_changes: Box<Changes>,
    events: Box<EventList>,
    handler: Box<ComponentHandler>,
    shared: Arc<Shared>,
    max_frames: usize,
    active: bool,
}

// SAFETY: Das Format erlaubt, eine Instanz zwischen Threads weiterzugeben, solange je Funktion
// die Thread-Klasse eingehalten wird (Erzeugen/Freigeben im Hauptthread, process im Audio-Thread).
unsafe impl Send for Vst3Instance {}

impl Vst3Instance {
    /// Erzeugt, initialisiert und aktiviert das Plugin. Nur im Hauptthread.
    pub fn new(info: &PluginInfo, sample_rate: u32, max_frames: u32) -> Result<Self, String> {
        let _main = main_thread_lock();
        let cid = unhex(&info.id).ok_or("ungültige VST3-Klassen-Id")?;
        let f = module(&info.path)?;
        let max = max_frames.max(1) as usize;
        let shared = Arc::new(Shared::default());
        let mut me = Self {
            component: ptr::null_mut(),
            processor: ptr::null_mut(),
            controller: ptr::null_mut(),
            separate_controller: false,
            connections: None,
            info: info.clone(),
            params: Vec::new(),
            steps: Arc::default(),
            ins: Vec::new(),
            outs: Vec::new(),
            in_bufs: Box::new([]),
            out_bufs: Box::new([]),
            changes: Changes::new(),
            out_changes: Changes::new(),
            events: EventList::new(),
            handler: Box::new(ComponentHandler {
                vt: &HANDLER_VT,
                shared: shared.clone(),
            }),
            shared,
            max_frames: max,
            active: false,
        };
        // SAFETY: Fabrik lebt; alle Aufrufe im Hauptthread in der vorgesehenen Reihenfolge.
        // Bei Fehlern räumt Drop auf, was schon besteht.
        unsafe {
            let mut c: Obj = ptr::null_mut();
            if (vt::<FactoryVt>(f).create_instance)(f, &cid, &IID_COMPONENT, &mut c) != RESULT_OK
                || c.is_null()
            {
                return Err(format!(
                    "{}: Komponente ließ sich nicht erzeugen",
                    info.name
                ));
            }
            me.component = c;
            if (vt::<PluginBaseVt>(c).initialize)(c, host_app()) != RESULT_OK {
                (vt::<UnknownVt>(c).release)(c);
                me.component = ptr::null_mut();
                return Err(format!("{}: initialize fehlgeschlagen", info.name));
            }
            me.processor = query(c, &IID_AUDIO_PROCESSOR)
                .ok_or_else(|| format!("{}: kein Audioprozessor", info.name))?;
            // Controller: die Komponente selbst oder eine eigene Klasse.
            if let Some(ctl) = query(c, &IID_EDIT_CONTROLLER) {
                me.controller = ctl;
            } else {
                let mut ccid: Tuid = [0; 16];
                if (vt::<ComponentVt>(c).get_controller_class_id)(c, &mut ccid) == RESULT_OK
                    && ccid != [0; 16]
                {
                    let mut ctl: Obj = ptr::null_mut();
                    if (vt::<FactoryVt>(f).create_instance)(
                        f,
                        &ccid,
                        &IID_EDIT_CONTROLLER,
                        &mut ctl,
                    ) == RESULT_OK
                        && !ctl.is_null()
                    {
                        if (vt::<PluginBaseVt>(ctl).initialize)(ctl, host_app()) == RESULT_OK {
                            me.controller = ctl;
                            me.separate_controller = true;
                            if let (Some(a), Some(b)) = (
                                query(c, &IID_CONNECTION_POINT),
                                query(ctl, &IID_CONNECTION_POINT),
                            ) {
                                (vt::<ConnectionPointVt>(a).connect)(a, b);
                                (vt::<ConnectionPointVt>(b).connect)(b, a);
                                me.connections = Some((a, b));
                            }
                        } else {
                            release(ctl);
                        }
                    }
                }
            }
            if !me.controller.is_null() {
                let h: Obj = (&mut *me.handler as *mut ComponentHandler).cast();
                (vt::<EditControllerVt>(me.controller).set_component_handler)(me.controller, h);
                me.read_params();
            }
            me.setup_buses(max)?;
            let p = me.processor;
            let mut setup = ProcessSetup {
                process_mode: 0,
                symbolic_sample_size: 0,
                max_samples_per_block: max as i32,
                sample_rate: f64::from(sample_rate),
            };
            if (vt::<AudioProcessorVt>(p).setup_processing)(p, &mut setup) != RESULT_OK {
                return Err(format!("{}: setupProcessing fehlgeschlagen", info.name));
            }
            (vt::<ComponentVt>(c).set_active)(c, 1);
            me.active = true;
            (vt::<AudioProcessorVt>(p).set_processing)(p, 1);
            let lat = (vt::<AudioProcessorVt>(p).get_latency_samples)(p).min(MAX_LATENCY);
            me.shared.latency.store(lat, Ordering::Release);
        }
        Ok(me)
    }

    /// Parameterliste vom Controller lesen (Hauptthread, Controller gültig).
    fn read_params(&mut self) {
        let ctl = self.controller;
        // SAFETY: Controller lebt; Struktur in voller Größe; Hauptthread.
        unsafe {
            let cvt = vt::<EditControllerVt>(ctl);
            let n = (cvt.get_parameter_count)(ctl).clamp(0, 100_000);
            for i in 0..n {
                let mut p: ParameterInfo = std::mem::zeroed();
                if (cvt.get_parameter_info)(ctl, i, &mut p) != RESULT_OK
                    || p.flags & (PARAM_HIDDEN | PARAM_READ_ONLY) != 0
                {
                    continue;
                }
                let steps = u32::try_from(p.step_count).unwrap_or(0);
                let def = if p.default_normalized_value.is_finite() {
                    p.default_normalized_value.clamp(0.0, 1.0)
                } else {
                    0.0
                };
                let scale = if steps > 0 { f64::from(steps) } else { 1.0 };
                Arc::make_mut(&mut self.steps).insert(p.id, steps);
                self.params.push(ParamInfo {
                    id: p.id,
                    name: utf16(&p.title),
                    module: String::new(),
                    min: 0.0,
                    max: scale,
                    default: def * scale,
                    value: def * scale,
                    stepped: steps > 0,
                });
            }
        }
    }

    /// Busse aktivieren (Haupt-Audio, Ereignisse) und Puffer für alle Audio-Busse anlegen.
    ///
    /// # Safety
    /// Komponente und Prozessor müssen leben; Hauptthread.
    unsafe fn setup_buses(&mut self, frames: usize) -> Result<(), String> {
        let (c, p) = (self.component, self.processor);
        // SAFETY: laut Vertrag.
        unsafe {
            let cvt = vt::<ComponentVt>(c);
            let n_in = (cvt.get_bus_count)(c, MEDIA_AUDIO, DIR_IN).clamp(0, 16);
            let n_out = (cvt.get_bus_count)(c, MEDIA_AUDIO, DIR_OUT).clamp(0, 16);
            if n_out == 0 {
                return Err(format!("{}: kein Audio-Ausgang", self.info.name));
            }
            // Stereo für die Hauptbusse vorschlagen; lehnt das Plugin ab, gilt seine Vorgabe.
            let mut ins: Vec<u64> = (0..n_in)
                .map(|i| self.arrangement(DIR_IN, i, i == 0))
                .collect();
            let mut outs: Vec<u64> = (0..n_out)
                .map(|i| self.arrangement(DIR_OUT, i, i == 0))
                .collect();
            (vt::<AudioProcessorVt>(p).set_bus_arrangements)(
                p,
                ins.as_mut_ptr(),
                n_in,
                outs.as_mut_ptr(),
                n_out,
            );
            for (dir, n) in [(DIR_IN, n_in), (DIR_OUT, n_out)] {
                for i in 0..n {
                    let mut b: BusInfo = std::mem::zeroed();
                    let ch = if (cvt.get_bus_info)(c, MEDIA_AUDIO, dir, i, &mut b) == RESULT_OK {
                        b.channel_count.clamp(0, 32) as usize
                    } else {
                        0
                    };
                    (cvt.activate_bus)(c, MEDIA_AUDIO, dir, i, u8::from(i == 0));
                    let bus = Bus::new(ch, frames);
                    if dir == DIR_IN {
                        self.ins.push(bus);
                    } else {
                        self.outs.push(bus);
                    }
                }
            }
            if (cvt.get_bus_count)(c, MEDIA_EVENT, DIR_IN) > 0 {
                (cvt.activate_bus)(c, MEDIA_EVENT, DIR_IN, 0, 1);
            }
        }
        let table = |buses: &mut [Bus]| -> Box<[AudioBusBuffers]> {
            buses
                .iter_mut()
                .map(|b| AudioBusBuffers {
                    num_channels: b.chans.len() as i32,
                    silence_flags: 0,
                    channel_buffers32: b.ptrs.as_mut_ptr(),
                })
                .collect()
        };
        self.in_bufs = table(&mut self.ins);
        self.out_bufs = table(&mut self.outs);
        Ok(())
    }

    /// Vorgeschlagene Kanalbelegung: Hauptbus Stereo, sonst die des Plugins.
    fn arrangement(&self, dir: i32, index: i32, main: bool) -> u64 {
        if main {
            return SPEAKER_STEREO;
        }
        let mut a = SPEAKER_MONO;
        // SAFETY: Prozessor lebt; Ausgabezeiger gültig.
        unsafe {
            (vt::<AudioProcessorVt>(self.processor).get_bus_arrangement)(
                self.processor,
                dir,
                index,
                &mut a,
            );
        }
        a
    }

    pub fn handle(&self) -> Vst3Handle {
        Vst3Handle {
            shared: self.shared.clone(),
            component: self.component,
            processor: self.processor,
            controller: if self.separate_controller {
                self.controller
            } else {
                ptr::null_mut()
            },
            any_controller: self.controller,
            steps: self.steps.clone(),
            gui_supported: OnceLock::new(),
            gui: None,
        }
    }

    fn normalized(&self, id: u32, value: f64) -> f64 {
        match self.steps.get(&id) {
            Some(&s) if s > 0 => value / f64::from(s),
            _ => value,
        }
    }

    pub fn set_param(&mut self, id: u32, value: f64) {
        self.set_param_at(0, id, value);
    }

    /// Parameteränderung an Frame `time` des nächsten Blocks (Audio-Thread, ohne Allokation).
    pub fn set_param_at(&mut self, time: u32, id: u32, value: f64) {
        if value.is_finite() {
            let v = self.normalized(id, value);
            self.changes.push(id, time.min(i32::MAX as u32) as i32, v);
        }
    }

    pub fn note(&mut self, time: u32, key: u8, velocity: f32, on: bool) {
        let mut e = Event {
            bus_index: 0,
            sample_offset: time.min(i32::MAX as u32) as i32,
            ppq_position: 0.0,
            flags: 0,
            kind: if on { EVENT_NOTE_ON } else { EVENT_NOTE_OFF },
            body: EventBody { raw: [0; 3] },
        };
        let (pitch, vel) = (i16::from(key.min(127)), velocity.clamp(0.0, 1.0));
        e.body = if on {
            EventBody {
                on: NoteOn {
                    channel: 0,
                    pitch,
                    tuning: 0.0,
                    velocity: vel,
                    length: 0,
                    note_id: -1,
                },
            }
        } else {
            EventBody {
                off: NoteOff {
                    channel: 0,
                    pitch,
                    velocity: vel,
                    note_id: -1,
                    tuning: 0.0,
                },
            }
        };
        self.events.push(e);
    }

    /// MIDI-Bytes: Noten werden zu Ereignissen; andere Meldungen bisher nicht.
    pub fn midi(&mut self, time: u32, data: [u8; 3]) {
        match data[0] & 0xF0 {
            0x90 if data[2] > 0 => self.note(time, data[1], f32::from(data[2]) / 127.0, true),
            0x80 | 0x90 => self.note(time, data[1], f32::from(data[2]) / 127.0, false),
            _ => {}
        }
    }

    /// Einen Stereo-Block an Ort und Stelle rechnen (Audio-Thread, keine Allokation).
    pub fn process(&mut self, left: &mut [f32], right: &mut [f32]) {
        let _tests = crate::clap::test_process_guard();
        let total = left.len().min(right.len());
        let mut done = 0;
        while done < total {
            let n = (total - done).min(self.max_frames);
            let (l, r) = (&mut left[done..done + n], &mut right[done..done + n]);
            for (k, bus) in self.ins.iter_mut().enumerate() {
                match (k, bus.chans.len()) {
                    (0, 1) => {
                        for (d, (a, b)) in bus.chans[0].iter_mut().zip(l.iter().zip(r.iter())) {
                            *d = 0.5 * (a + b);
                        }
                    }
                    (0, c) if c >= 2 => {
                        bus.chans[0][..n].copy_from_slice(l);
                        bus.chans[1][..n].copy_from_slice(r);
                        for ch in bus.chans.iter_mut().skip(2) {
                            ch[..n].fill(0.0);
                        }
                    }
                    _ => {
                        for ch in &mut bus.chans {
                            ch[..n].fill(0.0);
                        }
                    }
                }
            }
            let mut data = ProcessData {
                process_mode: 0,
                symbolic_sample_size: 0,
                num_samples: n as i32,
                num_inputs: self.in_bufs.len() as i32,
                num_outputs: self.out_bufs.len() as i32,
                inputs: self.in_bufs.as_mut_ptr(),
                outputs: self.out_bufs.as_mut_ptr(),
                input_parameter_changes: (&mut *self.changes as *mut Changes).cast(),
                output_parameter_changes: (&mut *self.out_changes as *mut Changes).cast(),
                input_events: (&mut *self.events as *mut EventList).cast(),
                output_events: ptr::null_mut(),
                process_context: ptr::null_mut(),
            };
            // SAFETY: Prozessor aktiv; alle Busse haben Puffer ≥ n; Audio-Thread.
            unsafe {
                (vt::<AudioProcessorVt>(self.processor).process)(self.processor, &mut data);
            }
            if let Some(out) = self.outs.first() {
                match out.chans.len() {
                    0 => {}
                    1 => {
                        l.copy_from_slice(&out.chans[0][..n]);
                        r.copy_from_slice(&out.chans[0][..n]);
                    }
                    _ => {
                        l.copy_from_slice(&out.chans[0][..n]);
                        r.copy_from_slice(&out.chans[1][..n]);
                    }
                }
            }
            for v in l.iter_mut().chain(r.iter_mut()) {
                if !v.is_finite() {
                    *v = 0.0;
                }
            }
            // Änderungen und Ereignisse gelten für den ersten Teilblock.
            self.changes.clear();
            self.out_changes.clear();
            self.events.len = 0;
            done += n;
        }
    }

    pub fn stop_processing(&mut self) {}
}

impl Drop for Vst3Instance {
    /// Anhalten, trennen, beenden und freigeben. Gehört in den Hauptthread.
    fn drop(&mut self) {
        let _main = main_thread_lock();
        // SAFETY: Nur bestehende Objekte; jede Referenz wird genau einmal freigegeben.
        unsafe {
            if self.active {
                let p = self.processor;
                (vt::<AudioProcessorVt>(p).set_processing)(p, 0);
                (vt::<ComponentVt>(self.component).set_active)(self.component, 0);
            }
            if let Some((a, b)) = self.connections.take() {
                (vt::<ConnectionPointVt>(a).disconnect)(a, b);
                (vt::<ConnectionPointVt>(b).disconnect)(b, a);
                release(a);
                release(b);
            }
            if !self.controller.is_null() {
                let ctl = self.controller;
                (vt::<EditControllerVt>(ctl).set_component_handler)(ctl, ptr::null_mut());
                if self.separate_controller {
                    (vt::<PluginBaseVt>(ctl).terminate)(ctl);
                }
                release(ctl);
            }
            release(self.processor);
            if !self.component.is_null() {
                (vt::<PluginBaseVt>(self.component).terminate)(self.component);
                release(self.component);
            }
        }
    }
}

/// Griff auf eine VST3-Instanz für den Hauptthread (Zustand, Latenz), während sie in der
/// Engine rechnet. Muss vor der Instanz freigegeben werden; die Zeiger leben mit ihr.
pub struct Vst3Handle {
    shared: Arc<Shared>,
    component: Obj,
    processor: Obj,
    /// Eigener Controller (null, wenn die Komponente selbst Controller ist).
    controller: Obj,
    /// Controller überhaupt (eigener oder die Komponente), für setComponentState.
    any_controller: Obj,
    steps: Arc<HashMap<u32, u32>>,
    gui_supported: OnceLock<bool>,
    gui: Option<Box<crate::vst3_gui::Vst3Gui>>,
}

impl Drop for Vst3Handle {
    fn drop(&mut self) {
        // Fenster vor der Instanz schließen (Vertrag: Griff stirbt zuerst).
        self.close_gui();
    }
}

// SAFETY: Zustand und Latenz sind laut Formatbeschreibung Aufrufe aus dem UI-Thread, parallel zur
// Verarbeitung erlaubt; der Griff wird nur dort benutzt und lebt kürzer als die Instanz.
unsafe impl Send for Vst3Handle {}

impl Vst3Handle {
    /// Ob das Plugin ein Fenster hat, das sich einbetten lässt (einmal geprüft).
    pub fn has_gui(&self) -> bool {
        *self.gui_supported.get_or_init(|| {
            let _main = main_thread_lock();
            crate::vst3_gui::Vst3Gui::supported(self.any_controller)
        })
    }

    pub fn gui_open(&self) -> bool {
        self.gui.is_some()
    }

    /// Öffnet das Plugin-Fenster. Nur im Hauptthread; danach Werte mit
    /// [`Self::set_ui_value`] übergeben.
    pub fn open_gui(&mut self, title: &str) -> Result<(), String> {
        if self.gui.is_some() {
            return Ok(());
        }
        if !crate::host::on_main_thread() {
            return Err("Plugin-Fenster nur im Hauptthread".into());
        }
        let _main = main_thread_lock();
        let gui = crate::vst3_gui::Vst3Gui::open(self.any_controller, title)?;
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

    /// Bearbeitungen im Plugin-Fenster seit dem letzten Aufruf: (Id, Wert in Anzeige-Einheiten,
    /// also Stufen bei gestuften Parametern).
    pub fn take_param_changes(&self) -> Vec<(u32, f64)> {
        self.shared
            .edits
            .take()
            .into_iter()
            .map(|(id, v)| match self.steps.get(&id) {
                Some(&s) if s > 0 => (id, (v * f64::from(s)).round()),
                _ => (id, v),
            })
            .collect()
    }

    /// Bearbeitungen, die verworfen wurden, weil mehr Parameter gemeldet wurden, als die
    /// Tabelle fasst.
    pub fn dropped_param_changes(&self) -> u64 {
        self.shared.edits.dropped()
    }

    /// Parameterwert (Anzeige-Einheiten) an den Controller geben, damit ein offenes Fenster ihn
    /// zeigt. Hauptthread.
    pub fn set_ui_value(&self, id: u32, value: f64) {
        if self.any_controller.is_null() || !value.is_finite() {
            return;
        }
        let v = match self.steps.get(&id) {
            Some(&s) if s > 0 => value / f64::from(s),
            _ => value,
        };
        // SAFETY: Controller lebt (Vertrag des Griffs); Hauptthread.
        unsafe {
            (vt::<EditControllerVt>(self.any_controller).set_param_normalized)(
                self.any_controller,
                id,
                v.clamp(0.0, 1.0),
            );
        }
    }

    pub fn latency(&self) -> u32 {
        self.shared.latency.load(Ordering::Acquire)
    }

    /// Hat das Plugin eine geänderte Latenz gemeldet? Dann wird sie hier neu gelesen.
    pub fn take_latency_changed(&self) -> bool {
        if self.shared.latency_dirty.swap(0, Ordering::AcqRel) == 0 {
            return false;
        }
        let _main = main_thread_lock();
        // SAFETY: Prozessor lebt (Vertrag des Griffs); Abruf im Hauptthread.
        let lat =
            unsafe { (vt::<AudioProcessorVt>(self.processor).get_latency_samples)(self.processor) };
        self.shared
            .latency
            .store(lat.min(MAX_LATENCY), Ordering::Release);
        true
    }

    /// Zustand von Komponente und (eigenem) Controller speichern. Hauptthread.
    pub fn save_state(&self) -> Option<Vec<u8>> {
        let _main = main_thread_lock();
        let mut comp = Stream::new(Vec::new());
        // SAFETY: Komponente lebt; der Strom lebt während des Aufrufs.
        let ok = unsafe {
            (vt::<ComponentVt>(self.component).get_state)(self.component, comp.obj()) == RESULT_OK
        };
        if !ok {
            return None;
        }
        let mut ctl = Stream::new(Vec::new());
        if !self.controller.is_null() {
            // SAFETY: Controller lebt; ein Fehler heißt nur: kein Controller-Zustand.
            let ok = unsafe {
                (vt::<EditControllerVt>(self.controller).get_state)(self.controller, ctl.obj())
                    == RESULT_OK
            };
            if !ok {
                ctl.data.clear();
            }
        }
        Some(pack_state(&comp.data, &ctl.data))
    }

    /// Gespeicherten Zustand laden. Nur bevor die Instanz rechnet – StudioCraft lädt ihn beim
    /// Erzeugen. Der Controller bekommt den Zustand der Komponente mit (gleiche Anzeige).
    pub fn load_state(&self, data: &[u8]) -> bool {
        let Some((comp, ctl)) = unpack_state(data) else {
            return false;
        };
        let _main = main_thread_lock();
        // SAFETY: Objekte leben; Ströme leben während der Aufrufe.
        unsafe {
            let mut s = Stream::new(comp.to_vec());
            if (vt::<ComponentVt>(self.component).set_state)(self.component, s.obj()) != RESULT_OK {
                return false;
            }
            if !self.any_controller.is_null() {
                let mut s = Stream::new(comp.to_vec());
                (vt::<EditControllerVt>(self.any_controller).set_component_state)(
                    self.any_controller,
                    s.obj(),
                );
            }
            if !self.controller.is_null() && !ctl.is_empty() {
                let mut s = Stream::new(ctl.to_vec());
                (vt::<EditControllerVt>(self.controller).set_state)(self.controller, s.obj());
            }
        }
        true
    }
}

/// Ist `path` ein VST3-Bundle oder eine VST3-Datei?
pub(crate) fn is_vst3(path: &Path) -> bool {
    path.extension()
        .is_some_and(|x| x.eq_ignore_ascii_case("vst3"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn uids_follow_the_platform_layout() {
        let u = uid(0x0102_0304, 0x0506_0708, 0x090A_0B0C, 0x0D0E_0F10);
        if cfg!(windows) {
            assert_eq!(&u[..8], &[4, 3, 2, 1, 6, 5, 8, 7]);
        } else {
            assert_eq!(&u[..8], &[1, 2, 3, 4, 5, 6, 7, 8]);
        }
        assert_eq!(&u[8..], &[9, 10, 11, 12, 13, 14, 15, 16]);
        assert_eq!(unhex(&hex(&u)), Some(u));
        assert_eq!(unhex("zz"), None);
    }

    #[test]
    fn struct_layouts_match_the_format() {
        assert_eq!(std::mem::size_of::<Event>(), 48);
        assert_eq!(std::mem::size_of::<ParameterInfo>(), 792);
        assert_eq!(std::mem::size_of::<AudioBusBuffers>(), 24);
        assert_eq!(std::mem::size_of::<ProcessSetup>(), 24);
        assert_eq!(std::mem::size_of::<ClassInfo2>(), 116 + 4 + 128 + 64 * 3);
    }

    #[test]
    fn parameter_changes_stay_bounded() {
        let mut c = Changes::new();
        for i in 0..(MAX_QUEUES as u32 + 10) {
            c.push(i, 0, 0.5);
        }
        assert_eq!(c.used, MAX_QUEUES);
        for k in 0..40 {
            c.push(0, k, 2.0);
        }
        assert_eq!(c.queues[0].len, MAX_POINTS);
        assert_eq!(c.queues[0].points[MAX_POINTS - 1].1, 1.0, "clamped");
        c.clear();
        assert_eq!(c.used, 0);
    }

    fn installed() -> Vec<PluginInfo> {
        let mut out = Vec::new();
        for dir in ["/usr/lib/vst3", "/usr/local/lib/vst3"] {
            let Ok(rd) = std::fs::read_dir(dir) else {
                continue;
            };
            for e in rd.flatten() {
                if is_vst3(&e.path())
                    && let Ok(list) = descriptors(&e.path())
                {
                    out.extend(list);
                }
            }
        }
        out
    }

    #[test]
    fn state_container_roundtrips_and_rejects_garbage() {
        let packed = pack_state(b"comp", b"");
        assert_eq!(unpack_state(&packed), Some((&b"comp"[..], &b""[..])));
        let packed = pack_state(b"", b"ctl");
        assert_eq!(unpack_state(&packed), Some((&b""[..], &b"ctl"[..])));
        assert_eq!(
            unpack_state(b"SCV3\xff\xff\xff\xff"),
            None,
            "length past the end"
        );
        assert_eq!(unpack_state(b"XXXX"), None);
        let mut long = pack_state(b"a", b"b");
        long.push(0);
        assert_eq!(unpack_state(&long), None, "trailing bytes");
    }

    #[test]
    fn stream_reads_writes_and_seeks_within_bounds() {
        let mut s = Stream::new(Vec::new());
        let o = s.obj();
        let (mut n, mut pos) = (0i32, 0i64);
        let mut buf = [0u8; 8];
        // SAFETY: Test ruft den Strom wie ein Plugin auf.
        unsafe {
            assert_eq!(
                stream_write(o, b"hello".as_ptr().cast(), 5, &mut n),
                RESULT_OK
            );
            assert_eq!(n, 5);
            assert_eq!(stream_seek(o, 1, 0, &mut pos), RESULT_OK);
            assert_eq!(
                stream_read(o, buf.as_mut_ptr().cast(), 8, &mut n),
                RESULT_OK
            );
            assert_eq!((n, &buf[..4]), (4, &b"ello"[..]));
            assert_eq!(stream_seek(o, -1, 0, &mut pos), -1, "before the start");
            assert_eq!(stream_seek(o, 0, 9, &mut pos), -1, "unknown mode");
            assert_eq!(stream_tell(o, &mut pos), RESULT_OK);
            assert_eq!(pos, 5);
        }
    }

    #[test]
    fn plugin_state_survives_a_new_instance() {
        let Some(info) = installed().into_iter().find(|p| p.name == "ZamComp") else {
            eprintln!("ZamComp (VST3) nicht installiert – übersprungen");
            return;
        };
        let a = Vst3Instance::new(&info, 48_000, 256);
        assert!(a.is_ok());
        let Ok(mut a) = a else { return };
        // Einen Parameter verstellen und einen Block rechnen, damit er im Plugin ankommt.
        let p = a.params[0].clone();
        a.set_param(p.id, (p.min + p.max) * 0.25);
        let (mut l, mut r) = (vec![0.0f32; 64], vec![0.0f32; 64]);
        a.process(&mut l, &mut r);
        let state = a.handle().save_state();
        assert!(state.is_some(), "ZamComp saves its state");
        let Some(state) = state else { return };
        let fresh = Vst3Instance::new(&info, 48_000, 256);
        assert!(fresh.is_ok());
        let Ok(b) = fresh else { return };
        let default_state = b.handle().save_state();
        assert_ne!(
            default_state.as_ref(),
            Some(&state),
            "changed parameter is in the state"
        );
        assert!(b.handle().load_state(&state));
        assert_eq!(
            b.handle().save_state(),
            Some(state),
            "same state after loading"
        );
        assert!(!b.handle().load_state(b"kein Zustand"));
    }

    #[test]
    fn host_creates_messages_with_attributes() {
        let mut m: Obj = ptr::null_mut();
        // SAFETY: Test ruft die Host-Objekte wie ein Plugin auf.
        unsafe {
            assert_eq!(
                app_create_instance(host_app(), &IID_MESSAGE, &IID_MESSAGE, &mut m),
                RESULT_OK
            );
            let mv = vt::<MessageVt>(m);
            (mv.set_message_id)(m, c"state".as_ptr());
            assert_eq!(cstr_of((mv.get_message_id)(m)), "state");
            let a = (mv.get_attributes)(m);
            let av = vt::<AttrListVt>(a);
            assert_eq!((av.set_int)(a, c"n".as_ptr(), 7), RESULT_OK);
            let mut n = 0i64;
            assert_eq!((av.get_int)(a, c"n".as_ptr(), &mut n), RESULT_OK);
            assert_eq!(n, 7);
            assert_eq!((av.get_float)(a, c"n".as_ptr(), &mut 0.0), -1, "wrong type");
            let text: Vec<u16> = "Hallo\0".encode_utf16().collect();
            assert_eq!((av.set_string)(a, c"t".as_ptr(), text.as_ptr()), RESULT_OK);
            let mut buf = [0u16; 4];
            assert_eq!(
                (av.get_string)(a, c"t".as_ptr(), buf.as_mut_ptr(), 8),
                RESULT_OK
            );
            assert_eq!(utf16(&buf), "Hal", "cut to the buffer");
            let bytes = [1u8, 2, 3];
            (av.set_binary)(a, c"b".as_ptr(), bytes.as_ptr().cast(), 3);
            let (mut dp, mut len): (*const c_void, u32) = (ptr::null(), 0);
            assert_eq!(
                (av.get_binary)(a, c"b".as_ptr(), &mut dp, &mut len),
                RESULT_OK
            );
            assert_eq!(
                std::slice::from_raw_parts(dp.cast::<u8>(), len as usize),
                &bytes
            );
            // Zusätzliche Referenz, dann beide freigeben (die Nachricht gibt ihre Liste frei).
            assert_eq!((vt::<UnknownVt>(m).add_ref)(m), 2);
            assert_eq!((vt::<UnknownVt>(m).release)(m), 1);
            assert_eq!((vt::<UnknownVt>(m).release)(m), 0);
        }
    }

    fn cstr_of(p: *const c_char) -> String {
        // SAFETY: Test: nullterminierte Zeichenkette aus einer Nachricht.
        unsafe { std::ffi::CStr::from_ptr(p) }
            .to_string_lossy()
            .into_owned()
    }

    #[test]
    fn window_edits_reach_the_host_in_display_units() {
        let shared = Arc::new(Shared::default());
        let mut handler = ComponentHandler {
            vt: &HANDLER_VT,
            shared: shared.clone(),
        };
        let h: Obj = (&mut handler as *mut ComponentHandler).cast();
        // SAFETY: Test ruft den Handler wie ein Plugin auf.
        unsafe {
            handler_perform(h, 1, 0.25);
            handler_perform(h, 2, 0.5);
            handler_perform(h, 1, 0.75);
            assert_eq!(handler_perform(h, 3, f64::NAN), -1);
        }
        let handle = Vst3Handle {
            shared,
            component: ptr::null_mut(),
            processor: ptr::null_mut(),
            controller: ptr::null_mut(),
            any_controller: ptr::null_mut(),
            steps: Arc::new(HashMap::from([(2, 4)])),
            gui_supported: OnceLock::new(),
            gui: None,
        };
        let mut edits = handle.take_param_changes();
        edits.sort_by_key(|e| e.0);
        assert_eq!(
            edits,
            vec![(1, 0.75), (2, 2.0)],
            "last value, steps for stepped"
        );
        assert!(handle.take_param_changes().is_empty());
    }

    #[test]
    fn host_values_reach_the_controller() {
        let Some(info) = installed().into_iter().find(|p| p.name == "ZamComp") else {
            return;
        };
        let inst = Vst3Instance::new(&info, 48_000, 256);
        assert!(inst.is_ok());
        let Ok(inst) = inst else { return };
        let p = inst.params[0].clone();
        let h = inst.handle();
        h.set_ui_value(p.id, p.max);
        // SAFETY: Controller lebt; Abruf im Hauptthread (Test).
        let v = unsafe {
            (vt::<EditControllerVt>(h.any_controller).get_param_normalized)(h.any_controller, p.id)
        };
        assert!(
            (v - 1.0).abs() < 1e-6,
            "controller shows the host value: {v}"
        );
        assert!(h.has_gui(), "ZamComp has an embeddable editor");
    }

    #[test]
    fn installed_vst3_effects_process_audio() {
        let plugins = installed();
        if plugins.is_empty() {
            eprintln!("keine VST3-Plugins installiert – übersprungen");
            return;
        }
        let mut ok = 0;
        for info in plugins.iter().filter(|p| !p.is_instrument()).take(30) {
            let Ok(mut inst) = Vst3Instance::new(info, 48_000, 256) else {
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
        assert!(ok > 0, "at least one VST3 effect instantiates");
    }
}
