//! LV2-Plugin-Fenster (Issue #33): Oberflächen vom Typ X11UI, eingebettet in ein eigenes
//! Host-Fenster über `ui:parent`. Andere Toolkits (GTK, Qt) bräuchten eine Brücke und folgen
//! später. Alles im Hauptthread.
//!
//! Die Oberfläche meldet Reglerbewegungen über ihre Schreibfunktion; der Host holt sie mit
//! [`Lv2Gui::take_param_changes`] ab und setzt sie wie andere Parameteränderungen. Umgekehrt
//! gibt [`Lv2Gui::set_value`] Werte vom Host an die Oberfläche weiter.

use std::{
    collections::HashMap,
    ffi::{CStr, CString, c_char, c_void},
    path::{Path, PathBuf},
    ptr,
    sync::{Mutex, OnceLock},
};

#[cfg(all(unix, not(target_os = "macos")))]
use crate::gui::x11win::HostWindow;

/// Ohne X11 (Windows, macOS): kein eingebettetes LV2-Fenster.
#[cfg(not(all(unix, not(target_os = "macos"))))]
struct HostWindow;

#[cfg(not(all(unix, not(target_os = "macos"))))]
impl HostWindow {
    fn new(_: &str, _: u32, _: u32) -> Result<Self, crate::error::PluginError> {
        Err(crate::error::PluginError::Window(
            crate::error::WindowProblem::Lv2OnlyX11,
        ))
    }
    fn id(&self) -> u32 {
        0
    }
    fn show(&self) {}
    fn resize(&self, _: u32, _: u32) {}
    fn pump_closed(&self) -> bool {
        true
    }
    fn close(self) {}
}

/// `LV2UI_Descriptor` (lv2plug.in, ui.h).
#[repr(C)]
struct UiDescriptor {
    uri: *const c_char,
    instantiate: Option<
        unsafe extern "C" fn(
            *const UiDescriptor,
            *const c_char,
            *const c_char,
            WriteFunction,
            *mut c_void,
            *mut *mut c_void,
            *const *const super::lv2::Lv2Feature,
        ) -> *mut c_void,
    >,
    cleanup: Option<unsafe extern "C" fn(*mut c_void)>,
    port_event: Option<unsafe extern "C" fn(*mut c_void, u32, u32, u32, *const c_void)>,
    extension_data: Option<unsafe extern "C" fn(*const c_char) -> *const c_void>,
}

type WriteFunction = unsafe extern "C" fn(*mut c_void, u32, u32, u32, *const c_void);

#[repr(C)]
struct UiResize {
    handle: *mut c_void,
    ui_resize: unsafe extern "C" fn(*mut c_void, i32, i32) -> i32,
}

#[repr(C)]
struct IdleInterface {
    idle: Option<unsafe extern "C" fn(*mut c_void) -> i32>,
}

#[repr(C)]
struct DataAccess {
    data_access: Option<unsafe extern "C" fn(*const c_char) -> *const c_void>,
}

const FEATURE_PARENT: &CStr = c"http://lv2plug.in/ns/extensions/ui#parent";
const FEATURE_RESIZE: &CStr = c"http://lv2plug.in/ns/extensions/ui#resize";
const FEATURE_IDLE: &CStr = c"http://lv2plug.in/ns/extensions/ui#idleInterface";
const FEATURE_INSTANCE: &CStr = c"http://lv2plug.in/ns/ext/instance-access";
const FEATURE_DATA: &CStr = c"http://lv2plug.in/ns/ext/data-access";
const IDLE_INTERFACE: &CStr = c"http://lv2plug.in/ns/extensions/ui#idleInterface";

/// Was der Oberfläche über Rückrufe mitgeteilt wird; lebt in einer Box, die Oberfläche
/// bekommt einen Zeiger darauf.
#[derive(Default)]
struct Controller {
    changes: Mutex<Vec<(u32, f64)>>,
    resize: Mutex<Option<(u32, u32)>>,
}

unsafe extern "C" fn write_port(
    controller: *mut c_void,
    port: u32,
    size: u32,
    protocol: u32,
    buffer: *const c_void,
) {
    // Nur das Float-Protokoll (0): ein Control-Eingang, ein f32.
    if controller.is_null() || buffer.is_null() || protocol != 0 || size != 4 {
        return;
    }
    // SAFETY: `controller` ist unser Box-Zeiger, er lebt so lange wie die Oberfläche;
    // `buffer` zeigt laut LV2 auf ein f32.
    let (c, v) = unsafe { (&*(controller as *const Controller), *(buffer as *const f32)) };
    if v.is_finite()
        && let Ok(mut list) = c.changes.lock()
    {
        list.push((port, f64::from(v)));
    }
}

unsafe extern "C" fn request_resize(handle: *mut c_void, w: i32, h: i32) -> i32 {
    if handle.is_null() || w <= 0 || h <= 0 {
        return 1;
    }
    // SAFETY: `handle` ist unser Controller (siehe `open`).
    let c = unsafe { &*(handle as *const Controller) };
    if let Ok(mut r) = c.resize.lock() {
        *r = Some((w as u32, h as u32));
    }
    0
}

/// Die Oberfläche eines Plugins laut seinen Beschreibungen.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct UiInfo {
    pub uri: String,
    pub binary: PathBuf,
    pub bundle: PathBuf,
}

/// UI-Bibliotheken bleiben bis zum Programmende geladen: Entladen, während Toolkit-Threads
/// oder -Rückrufe der Bibliothek noch leben, stürzt ab.
fn load_library(path: &Path) -> Result<&'static libloading::Library, crate::error::PluginError> {
    static LIBS: OnceLock<Mutex<HashMap<PathBuf, &'static libloading::Library>>> = OnceLock::new();
    let mut libs = LIBS
        .get_or_init(Default::default)
        .lock()
        .map_err(|_| crate::error::PluginError::Locked(crate::error::Lock::UiLibraries))?;
    if let Some(lib) = libs.get(path) {
        return Ok(lib);
    }
    // SAFETY: Laden fremden Codes ist der Zweck; Initialisierer der Bibliothek laufen hier.
    let lib = unsafe { libloading::Library::new(path) }
        .map_err(|e| crate::error::PluginError::Library(e.to_string()))?;
    let lib: &'static libloading::Library = Box::leak(Box::new(lib));
    libs.insert(path.to_path_buf(), lib);
    Ok(lib)
}

/// Ein offenes LV2-Plugin-Fenster.
pub(crate) struct Lv2Gui {
    desc: *const UiDescriptor,
    ui: *mut c_void,
    window: HostWindow,
    idle: Option<unsafe extern "C" fn(*mut c_void) -> i32>,
    controller: Box<Controller>,
    // Features leben mit der Oberfläche.
    _resize: Box<UiResize>,
    _data: Box<DataAccess>,
    _features: Box<[super::lv2::Lv2Feature]>,
    _feature_ptrs: Box<[*const super::lv2::Lv2Feature]>,
}

impl Lv2Gui {
    /// Lädt die Oberfläche `info` für das Plugin `plugin_uri` und bettet sie in ein neues
    /// Fenster ein. `plugin_handle` und `extension_data` stammen von der laufenden Instanz.
    pub(crate) fn open(
        info: &UiInfo,
        plugin_uri: &str,
        title: &str,
        plugin_handle: *mut c_void,
        extension_data: Option<unsafe extern "C" fn(*const c_char) -> *const c_void>,
        base_features: &[*const super::lv2::Lv2Feature],
    ) -> Result<Self, crate::error::PluginError> {
        let lib = load_library(&info.binary)?;
        // SAFETY: `lv2ui_descriptor` ist laut LV2 eine Funktion (Index) → Beschreibung.
        let get: libloading::Symbol<unsafe extern "C" fn(u32) -> *const UiDescriptor> = unsafe {
            lib.get(b"lv2ui_descriptor\0")
        }
        .map_err(|_| crate::error::PluginError::Window(crate::error::WindowProblem::Lv2NoUi))?;
        let mut desc = ptr::null();
        for i in 0..256 {
            // SAFETY: Aufruf laut Vertrag; null beendet die Liste.
            let d = unsafe { get(i) };
            if d.is_null() {
                break;
            }
            // SAFETY: Beschreibung gültig, URI nullterminiert.
            if unsafe { CStr::from_ptr((*d).uri) }.to_string_lossy() == info.uri {
                desc = d;
                break;
            }
        }
        if desc.is_null() {
            return Err(crate::error::PluginError::Window(
                crate::error::WindowProblem::Lv2UiNotInLibrary,
            ));
        }
        let window = HostWindow::new(title, 640, 480)?;
        let mut controller = Box::<Controller>::default();
        let ctl_ptr: *mut c_void = (&mut *controller as *mut Controller).cast();
        let mut resize = Box::new(UiResize {
            handle: ctl_ptr,
            ui_resize: request_resize,
        });
        let mut data = Box::new(DataAccess {
            data_access: extension_data,
        });
        let feature = |uri: &CStr, data: *mut c_void| super::lv2::Lv2Feature {
            uri: uri.as_ptr(),
            data,
        };
        let features: Box<[super::lv2::Lv2Feature]> = Box::new([
            feature(FEATURE_PARENT, window.id() as usize as *mut c_void),
            feature(FEATURE_RESIZE, (&mut *resize as *mut UiResize).cast()),
            feature(FEATURE_IDLE, ptr::null_mut()),
            feature(FEATURE_INSTANCE, plugin_handle),
            feature(FEATURE_DATA, (&mut *data as *mut DataAccess).cast()),
        ]);
        let feature_ptrs: Box<[*const super::lv2::Lv2Feature]> = base_features
            .iter()
            .copied()
            .filter(|p| !p.is_null())
            .chain(features.iter().map(|f| f as *const super::lv2::Lv2Feature))
            .chain(std::iter::once(ptr::null()))
            .collect();
        let uri = CString::new(plugin_uri)
            .map_err(|e| crate::error::PluginError::Library(e.to_string()))?;
        let mut bundle = info.bundle.to_string_lossy().into_owned();
        if !bundle.ends_with('/') {
            bundle.push('/');
        }
        let bundle =
            CString::new(bundle).map_err(|e| crate::error::PluginError::Library(e.to_string()))?;
        let mut widget: *mut c_void = ptr::null_mut();
        // SAFETY: Beschreibung gültig; alle Zeiger leben mindestens so lange wie die Oberfläche
        // (Felder dieser Struktur); Hauptthread.
        let ui = unsafe {
            match (*desc).instantiate {
                Some(f) => f(
                    desc,
                    uri.as_ptr(),
                    bundle.as_ptr(),
                    write_port,
                    ctl_ptr,
                    &mut widget,
                    feature_ptrs.as_ptr(),
                ),
                None => ptr::null_mut(),
            }
        };
        if ui.is_null() {
            window.close();
            return Err(crate::error::PluginError::Window(
                crate::error::WindowProblem::Lv2UiCreate,
            ));
        }
        // SAFETY: Beschreibung gültig; extension_data ist ein reiner Abruf.
        let idle = unsafe {
            (*desc)
                .extension_data
                .map(|f| f(IDLE_INTERFACE.as_ptr()) as *const IdleInterface)
                .and_then(|p| p.as_ref())
                .and_then(|i| i.idle)
        };
        if let Ok(Some((w, h))) = controller.resize.lock().map(|mut r| r.take()) {
            window.resize(w, h);
        }
        window.show();
        Ok(Self {
            desc,
            ui,
            window,
            idle,
            controller,
            _resize: resize,
            _data: data,
            _features: features,
            _feature_ptrs: feature_ptrs,
        })
    }

    /// Wert eines Control-Ports an die Oberfläche geben.
    pub(crate) fn set_value(&self, port: u32, value: f64) {
        let v = value as f32;
        // SAFETY: Oberfläche lebt; Float-Protokoll (0) mit einem f32; Hauptthread.
        unsafe {
            if let Some(f) = (*self.desc).port_event {
                f(self.ui, port, 4, 0, (&v as *const f32).cast());
            }
        }
    }

    /// Reglerbewegungen in der Oberfläche seit dem letzten Aufruf: (Port, Wert).
    pub(crate) fn take_param_changes(&self) -> Vec<(u32, f64)> {
        self.controller
            .changes
            .lock()
            .map(|mut l| std::mem::take(&mut *l))
            .unwrap_or_default()
    }

    /// Im UI-Takt: Oberfläche bedienen, Größenwünsche umsetzen. `false`, wenn das Fenster
    /// geschlossen wurde (dann [`Self::close`] aufrufen).
    pub(crate) fn idle(&mut self) -> bool {
        if let Some(f) = self.idle {
            // SAFETY: Oberfläche lebt; Hauptthread.
            if unsafe { f(self.ui) } != 0 {
                return false;
            }
        }
        if let Ok(Some((w, h))) = self.controller.resize.lock().map(|mut r| r.take()) {
            self.window.resize(w, h);
        }
        !self.window.pump_closed()
    }

    pub(crate) fn close(self) {
        // SAFETY: Oberfläche lebt und wird genau einmal aufgeräumt; danach das Fenster.
        unsafe {
            if let Some(f) = (*self.desc).cleanup {
                f(self.ui);
            }
        }
        self.window.close();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn write_function_takes_only_float_values() {
        let c = Controller::default();
        let p: *mut c_void = (&c as *const Controller).cast_mut().cast();
        let v = 0.5f32;
        let nan = f32::NAN;
        // SAFETY: Test ruft die Rückrufe wie eine Oberfläche auf.
        unsafe {
            write_port(p, 3, 4, 0, (&v as *const f32).cast());
            write_port(p, 4, 4, 7, (&v as *const f32).cast());
            write_port(p, 5, 8, 0, (&v as *const f32).cast());
            write_port(p, 6, 4, 0, (&nan as *const f32).cast());
            write_port(ptr::null_mut(), 7, 4, 0, (&v as *const f32).cast());
            assert_eq!(request_resize(p, 300, 200), 0);
            assert_eq!(request_resize(p, -1, 200), 1);
        }
        assert_eq!(
            c.changes.lock().map(|l| l.clone()).unwrap_or_default(),
            vec![(3, 0.5)]
        );
        assert_eq!(
            c.resize.lock().map(|r| *r).unwrap_or_default(),
            Some((300, 200))
        );
    }
}
