//! Audio-Unit-Hosting (Issue #34), nur macOS: eigene, schlanke Anbindung an AudioToolbox und
//! CoreFoundation (Systembibliotheken, keine Abhängigkeit). Auf anderen Systemen gibt es keine
//! Audio Units; die Typen melden dort „nicht verfügbar“.
//!
//! Umfang: Komponenten aus der System-Registry lesen (lädt keinen Plugin-Code), Instanz mit
//! Stereo-Float-Format, Eingang über einen Render-Callback, Parameter (globaler Bereich),
//! MIDI für Instrumente, Latenz, Zustand (Klassen-Info als binäre Property-List). Fenster folgen.

use crate::{clap::ParamInfo, scan::PluginInfo};

/// Vierzeichen-Code als Text (`aufx`).
#[cfg_attr(not(target_os = "macos"), allow(dead_code))]
fn fourcc(v: u32) -> String {
    v.to_be_bytes()
        .iter()
        .map(|&b| {
            if b.is_ascii_graphic() || b == b' ' {
                b as char
            } else {
                '?'
            }
        })
        .collect()
}

/// Text (genau vier Zeichen) als Vierzeichen-Code.
#[cfg_attr(not(target_os = "macos"), allow(dead_code))]
fn from_fourcc(s: &str) -> Option<u32> {
    let b = s.as_bytes();
    (b.len() == 4).then(|| u32::from_be_bytes([b[0], b[1], b[2], b[3]]))
}

/// Plugin-Id einer Audio Unit: `typ:subtyp:hersteller` als Vierzeichen-Codes.
#[cfg_attr(not(target_os = "macos"), allow(dead_code))]
fn parse_id(id: &str) -> Option<(u32, u32, u32)> {
    let mut it = id.split(':');
    let t = from_fourcc(it.next()?)?;
    let s = from_fourcc(it.next()?)?;
    let m = from_fourcc(it.next()?)?;
    it.next().is_none().then_some((t, s, m))
}

#[cfg(target_os = "macos")]
pub use mac::{AuHandle, AuInstance, scan};

#[cfg(not(target_os = "macos"))]
pub use other::{AuHandle, AuInstance, scan};

#[cfg(not(target_os = "macos"))]
mod other {
    use super::*;

    /// Audio Units gibt es nur unter macOS.
    pub fn scan() -> Vec<PluginInfo> {
        Vec::new()
    }

    pub struct AuInstance {
        pub info: PluginInfo,
        pub params: Vec<ParamInfo>,
    }

    impl AuInstance {
        pub fn new(_: &PluginInfo, _: u32, _: u32) -> Result<Self, String> {
            Err("Audio Units gibt es nur unter macOS".into())
        }
        pub fn handle(&self) -> AuHandle {
            AuHandle
        }
        pub fn set_param(&mut self, _: u32, _: f64) {}
        pub fn set_param_at(&mut self, _: u32, _: u32, _: f64) {}
        pub fn midi(&mut self, _: u32, _: [u8; 3]) {}
        pub fn note(&mut self, _: u32, _: u8, _: f32, _: bool) {}
        pub fn process(&mut self, _: &mut [f32], _: &mut [f32]) {}
        pub fn stop_processing(&mut self) {}
    }

    pub struct AuHandle;

    impl AuHandle {
        pub fn latency(&self) -> u32 {
            0
        }
        pub fn take_latency_changed(&self) -> bool {
            false
        }
        pub fn save_state(&self) -> Option<Vec<u8>> {
            None
        }
        pub fn load_state(&self, _: &[u8]) -> bool {
            false
        }
        pub fn has_gui(&self) -> bool {
            false
        }
        pub fn gui_open(&self) -> bool {
            false
        }
        pub fn open_gui(&mut self, _: &str) -> Result<(), String> {
            Err("Audio Units gibt es nur unter macOS".into())
        }
        pub fn close_gui(&mut self) {}
        pub fn idle(&mut self) -> bool {
            false
        }
    }
}

#[cfg(target_os = "macos")]
mod mac {
    use std::{
        ffi::{c_char, c_void},
        ptr,
        sync::{
            Arc,
            atomic::{AtomicU32, Ordering},
        },
    };

    use super::*;
    use crate::{
        clap::{MAX_LATENCY, main_thread_lock},
        scan::PluginFormat,
    };

    type OsStatus = i32;
    type Unit = *mut c_void;
    type Component = *mut c_void;
    type CfRef = *const c_void;

    #[repr(C)]
    #[derive(Default, Clone, Copy)]
    struct ComponentDescription {
        kind: u32,
        sub_kind: u32,
        manufacturer: u32,
        flags: u32,
        flags_mask: u32,
    }

    #[repr(C)]
    struct StreamFormat {
        sample_rate: f64,
        format_id: u32,
        format_flags: u32,
        bytes_per_packet: u32,
        frames_per_packet: u32,
        bytes_per_frame: u32,
        channels_per_frame: u32,
        bits_per_channel: u32,
        reserved: u32,
    }

    #[repr(C)]
    #[derive(Default)]
    struct SmpteTime {
        subframes: i16,
        subframe_divisor: i16,
        counter: u32,
        kind: u32,
        flags: u32,
        hours: i16,
        minutes: i16,
        seconds: i16,
        frames: i16,
    }

    #[repr(C)]
    #[derive(Default)]
    struct TimeStamp {
        sample_time: f64,
        host_time: u64,
        rate_scalar: f64,
        word_clock_time: u64,
        smpte: SmpteTime,
        flags: u32,
        reserved: u32,
    }

    #[repr(C)]
    #[derive(Clone, Copy)]
    struct Buffer {
        channels: u32,
        byte_size: u32,
        data: *mut c_void,
    }

    /// Pufferliste mit Platz für zwei Kanäle (nicht verschachtelt).
    #[repr(C)]
    struct BufferList {
        count: u32,
        buffers: [Buffer; 2],
    }

    type RenderCallback = unsafe extern "C" fn(
        *mut c_void,
        *mut u32,
        *const TimeStamp,
        u32,
        u32,
        *mut BufferList,
    ) -> OsStatus;

    #[repr(C)]
    struct RenderCallbackStruct {
        proc_: Option<RenderCallback>,
        ref_con: *mut c_void,
    }

    #[repr(C)]
    struct ParameterInfo {
        name: [c_char; 52],
        unit_name: CfRef,
        clump_id: u32,
        cf_name: CfRef,
        unit: u32,
        min: f32,
        max: f32,
        default: f32,
        flags: u32,
    }

    // Auf älteren Systemen liegen die Audio-Unit-Funktionen im AudioUnit-Framework.
    #[link(name = "AudioUnit", kind = "framework")]
    unsafe extern "C" {}

    #[link(name = "AudioToolbox", kind = "framework")]
    unsafe extern "C" {
        fn AudioComponentFindNext(c: Component, d: *const ComponentDescription) -> Component;
        fn AudioComponentCopyName(c: Component, name: *mut CfRef) -> OsStatus;
        fn AudioComponentGetDescription(c: Component, d: *mut ComponentDescription) -> OsStatus;
        fn AudioComponentGetVersion(c: Component, v: *mut u32) -> OsStatus;
        fn AudioComponentInstanceNew(c: Component, out: *mut Unit) -> OsStatus;
        fn AudioComponentInstanceDispose(u: Unit) -> OsStatus;
        fn AudioUnitInitialize(u: Unit) -> OsStatus;
        fn AudioUnitUninitialize(u: Unit) -> OsStatus;
        fn AudioUnitSetProperty(
            u: Unit,
            id: u32,
            scope: u32,
            element: u32,
            data: *const c_void,
            size: u32,
        ) -> OsStatus;
        fn AudioUnitGetProperty(
            u: Unit,
            id: u32,
            scope: u32,
            element: u32,
            data: *mut c_void,
            size: *mut u32,
        ) -> OsStatus;
        fn AudioUnitGetPropertyInfo(
            u: Unit,
            id: u32,
            scope: u32,
            element: u32,
            size: *mut u32,
            writable: *mut u8,
        ) -> OsStatus;
        fn AudioUnitSetParameter(
            u: Unit,
            id: u32,
            scope: u32,
            element: u32,
            value: f32,
            offset: u32,
        ) -> OsStatus;
        fn AudioUnitRender(
            u: Unit,
            flags: *mut u32,
            time: *const TimeStamp,
            bus: u32,
            frames: u32,
            data: *mut BufferList,
        ) -> OsStatus;
        fn MusicDeviceMIDIEvent(u: Unit, status: u32, d1: u32, d2: u32, offset: u32) -> OsStatus;
    }

    #[link(name = "CoreFoundation", kind = "framework")]
    unsafe extern "C" {
        fn CFRelease(cf: CfRef);
        fn CFStringGetCString(s: CfRef, buf: *mut c_char, size: isize, encoding: u32) -> u8;
        fn CFPropertyListCreateData(
            alloc: CfRef,
            plist: CfRef,
            format: isize,
            options: usize,
            error: *mut CfRef,
        ) -> CfRef;
        fn CFPropertyListCreateWithData(
            alloc: CfRef,
            data: CfRef,
            options: usize,
            format: *mut isize,
            error: *mut CfRef,
        ) -> CfRef;
        fn CFDataCreate(alloc: CfRef, bytes: *const u8, len: isize) -> CfRef;
        fn CFDataGetLength(d: CfRef) -> isize;
        fn CFDataGetBytePtr(d: CfRef) -> *const u8;
    }

    const TYPE_EFFECT: u32 = u32::from_be_bytes(*b"aufx");
    const TYPE_INSTRUMENT: u32 = u32::from_be_bytes(*b"aumu");
    const TYPE_MUSIC_EFFECT: u32 = u32::from_be_bytes(*b"aumf");
    const FORMAT_LPCM: u32 = u32::from_be_bytes(*b"lpcm");
    /// Float | gepackt | nicht verschachtelt.
    const FLAGS_FLOAT_PLANAR: u32 = 1 | 8 | 32;
    const SCOPE_GLOBAL: u32 = 0;
    const SCOPE_INPUT: u32 = 1;
    const SCOPE_OUTPUT: u32 = 2;
    const PROP_CLASS_INFO: u32 = 0;
    const PROP_PARAMETER_LIST: u32 = 3;
    const PROP_PARAMETER_INFO: u32 = 4;
    const PROP_STREAM_FORMAT: u32 = 8;
    const PROP_LATENCY: u32 = 12;
    const PROP_MAX_FRAMES: u32 = 14;
    const PROP_RENDER_CALLBACK: u32 = 23;
    const PARAM_FLAG_HAS_CF_NAME: u32 = 1 << 27;
    const PARAM_FLAG_CF_NAME_RELEASE: u32 = 1 << 4;
    const PARAM_FLAG_WRITABLE: u32 = 1 << 31;
    const UNIT_INDEXED: u32 = 1;
    const UNIT_BOOLEAN: u32 = 2;
    const UTF8: u32 = 0x0800_0100;
    const PLIST_BINARY: isize = 200;
    const TIME_SAMPLE_VALID: u32 = 1;
    /// Höchstgröße eines Zustands (Schutz gegen entgleiste Plugins und kaputte Dateien).
    const MAX_STATE: isize = 256 << 20;

    /// CFString als Rust-Text (ohne Freigabe).
    fn cf_string(s: CfRef) -> String {
        if s.is_null() {
            return String::new();
        }
        let mut buf = [0 as c_char; 512];
        // SAFETY: `s` ist ein CFString; Puffer mit angegebener Größe.
        let ok = unsafe { CFStringGetCString(s, buf.as_mut_ptr(), buf.len() as isize, UTF8) };
        if ok == 0 {
            return String::new();
        }
        let bytes: Vec<u8> = buf
            .iter()
            .take_while(|&&c| c != 0)
            .map(|&c| c as u8)
            .collect();
        String::from_utf8_lossy(&bytes).into_owned()
    }

    fn describe(c: Component) -> Option<PluginInfo> {
        let mut d = ComponentDescription::default();
        let mut name: CfRef = ptr::null();
        let mut version = 0u32;
        // SAFETY: Komponente aus der Registry; Ausgabezeiger gültig. Der Name gehört uns.
        let full = unsafe {
            if AudioComponentGetDescription(c, &mut d) != 0 {
                return None;
            }
            AudioComponentGetVersion(c, &mut version);
            if AudioComponentCopyName(c, &mut name) != 0 {
                return None;
            }
            let s = cf_string(name);
            if !name.is_null() {
                CFRelease(name);
            }
            s
        };
        // Namen kommen als „Hersteller: Plugin“.
        let (vendor, plugin) = match full.split_once(": ") {
            Some((v, p)) => (v.to_owned(), p.to_owned()),
            None => (String::new(), full),
        };
        Some(PluginInfo {
            format: PluginFormat::Au,
            id: format!(
                "{}:{}:{}",
                fourcc(d.kind),
                fourcc(d.sub_kind),
                fourcc(d.manufacturer)
            ),
            name: plugin,
            vendor,
            version: format!(
                "{}.{}.{}",
                version >> 16,
                (version >> 8) & 0xFF,
                version & 0xFF
            ),
            features: vec![if d.kind == TYPE_INSTRUMENT {
                "instrument".into()
            } else {
                "audio-effect".into()
            }],
            path: Default::default(),
        })
    }

    /// Alle installierten Effekte und Instrumente. Liest nur die System-Registry.
    pub fn scan() -> Vec<PluginInfo> {
        let _main = main_thread_lock();
        let mut out = Vec::new();
        for kind in [TYPE_EFFECT, TYPE_MUSIC_EFFECT, TYPE_INSTRUMENT] {
            let d = ComponentDescription {
                kind,
                ..Default::default()
            };
            let mut c: Component = ptr::null_mut();
            for _ in 0..4096 {
                // SAFETY: Beschreibung gültig; null beginnt bzw. beendet die Suche.
                c = unsafe { AudioComponentFindNext(c, &d) };
                if c.is_null() {
                    break;
                }
                out.extend(describe(c));
            }
        }
        out
    }

    fn find(id: &str) -> Option<Component> {
        let (kind, sub_kind, manufacturer) = parse_id(id)?;
        let d = ComponentDescription {
            kind,
            sub_kind,
            manufacturer,
            ..Default::default()
        };
        // SAFETY: wie in `scan`.
        let c = unsafe { AudioComponentFindNext(ptr::null_mut(), &d) };
        (!c.is_null()).then_some(c)
    }

    /// Eingang für den Render-Callback: zwei Kanäle, vorab angelegt.
    struct Input {
        left: Box<[f32]>,
        right: Box<[f32]>,
    }

    unsafe extern "C" fn provide_input(
        ref_con: *mut c_void,
        _flags: *mut u32,
        _time: *const TimeStamp,
        _bus: u32,
        frames: u32,
        data: *mut BufferList,
    ) -> OsStatus {
        if ref_con.is_null() || data.is_null() {
            return -50;
        }
        // SAFETY: `ref_con` ist unser `Input` (lebt mit der Instanz); `data` vom Plugin.
        unsafe {
            let input = &*(ref_con as *const Input);
            let list = &mut *data;
            let n = (frames as usize).min(input.left.len());
            let count = (list.count as usize).min(2);
            for (k, buf) in list.buffers.iter_mut().take(count).enumerate() {
                let src = if k == 0 { &input.left } else { &input.right };
                if buf.data.is_null() {
                    // Das Plugin erwartet unsere Zeiger.
                    buf.data = src.as_ptr() as *mut c_void;
                } else {
                    ptr::copy_nonoverlapping(src.as_ptr(), buf.data.cast::<f32>(), n);
                }
                buf.byte_size = (n * 4) as u32;
            }
        }
        0
    }

    #[derive(Default)]
    struct Shared {
        latency: AtomicU32,
    }

    /// Eine Audio-Unit-Instanz. Erzeugen und Freigeben im Hauptthread, `process` im Audio-Thread.
    pub struct AuInstance {
        unit: Unit,
        pub info: PluginInfo,
        pub params: Vec<ParamInfo>,
        input: Box<Input>,
        out_left: Box<[f32]>,
        out_right: Box<[f32]>,
        /// Ausgabekanäle (1 oder 2), wie das Plugin sie annahm.
        out_channels: u32,
        sample_time: f64,
        max_frames: usize,
        initialized: bool,
        shared: Arc<Shared>,
    }

    // SAFETY: Audio Units dürfen zwischen Threads weitergegeben werden; Erzeugen und Freigeben im
    // Hauptthread, Rendern im Audio-Thread.
    unsafe impl Send for AuInstance {}

    fn stream_format(rate: u32, channels: u32) -> StreamFormat {
        StreamFormat {
            sample_rate: f64::from(rate),
            format_id: FORMAT_LPCM,
            format_flags: FLAGS_FLOAT_PLANAR,
            bytes_per_packet: 4,
            frames_per_packet: 1,
            bytes_per_frame: 4,
            channels_per_frame: channels,
            bits_per_channel: 32,
            reserved: 0,
        }
    }

    /// Stereo anbieten, sonst Mono; die angenommene Kanalzahl.
    ///
    /// # Safety
    /// `unit` muss eine lebende, nicht initialisierte Instanz sein.
    unsafe fn set_format(unit: Unit, scope: u32, rate: u32) -> Option<u32> {
        for ch in [2, 1] {
            let f = stream_format(rate, ch);
            // SAFETY: laut Vertrag; Struktur in voller Größe.
            let r = unsafe {
                AudioUnitSetProperty(
                    unit,
                    PROP_STREAM_FORMAT,
                    scope,
                    0,
                    (&f as *const StreamFormat).cast(),
                    std::mem::size_of::<StreamFormat>() as u32,
                )
            };
            if r == 0 {
                return Some(ch);
            }
        }
        None
    }

    impl AuInstance {
        /// Erzeugt und initialisiert die Audio Unit. Nur im Hauptthread.
        pub fn new(info: &PluginInfo, sample_rate: u32, max_frames: u32) -> Result<Self, String> {
            let _main = main_thread_lock();
            let comp = find(&info.id)
                .ok_or_else(|| format!("{}: Audio Unit nicht gefunden", info.name))?;
            let max = max_frames.max(1) as usize;
            let mut unit: Unit = ptr::null_mut();
            // SAFETY: Komponente aus der Registry; Ausgabezeiger gültig.
            if unsafe { AudioComponentInstanceNew(comp, &mut unit) } != 0 || unit.is_null() {
                return Err(format!(
                    "{}: Audio Unit ließ sich nicht erzeugen",
                    info.name
                ));
            }
            let mut me = Self {
                unit,
                info: info.clone(),
                params: Vec::new(),
                input: Box::new(Input {
                    left: vec![0.0; max].into_boxed_slice(),
                    right: vec![0.0; max].into_boxed_slice(),
                }),
                out_left: vec![0.0; max].into_boxed_slice(),
                out_right: vec![0.0; max].into_boxed_slice(),
                out_channels: 2,
                sample_time: 0.0,
                max_frames: max,
                initialized: false,
                shared: Arc::default(),
            };
            // SAFETY: Instanz lebt, noch nicht initialisiert; Hauptthread. Bei Fehlern räumt Drop auf.
            unsafe {
                let frames = max as u32;
                AudioUnitSetProperty(
                    unit,
                    PROP_MAX_FRAMES,
                    SCOPE_GLOBAL,
                    0,
                    (&frames as *const u32).cast(),
                    4,
                );
                me.out_channels = set_format(unit, SCOPE_OUTPUT, sample_rate)
                    .ok_or_else(|| format!("{}: kein passendes Ausgabeformat", info.name))?;
                if !info.is_instrument() {
                    set_format(unit, SCOPE_INPUT, sample_rate);
                    let cb = RenderCallbackStruct {
                        proc_: Some(provide_input),
                        ref_con: (&*me.input as *const Input).cast_mut().cast(),
                    };
                    AudioUnitSetProperty(
                        unit,
                        PROP_RENDER_CALLBACK,
                        SCOPE_INPUT,
                        0,
                        (&cb as *const RenderCallbackStruct).cast(),
                        std::mem::size_of::<RenderCallbackStruct>() as u32,
                    );
                }
                if AudioUnitInitialize(unit) != 0 {
                    return Err(format!("{}: initialisieren fehlgeschlagen", info.name));
                }
            }
            me.initialized = true;
            me.read_params();
            me.read_latency(sample_rate);
            Ok(me)
        }

        fn read_params(&mut self) {
            let u = self.unit;
            let mut size = 0u32;
            // SAFETY: Instanz lebt; Größen und Puffer passen zusammen.
            unsafe {
                if AudioUnitGetPropertyInfo(
                    u,
                    PROP_PARAMETER_LIST,
                    SCOPE_GLOBAL,
                    0,
                    &mut size,
                    ptr::null_mut(),
                ) != 0
                {
                    return;
                }
                let n = (size as usize / 4).min(100_000);
                let mut ids = vec![0u32; n];
                let mut size = (n * 4) as u32;
                if AudioUnitGetProperty(
                    u,
                    PROP_PARAMETER_LIST,
                    SCOPE_GLOBAL,
                    0,
                    ids.as_mut_ptr().cast(),
                    &mut size,
                ) != 0
                {
                    return;
                }
                ids.truncate(size as usize / 4);
                for id in ids {
                    let mut p: ParameterInfo = std::mem::zeroed();
                    let mut size = std::mem::size_of::<ParameterInfo>() as u32;
                    if AudioUnitGetProperty(
                        u,
                        PROP_PARAMETER_INFO,
                        SCOPE_GLOBAL,
                        id,
                        (&mut p as *mut ParameterInfo).cast(),
                        &mut size,
                    ) != 0
                    {
                        continue;
                    }
                    let name = if p.flags & PARAM_FLAG_HAS_CF_NAME != 0 && !p.cf_name.is_null() {
                        let s = cf_string(p.cf_name);
                        if p.flags & PARAM_FLAG_CF_NAME_RELEASE != 0 {
                            CFRelease(p.cf_name);
                        }
                        s
                    } else {
                        let bytes: Vec<u8> = p
                            .name
                            .iter()
                            .take_while(|&&c| c != 0)
                            .map(|&c| c as u8)
                            .collect();
                        String::from_utf8_lossy(&bytes).into_owned()
                    };
                    if p.flags & PARAM_FLAG_WRITABLE == 0 {
                        continue;
                    }
                    let (min, max) = if p.min.is_finite() && p.max.is_finite() && p.max > p.min {
                        (f64::from(p.min), f64::from(p.max))
                    } else {
                        (0.0, 1.0)
                    };
                    let def = if p.default.is_finite() {
                        f64::from(p.default).clamp(min, max)
                    } else {
                        min
                    };
                    self.params.push(ParamInfo {
                        id,
                        name,
                        module: String::new(),
                        min,
                        max,
                        default: def,
                        value: def,
                        stepped: p.unit == UNIT_INDEXED || p.unit == UNIT_BOOLEAN,
                    });
                }
            }
        }

        fn read_latency(&self, rate: u32) {
            let mut secs = 0f64;
            let mut size = 8u32;
            // SAFETY: Instanz lebt; Ausgabe ist ein f64.
            let ok = unsafe {
                AudioUnitGetProperty(
                    self.unit,
                    PROP_LATENCY,
                    SCOPE_GLOBAL,
                    0,
                    (&mut secs as *mut f64).cast(),
                    &mut size,
                ) == 0
            };
            if ok && secs.is_finite() && secs > 0.0 {
                let frames = (secs * f64::from(rate)).round().min(f64::from(MAX_LATENCY)) as u32;
                self.shared.latency.store(frames, Ordering::Release);
            }
        }

        pub fn handle(&self) -> AuHandle {
            AuHandle {
                unit: self.unit,
                shared: self.shared.clone(),
                gui: None,
            }
        }

        pub fn set_param(&mut self, id: u32, value: f64) {
            self.set_param_at(0, id, value);
        }

        pub fn set_param_at(&mut self, time: u32, id: u32, value: f64) {
            if value.is_finite() {
                // SAFETY: Instanz lebt; Parameter setzen ist aus dem Audio-Thread erlaubt.
                unsafe {
                    AudioUnitSetParameter(self.unit, id, SCOPE_GLOBAL, 0, value as f32, time)
                };
            }
        }

        pub fn midi(&mut self, time: u32, data: [u8; 3]) {
            if self.info.is_instrument() {
                // SAFETY: Instanz lebt; MIDI aus dem Audio-Thread ist vorgesehen.
                unsafe {
                    MusicDeviceMIDIEvent(
                        self.unit,
                        u32::from(data[0]),
                        u32::from(data[1]),
                        u32::from(data[2]),
                        time,
                    )
                };
            }
        }

        pub fn note(&mut self, time: u32, key: u8, velocity: f32, on: bool) {
            let vel = (velocity.clamp(0.0, 1.0) * 127.0).round() as u8;
            if on {
                self.midi(time, [0x90, key.min(127), vel.max(1)]);
            } else {
                self.midi(time, [0x80, key.min(127), vel]);
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
                self.input.left[..n].copy_from_slice(l);
                self.input.right[..n].copy_from_slice(r);
                let mut list = BufferList {
                    count: self.out_channels,
                    buffers: [
                        Buffer {
                            channels: 1,
                            byte_size: (n * 4) as u32,
                            data: self.out_left.as_mut_ptr().cast(),
                        },
                        Buffer {
                            channels: 1,
                            byte_size: (n * 4) as u32,
                            data: self.out_right.as_mut_ptr().cast(),
                        },
                    ],
                };
                let time = TimeStamp {
                    sample_time: self.sample_time,
                    flags: TIME_SAMPLE_VALID,
                    ..Default::default()
                };
                let mut flags = 0u32;
                // SAFETY: Instanz initialisiert; Puffer haben n Plätze; Audio-Thread.
                let status = unsafe {
                    AudioUnitRender(self.unit, &mut flags, &time, 0, n as u32, &mut list)
                };
                if status == 0 {
                    // Das Plugin darf eigene Puffer liefern: dann von dort kopieren.
                    let read = |b: &Buffer, own: &[f32], dst: &mut [f32]| {
                        if b.data.is_null() {
                            dst.fill(0.0);
                        } else if b.data.cast::<f32>() == own.as_ptr().cast_mut() {
                            dst.copy_from_slice(&own[..dst.len()]);
                        } else {
                            // SAFETY: Das Plugin liefert mindestens n Werte (laut byte_size geprüft).
                            let avail = (b.byte_size as usize / 4).min(dst.len());
                            let src =
                                unsafe { std::slice::from_raw_parts(b.data.cast::<f32>(), avail) };
                            dst[..avail].copy_from_slice(src);
                        }
                    };
                    read(&list.buffers[0], &self.out_left, l);
                    if self.out_channels >= 2 {
                        read(&list.buffers[1], &self.out_right, r);
                    } else {
                        r.copy_from_slice(l);
                    }
                } else {
                    l.fill(0.0);
                    r.fill(0.0);
                }
                for v in l.iter_mut().chain(r.iter_mut()) {
                    if !v.is_finite() {
                        *v = 0.0;
                    }
                }
                self.sample_time += n as f64;
                done += n;
            }
        }

        pub fn stop_processing(&mut self) {}
    }

    impl Drop for AuInstance {
        fn drop(&mut self) {
            if self.unit.is_null() {
                return;
            }
            let _main = main_thread_lock();
            // SAFETY: Instanz lebt; danach wird sie nicht mehr benutzt.
            unsafe {
                if self.initialized {
                    AudioUnitUninitialize(self.unit);
                }
                AudioComponentInstanceDispose(self.unit);
            }
            self.unit = ptr::null_mut();
        }
    }

    /// Griff für den Hauptthread (Zustand, Latenz). Muss vor der Instanz freigegeben werden.
    pub struct AuHandle {
        unit: Unit,
        shared: Arc<Shared>,
        gui: Option<Box<window::AuGui>>,
    }

    impl Drop for AuHandle {
        fn drop(&mut self) {
            // Fenster vor der Instanz schließen (Vertrag: Griff stirbt zuerst).
            self.close_gui();
        }
    }

    // SAFETY: Zustand lesen ist parallel zum Rendern erlaubt; der Griff lebt kürzer als die Instanz.
    unsafe impl Send for AuHandle {}

    impl AuHandle {
        /// Jede Audio Unit hat ein Fenster: ihre eigene Oberfläche oder Apples allgemeine
        /// Parameteransicht.
        pub fn has_gui(&self) -> bool {
            window::available()
        }

        pub fn gui_open(&self) -> bool {
            self.gui.is_some()
        }

        /// Öffnet das Fenster der Audio Unit. Nur im Hauptthread.
        pub fn open_gui(&mut self, title: &str) -> Result<(), String> {
            if self.gui.is_some() {
                return Ok(());
            }
            let _main = main_thread_lock();
            let gui = window::AuGui::open(self.unit, title)?;
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
            let open = self.gui.as_ref().is_some_and(|g| g.is_open());
            if !open {
                self.close_gui();
            }
            open
        }

        pub fn latency(&self) -> u32 {
            self.shared.latency.load(Ordering::Acquire)
        }

        pub fn take_latency_changed(&self) -> bool {
            false
        }

        /// Klassen-Info (Property-List) als binäre Property-List. Hauptthread.
        pub fn save_state(&self) -> Option<Vec<u8>> {
            let _main = main_thread_lock();
            let mut plist: CfRef = ptr::null();
            let mut size = std::mem::size_of::<CfRef>() as u32;
            // SAFETY: Instanz lebt; die gelieferte Property-List gehört uns und wird freigegeben.
            unsafe {
                if AudioUnitGetProperty(
                    self.unit,
                    PROP_CLASS_INFO,
                    SCOPE_GLOBAL,
                    0,
                    (&mut plist as *mut CfRef).cast(),
                    &mut size,
                ) != 0
                    || plist.is_null()
                {
                    return None;
                }
                let data =
                    CFPropertyListCreateData(ptr::null(), plist, PLIST_BINARY, 0, ptr::null_mut());
                CFRelease(plist);
                if data.is_null() {
                    return None;
                }
                let len = CFDataGetLength(data);
                let out = if (0..=MAX_STATE).contains(&len) {
                    std::slice::from_raw_parts(CFDataGetBytePtr(data), len as usize).to_vec()
                } else {
                    Vec::new()
                };
                CFRelease(data);
                (!out.is_empty()).then_some(out)
            }
        }

        /// Gespeicherten Zustand laden (beim Erzeugen, bevor die Instanz rechnet).
        pub fn load_state(&self, bytes: &[u8]) -> bool {
            if bytes.is_empty() || bytes.len() as isize > MAX_STATE {
                return false;
            }
            let _main = main_thread_lock();
            // SAFETY: Daten werden kopiert; alle angelegten CF-Objekte werden freigegeben.
            unsafe {
                let data = CFDataCreate(ptr::null(), bytes.as_ptr(), bytes.len() as isize);
                if data.is_null() {
                    return false;
                }
                let plist = CFPropertyListCreateWithData(
                    ptr::null(),
                    data,
                    0,
                    ptr::null_mut(),
                    ptr::null_mut(),
                );
                CFRelease(data);
                if plist.is_null() {
                    return false;
                }
                let r = AudioUnitSetProperty(
                    self.unit,
                    PROP_CLASS_INFO,
                    SCOPE_GLOBAL,
                    0,
                    (&plist as *const CfRef).cast(),
                    std::mem::size_of::<CfRef>() as u32,
                );
                CFRelease(plist);
                r == 0
            }
        }
    }

    /// Fenster einer Audio Unit: ihre eigene Cocoa-Oberfläche (falls sie eine mitbringt), sonst
    /// Apples allgemeine Parameteransicht, eingebettet in das Cocoa-Host-Fenster.
    mod window {
        use std::ffi::c_void;

        use objc2::{
            rc::Retained,
            runtime::{AnyClass, AnyObject, Sel},
            sel,
        };
        use objc2_app_kit::NSView;
        use objc2_foundation::{NSBundle, NSSize, NSString, NSURL};

        use super::{
            AudioUnitGetProperty, AudioUnitGetPropertyInfo, CFRelease, CfRef, SCOPE_GLOBAL, Unit,
        };
        use crate::gui::HostWindow;

        // Die allgemeine Ansicht liegt in CoreAudioKit.
        #[link(name = "CoreAudioKit", kind = "framework")]
        unsafe extern "C" {}

        /// Eigenschaft „Cocoa-Oberfläche“: Bundle-URL und Klassennamen der Ansichtsfabrik.
        const PROP_COCOA_UI: u32 = 31;

        type Id = *mut AnyObject;

        /// `objc_msgSend` ohne Argumente.
        ///
        /// # Safety
        /// `obj` muss ein Objekt oder eine Klasse sein, die `sel` versteht und ein Objekt liefert.
        unsafe fn send0(obj: *const c_void, sel: Sel) -> Id {
            // SAFETY: objc_msgSend mit der Signatur der aufgerufenen Methode (keine Argumente).
            unsafe {
                let f: unsafe extern "C-unwind" fn(*const c_void, Sel) -> Id =
                    std::mem::transmute(objc2::ffi::objc_msgSend as unsafe extern "C-unwind" fn());
                f(obj, sel)
            }
        }

        /// `objc_msgSend` mit einem Zeiger-Argument.
        ///
        /// # Safety
        /// Wie [`send0`], die Methode nimmt einen Zeiger.
        unsafe fn send_ptr(obj: *const c_void, sel: Sel, p: *mut c_void) -> Id {
            // SAFETY: wie oben, Signatur (Zeiger) -> Objekt.
            unsafe {
                let f: unsafe extern "C-unwind" fn(*const c_void, Sel, *mut c_void) -> Id =
                    std::mem::transmute(objc2::ffi::objc_msgSend as unsafe extern "C-unwind" fn());
                f(obj, sel, p)
            }
        }

        /// `objc_msgSend` mit Zeiger und Größe (`uiViewForAudioUnit:withSize:`).
        ///
        /// # Safety
        /// Wie [`send0`], die Methode nimmt einen Zeiger und eine NSSize.
        unsafe fn send_ptr_size(obj: *const c_void, sel: Sel, p: *mut c_void, size: NSSize) -> Id {
            // SAFETY: wie oben, Signatur (Zeiger, NSSize) -> Objekt.
            unsafe {
                let f: unsafe extern "C-unwind" fn(*const c_void, Sel, *mut c_void, NSSize) -> Id =
                    std::mem::transmute(objc2::ffi::objc_msgSend as unsafe extern "C-unwind" fn());
                f(obj, sel, p, size)
            }
        }

        /// Gibt es die allgemeine Ansicht (CoreAudioKit geladen)?
        pub(super) fn available() -> bool {
            AnyClass::get(c"AUGenericView").is_some()
        }

        /// Eigene Oberfläche der Audio Unit, falls sie eine mitbringt.
        fn cocoa_view(unit: Unit) -> Option<Retained<NSView>> {
            let mut size = 0u32;
            // SAFETY: Instanz lebt; Puffer passend zur gemeldeten Größe; alle gelieferten
            // CF-Objekte werden freigegeben.
            unsafe {
                if AudioUnitGetPropertyInfo(
                    unit,
                    PROP_COCOA_UI,
                    SCOPE_GLOBAL,
                    0,
                    &mut size,
                    std::ptr::null_mut(),
                ) != 0
                {
                    return None;
                }
                let n = size as usize / std::mem::size_of::<CfRef>();
                if n < 2 {
                    return None;
                }
                let mut buf: Vec<CfRef> = vec![std::ptr::null(); n];
                let mut size = (n * std::mem::size_of::<CfRef>()) as u32;
                if AudioUnitGetProperty(
                    unit,
                    PROP_COCOA_UI,
                    SCOPE_GLOBAL,
                    0,
                    buf.as_mut_ptr().cast(),
                    &mut size,
                ) != 0
                {
                    return None;
                }
                let view = (|| {
                    let url = (buf[0] as *const NSURL).as_ref()?;
                    let name = (buf[1] as *const NSString).as_ref()?;
                    let bundle = NSBundle::bundleWithURL(url)?;
                    let class = bundle.classNamed(name)?;
                    let factory = send0((class as *const AnyClass).cast(), sel!(new));
                    if factory.is_null() {
                        return None;
                    }
                    let view = send_ptr_size(
                        factory.cast(),
                        sel!(uiViewForAudioUnit:withSize:),
                        unit,
                        NSSize::new(400.0, 300.0),
                    );
                    objc2::ffi::objc_release(factory);
                    // Die Fabrik liefert die Ansicht ohne eigene Referenz: festhalten.
                    Retained::retain(view.cast::<NSView>())
                })();
                for &cf in buf.iter().take(n) {
                    if !cf.is_null() {
                        CFRelease(cf);
                    }
                }
                view
            }
        }

        /// Apples allgemeine Parameteransicht für jede Audio Unit.
        fn generic_view(unit: Unit) -> Option<Retained<NSView>> {
            let class = AnyClass::get(c"AUGenericView")?;
            // SAFETY: alloc/initWithAudioUnit: liefern ein Objekt mit einer Referenz (oder null).
            unsafe {
                let obj = send0((class as *const AnyClass).cast(), sel!(alloc));
                if obj.is_null() {
                    return None;
                }
                let view = send_ptr(obj.cast(), sel!(initWithAudioUnit:), unit);
                Retained::from_raw(view.cast::<NSView>())
            }
        }

        pub(super) struct AuGui {
            window: HostWindow,
            view: Retained<NSView>,
        }

        impl AuGui {
            pub(super) fn open(unit: Unit, title: &str) -> Result<Self, String> {
                let view = cocoa_view(unit)
                    .or_else(|| generic_view(unit))
                    .ok_or("Audio Unit hat kein Fenster")?;
                let frame = view.frame();
                let (w, h) = (
                    frame.size.width.clamp(200.0, 4000.0) as u32,
                    frame.size.height.clamp(100.0, 4000.0) as u32,
                );
                let window = HostWindow::new(title, w, h)?;
                let content = window.parent().cast::<NSView>();
                // SAFETY: Inhaltsansicht des lebenden Fensters (das Fenster hält sie).
                let Some(content) = (unsafe { content.as_ref() }) else {
                    window.close();
                    return Err("Fenster ohne Inhaltsansicht".into());
                };
                content.addSubview(&view);
                window.show();
                Ok(Self { window, view })
            }

            pub(super) fn is_open(&self) -> bool {
                !self.window.pump_closed()
            }

            pub(super) fn close(self) {
                self.view.removeFromSuperview();
                self.window.close();
            }
        }
    }

    #[cfg(test)]
    mod tests {
        use super::*;

        fn apple(sub: &str) -> Option<PluginInfo> {
            scan()
                .into_iter()
                .find(|p| p.id == format!("aufx:{sub}:appl"))
        }

        #[test]
        fn struct_layouts_match_the_system() {
            assert_eq!(std::mem::size_of::<StreamFormat>(), 40);
            assert_eq!(std::mem::size_of::<TimeStamp>(), 64);
            assert_eq!(std::mem::size_of::<ParameterInfo>(), 104);
            assert_eq!(std::mem::size_of::<Buffer>(), 16);
        }

        #[test]
        fn apple_effects_are_found_and_process_audio() {
            let list = scan();
            assert!(!list.is_empty(), "macOS ships Audio Units");
            let delay = apple("dely");
            assert!(delay.is_some(), "AUDelay is installed on every Mac");
            let Some(info) = delay else { return };
            assert_eq!(info.vendor, "Apple");
            let inst = AuInstance::new(&info, 48_000, 256);
            assert!(inst.is_ok(), "{:?}", inst.as_ref().err());
            let Ok(mut inst) = inst else { return };
            assert!(!inst.params.is_empty(), "AUDelay has parameters");
            let mut peak = 0f32;
            for _ in 0..40 {
                let (mut l, mut r) = (vec![0.25f32; 256], vec![0.25f32; 256]);
                inst.process(&mut l, &mut r);
                peak = l.iter().chain(&r).fold(peak, |m, v| m.max(v.abs()));
                assert!(l.iter().chain(&r).all(|v| v.is_finite()));
            }
            assert!(peak > 1e-3, "AUDelay passes audio: {peak}");
            let p = inst.params[0].clone();
            inst.set_param(p.id, p.max);
            let (mut l, mut r) = (vec![0.0f32; 300], vec![0.0f32; 300]);
            inst.process(&mut l, &mut r);
        }

        #[test]
        fn state_survives_a_new_instance() {
            let Some(info) = apple("dely") else { return };
            let Ok(mut a) = AuInstance::new(&info, 48_000, 256) else {
                return;
            };
            let p = a.params[0].clone();
            a.set_param(p.id, (p.min + p.max) / 2.0);
            let state = a.handle().save_state();
            assert!(state.is_some(), "class info can be saved");
            let Some(state) = state else { return };
            let Ok(b) = AuInstance::new(&info, 48_000, 256) else {
                return;
            };
            assert!(b.handle().load_state(&state));
            assert!(!b.handle().load_state(b"kein plist"));
        }

        #[test]
        fn the_apple_synth_plays_a_note() {
            let Some(info) = scan()
                .into_iter()
                .find(|p| p.is_instrument() && p.vendor == "Apple")
            else {
                return;
            };
            let Ok(mut inst) = AuInstance::new(&info, 48_000, 256) else {
                return;
            };
            let mut peak = 0f32;
            for b in 0..80 {
                if b == 1 {
                    inst.note(0, 60, 0.9, true);
                }
                let (mut l, mut r) = (vec![0f32; 256], vec![0f32; 256]);
                inst.process(&mut l, &mut r);
                peak = l.iter().chain(&r).fold(peak, |m, v| m.max(v.abs()));
            }
            assert!(peak > 1e-4, "{} sounds: {peak}", info.name);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ids_roundtrip_as_four_char_codes() {
        assert_eq!(fourcc(u32::from_be_bytes(*b"aufx")), "aufx");
        assert_eq!(from_fourcc("dely"), Some(u32::from_be_bytes(*b"dely")));
        assert_eq!(from_fourcc("toolong"), None);
        assert_eq!(
            parse_id("aufx:dely:appl"),
            Some((
                u32::from_be_bytes(*b"aufx"),
                u32::from_be_bytes(*b"dely"),
                u32::from_be_bytes(*b"appl")
            ))
        );
        assert_eq!(parse_id("aufx:dely"), None);
        assert_eq!(parse_id("aufx:dely:appl:x"), None);
    }
}
