//! Plugin-Fenster: bevorzugt schwebend (das Plugin öffnet sein eigenes
//! Fenster, das klappt auf allen Systemen), sonst eingebettet in ein eigenes
//! Host-Fenster: X11 (Linux, BSD; unter Wayland über XWayland), Win32 (Windows)
//! oder Cocoa (macOS).
//!
//! Unter Wayland (Issue #175) kommt zuerst ein schwebendes Wayland-Fenster des Plugins, dann
//! das eingebettete Fenster über XWayland (es bleibt über dem Hauptfenster und merkt sich
//! seine Position), zuletzt ein schwebendes X11-Fenster des Plugins. Unter X11 bleibt es bei
//! schwebend vor eingebettet; beide hängen dort am Hauptfenster (`WM_TRANSIENT_FOR`,
//! `clap.gui.set_transient`).

use std::{
    ffi::{CStr, CString},
    sync::atomic::Ordering,
    time::Instant,
};

use clap_sys::ext::{
    gui::{CLAP_EXT_GUI, clap_plugin_gui, clap_window, clap_window_handle},
    timer_support::{CLAP_EXT_TIMER_SUPPORT, clap_plugin_timer_support},
};

use crate::{clap::ClapHandle, host::on_main_thread};

/// Das Hauptfenster der App, wie es das Fenstersystem kennt (siehe [`set_main_window`]).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MainWindow {
    /// Ein X11-Fenster mit dieser Id (X11-Sitzung oder App unter XWayland).
    X11(u32),
    /// Ein natives Wayland-Fenster.
    Wayland,
    /// Windows, macOS oder unbekannt.
    Other,
}

static MAIN_WINDOW: std::sync::Mutex<Option<MainWindow>> = std::sync::Mutex::new(None);

/// Meldet das Hauptfenster der App: Plugin-Fenster hängen unter X11 daran und wählen unter
/// Wayland ihren Weg danach. Einmal beim Start aufrufen.
pub fn set_main_window(window: MainWindow) {
    if let Ok(mut m) = MAIN_WINDOW.lock() {
        *m = Some(window);
    }
}

fn main_window() -> Option<MainWindow> {
    MAIN_WINDOW.lock().ok().and_then(|m| *m)
}

/// Das Hauptfenster als Text für einen Hilfsprozess (Plugin-Bridge); leer, wenn unbekannt.
pub(crate) fn main_window_code() -> String {
    match main_window() {
        Some(MainWindow::X11(id)) => format!("x11:{id}"),
        Some(MainWindow::Wayland) => "wayland".into(),
        Some(MainWindow::Other) => "other".into(),
        None => String::new(),
    }
}

/// Gegenstück zu [`main_window_code`] im Hilfsprozess; Unbekanntes bleibt ungesetzt.
pub(crate) fn set_main_window_code(code: &str) {
    let window = match code {
        "wayland" => MainWindow::Wayland,
        "other" => MainWindow::Other,
        c => match c.strip_prefix("x11:").and_then(|v| v.parse().ok()) {
            Some(id) => MainWindow::X11(id),
            None => return,
        },
    };
    set_main_window(window);
}

/// Läuft die App als Wayland-Client? Ohne Meldung der App entscheidet die Umgebung.
fn wayland_session() -> bool {
    match main_window() {
        Some(MainWindow::Wayland) => true,
        Some(_) => false,
        None => {
            cfg!(all(unix, not(target_os = "macos")))
                && std::env::var_os("WAYLAND_DISPLAY").is_some_and(|v| !v.is_empty())
        }
    }
}

/// Wie ein Plugin-Fenster aufgeht.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Mode {
    /// Das Plugin öffnet ein eigenes Fenster dieser Art.
    Floating(&'static CStr),
    /// Das Plugin zeichnet in unser Host-Fenster dieser Art.
    Embedded(&'static CStr),
}

/// Die Reihenfolge der Versuche; `supports(api, floating)` fragt das Plugin.
fn plan(wayland: bool, supports: impl Fn(&CStr, bool) -> bool) -> Option<Mode> {
    let api = window_api()?;
    let order: Vec<Mode> = if wayland {
        // Das eingebettete Fenster (XWayland) vor dem schwebenden X11-Fenster des Plugins: nur
        // unser Fenster bleibt über dem Hauptfenster und merkt sich seine Position.
        vec![
            Mode::Floating(clap_sys::ext::gui::CLAP_WINDOW_API_WAYLAND),
            Mode::Embedded(api),
            Mode::Floating(api),
        ]
    } else {
        vec![Mode::Floating(api), Mode::Embedded(api)]
    };
    order.into_iter().find(|m| match m {
        Mode::Floating(a) => supports(a, true),
        Mode::Embedded(a) => supports(a, false),
    })
}

fn window_api() -> Option<&'static CStr> {
    if cfg!(target_os = "macos") {
        Some(clap_sys::ext::gui::CLAP_WINDOW_API_COCOA)
    } else if cfg!(windows) {
        Some(clap_sys::ext::gui::CLAP_WINDOW_API_WIN32)
    } else if cfg!(unix) {
        Some(clap_sys::ext::gui::CLAP_WINDOW_API_X11)
    } else {
        None
    }
}

/// Zustand eines geöffneten Plugin-Fensters.
#[derive(Default)]
pub(crate) struct GuiState {
    created: bool,
    window: Option<HostWindow>,
}

impl ClapHandle {
    fn gui_ext(&self) -> Option<clap_plugin_gui> {
        // SAFETY: Plugin lebt (Vertrag von ClapHandle); Ergebnis null oder gültig.
        unsafe {
            (self.extension::<clap_plugin_gui>(CLAP_EXT_GUI))
                .as_ref()
                .copied()
        }
    }

    /// Ob das Plugin ein eigenes Fenster anbietet.
    pub fn has_gui(&self) -> bool {
        self.gui_ext().is_some()
    }

    pub fn gui_open(&self) -> bool {
        self.gui.created
    }

    /// Hat das Plugin seit dem letzten Aufruf geänderten Zustand gemeldet?
    pub fn take_dirty(&self) -> bool {
        self.shared.state_dirty.swap(false, Ordering::AcqRel)
    }

    /// Öffnet (oder zeigt) das Plugin-Fenster. Nur im Hauptthread.
    pub fn open_gui(&mut self, title: &str) -> Result<(), crate::error::PluginError> {
        if !on_main_thread() {
            return Err(crate::error::PluginError::Window(
                crate::error::WindowProblem::MainThreadOnly,
            ));
        }
        let vt = self.gui_ext().ok_or(crate::error::PluginError::Window(
            crate::error::WindowProblem::NoWindow,
        ))?;
        let p = self.plugin;
        if self.gui.created {
            if let Some(show) = vt.show {
                // SAFETY: GUI existiert; Hauptthread.
                unsafe { show(p) };
            }
            return Ok(());
        }
        let (Some(supported), Some(create)) = (vt.is_api_supported, vt.create) else {
            return Err(crate::error::PluginError::Window(
                crate::error::WindowProblem::IncompleteGui,
            ));
        };
        if window_api().is_none() {
            return Err(crate::error::PluginError::Window(
                crate::error::WindowProblem::UnsupportedSystem,
            ));
        }
        // SAFETY: Plugin lebt; `api` ist statisch und nullterminiert; Hauptthread.
        let mode = plan(wayland_session(), |api, floating| unsafe {
            supported(p, api.as_ptr(), floating)
        });
        let api = match mode {
            Some(Mode::Embedded(api)) => return self.open_embedded(vt, api, title),
            Some(Mode::Floating(api)) => api,
            None => {
                return Err(crate::error::PluginError::Window(
                    crate::error::WindowProblem::NoMatchingApi,
                ));
            }
        };
        self.shared.gui_closed.store(false, Ordering::Relaxed);
        // SAFETY: wie oben; noch keine GUI.
        if !unsafe { create(p, api.as_ptr(), true) } {
            return Err(crate::error::PluginError::Window(
                crate::error::WindowProblem::CannotCreate,
            ));
        }
        self.gui.created = true;
        if let (Some(f), Ok(t)) = (vt.suggest_title, CString::new(title)) {
            // SAFETY: GUI existiert; `t` lebt während des Aufrufs.
            unsafe { f(p, t.as_ptr()) };
        }
        self.set_transient(vt, api);
        // SAFETY: GUI existiert.
        if !vt.show.is_some_and(|f| unsafe { f(p) }) {
            self.close_gui();
            return Err(crate::error::PluginError::Window(
                crate::error::WindowProblem::CannotShow,
            ));
        }
        Ok(())
    }

    /// Ein schwebendes X11-Fenster des Plugins hängt am X11-Hauptfenster (bleibt darüber).
    /// Unter Wayland nicht möglich: das Plugin hat eine eigene Verbindung zum Compositor.
    #[cfg(all(unix, not(target_os = "macos")))]
    fn set_transient(&self, vt: clap_plugin_gui, api: &CStr) {
        let (Some(f), Some(MainWindow::X11(id))) = (vt.set_transient, main_window()) else {
            return;
        };
        if api != clap_sys::ext::gui::CLAP_WINDOW_API_X11 {
            return;
        }
        let parent = clap_window {
            api: api.as_ptr(),
            specific: clap_window_handle { x11: u64::from(id) },
        };
        // SAFETY: GUI existiert (schwebend); `parent` benennt das lebende Hauptfenster.
        unsafe { f(self.plugin, &parent) };
    }

    #[cfg(not(all(unix, not(target_os = "macos"))))]
    fn set_transient(&self, _vt: clap_plugin_gui, _api: &CStr) {}

    fn open_embedded(
        &mut self,
        vt: clap_plugin_gui,
        api: &CStr,
        title: &str,
    ) -> Result<(), crate::error::PluginError> {
        let p = self.plugin;
        let (Some(create), Some(set_parent)) = (vt.create, vt.set_parent) else {
            return Err(crate::error::PluginError::Window(
                crate::error::WindowProblem::IncompleteGui,
            ));
        };
        self.shared.resize.store(0, Ordering::Relaxed);
        // SAFETY: Plugin lebt; Hauptthread; noch keine GUI.
        if !unsafe { create(p, api.as_ptr(), false) } {
            return Err(crate::error::PluginError::Window(
                crate::error::WindowProblem::CannotCreate,
            ));
        }
        self.gui.created = true;
        if let Some(f) = vt.set_scale {
            // SAFETY: GUI existiert.
            unsafe { f(p, 1.0) };
        }
        let (mut w, mut h) = (640u32, 480u32);
        if let Some(f) = vt.get_size {
            let (mut a, mut b) = (0, 0);
            // SAFETY: GUI existiert; Ausgabezeiger sind gültig.
            if unsafe { f(p, &mut a, &mut b) } && a > 0 && b > 0 {
                (w, h) = (a.min(16_384), b.min(16_384));
            }
        }
        let win = match HostWindow::new(title, w, h) {
            Ok(win) => win,
            Err(e) => {
                self.close_gui();
                return Err(e);
            }
        };
        #[cfg(all(unix, not(target_os = "macos")))]
        let specific = clap_window_handle {
            x11: win.parent() as usize as u64,
        };
        #[cfg(windows)]
        let specific = clap_window_handle {
            win32: win.parent(),
        };
        #[cfg(target_os = "macos")]
        let specific = clap_window_handle {
            cocoa: win.parent(),
        };
        let parent = clap_window {
            api: api.as_ptr(),
            specific,
        };
        // SAFETY: GUI existiert; `parent` benennt unser lebendes Fenster.
        if !unsafe { set_parent(p, &parent) } {
            win.close();
            self.close_gui();
            return Err(crate::error::PluginError::Window(
                crate::error::WindowProblem::CannotEmbed,
            ));
        }
        // SAFETY: GUI existiert und ist eingebettet.
        if vt.show.is_some_and(|f| !unsafe { f(p) }) {
            win.close();
            self.close_gui();
            return Err(crate::error::PluginError::Window(
                crate::error::WindowProblem::CannotShow,
            ));
        }
        win.show();
        self.gui.window = Some(win);
        Ok(())
    }

    /// Schließt und zerstört das Plugin-Fenster. Nur im Hauptthread.
    pub fn close_gui(&mut self) {
        if self.gui.created
            && let Some(f) = self.gui_ext().and_then(|v| v.destroy)
        {
            // SAFETY: GUI existiert; wird genau einmal zerstört.
            unsafe { f(self.plugin) };
        }
        self.gui.created = false;
        if let Some(w) = self.gui.window.take() {
            w.close();
        }
    }

    /// Im UI-Takt aufrufen: Rückrufe, Timer, Dateideskriptoren, Fensterereignisse.
    /// Gibt `true` zurück, solange weiter regelmäßig aufgerufen werden sollte.
    pub fn idle(&mut self) -> bool {
        let p = self.plugin;
        if self.shared.callback.swap(false, Ordering::AcqRel) {
            // SAFETY: Plugin lebt; Hauptthread (Aufrufer).
            unsafe {
                if let Some(f) = (*p).on_main_thread {
                    f(p);
                }
            }
        }
        // Timer
        let due: Vec<u32> = {
            let now = Instant::now();
            let mut t = self.shared.lock_timers();
            t.iter_mut()
                .filter(|x| x.next <= now)
                .map(|x| {
                    x.next = now + x.every;
                    x.id
                })
                .collect()
        };
        if !due.is_empty() {
            // SAFETY: Erweiterung null oder gültig.
            if let Some(f) = unsafe {
                self.extension::<clap_plugin_timer_support>(CLAP_EXT_TIMER_SUPPORT)
                    .as_ref()
            }
            .and_then(|t| t.on_timer)
            {
                for id in due {
                    // SAFETY: Plugin lebt; Hauptthread.
                    unsafe { f(p, id) };
                }
            }
        }
        // Dateideskriptoren
        #[cfg(unix)]
        {
            use clap_sys::ext::posix_fd_support::{
                CLAP_EXT_POSIX_FD_SUPPORT, clap_plugin_posix_fd_support,
            };
            let fds = self.shared.lock_fds().clone();
            let ready = crate::host::ready_fds(&fds);
            if !ready.is_empty()
                // SAFETY: Erweiterung null oder gültig.
                && let Some(f) = unsafe {
                    self.extension::<clap_plugin_posix_fd_support>(CLAP_EXT_POSIX_FD_SUPPORT).as_ref()
                }
                .and_then(|x| x.on_fd)
            {
                for (fd, flags) in ready {
                    // SAFETY: Plugin lebt; Hauptthread.
                    unsafe { f(p, fd, flags) };
                }
            }
        }
        // Schwebendes Fenster vom Plugin geschlossen?
        if self.shared.gui_closed.swap(false, Ordering::AcqRel) {
            self.close_gui();
        }
        {
            let req = self.shared.resize.swap(0, Ordering::AcqRel);
            let mut closed = false;
            if let Some(w) = &self.gui.window {
                if req != 0 {
                    w.resize((req >> 32) as u32, req as u32);
                }
                closed = w.pump_closed();
            }
            if closed {
                self.close_gui();
            }
        }
        self.gui.created
    }
}

#[cfg(target_os = "macos")]
pub(crate) use cocoawin::HostWindow;
#[cfg(windows)]
pub(crate) use win32win::HostWindow;
/// Host-Fenster für eingebettete Plugin-Editoren auf diesem System.
#[cfg(all(unix, not(target_os = "macos")))]
pub(crate) use x11win::HostWindow;

#[cfg(all(unix, not(target_os = "macos")))]
pub(crate) mod x11win {
    //! Ein schlichtes X11-Fenster als Elternfenster für eingebettete Plugin-Editoren. Es merkt
    //! sich seine Position je Titel (solange die App läuft) und bleibt über dem Hauptfenster:
    //! unter X11 als dessen Unterfenster (`WM_TRANSIENT_FOR`), unter Wayland, wo ein
    //! XWayland-Fenster kein Wayland-Fenster als Eltern haben kann, als „immer oben“.

    use std::{
        collections::HashMap,
        sync::{Mutex, OnceLock},
    };

    use super::{MainWindow, main_window, wayland_session};

    use x11rb::{
        connection::Connection,
        protocol::{
            Event,
            xproto::{
                AtomEnum, ConfigureWindowAux, ConnectionExt as _, CreateWindowAux, EventMask,
                PropMode, Window, WindowClass,
            },
        },
        rust_connection::RustConnection,
        wrapper::ConnectionExt as _,
    };

    struct X {
        conn: RustConnection,
        root: Window,
        visual: u32,
        wm_protocols: u32,
        wm_delete: u32,
        net_wm_name: u32,
        utf8: u32,
        net_wm_state: u32,
        net_wm_state_above: u32,
        net_frame_extents: u32,
    }

    /// Wo die Fenster zuletzt standen (linke obere Ecke des Rahmens), je Titel.
    static POSITIONS: Mutex<Option<HashMap<String, (i32, i32)>>> = Mutex::new(None);

    fn remembered(title: &str) -> Option<(i32, i32)> {
        POSITIONS.lock().ok()?.as_ref()?.get(title).copied()
    }

    fn remember(title: &str, pos: (i32, i32)) {
        if let Ok(mut p) = POSITIONS.lock() {
            let map = p.get_or_insert_with(HashMap::new);
            // Begrenzt: ein Eintrag je geöffnetem Plugin-Fenster, nicht beliebig viele.
            if map.len() < 1000 || map.contains_key(title) {
                map.insert(title.to_owned(), pos);
            }
        }
    }

    fn x() -> Option<&'static X> {
        static X: OnceLock<Option<X>> = OnceLock::new();
        X.get_or_init(|| connect().ok()).as_ref()
    }

    fn connect() -> Result<X, Box<dyn std::error::Error>> {
        let (conn, screen) = x11rb::connect(None)?;
        let s = conn
            .setup()
            .roots
            .get(screen)
            .ok_or(crate::error::PluginError::Window(
                crate::error::WindowProblem::NoX11Display,
            ))?;
        let (root, visual) = (s.root, s.root_visual);
        let atom = |name: &[u8]| -> Result<u32, Box<dyn std::error::Error>> {
            Ok(conn.intern_atom(false, name)?.reply()?.atom)
        };
        Ok(X {
            wm_protocols: atom(b"WM_PROTOCOLS")?,
            wm_delete: atom(b"WM_DELETE_WINDOW")?,
            net_wm_name: atom(b"_NET_WM_NAME")?,
            utf8: atom(b"UTF8_STRING")?,
            net_wm_state: atom(b"_NET_WM_STATE")?,
            net_wm_state_above: atom(b"_NET_WM_STATE_ABOVE")?,
            net_frame_extents: atom(b"_NET_FRAME_EXTENTS")?,
            root,
            visual,
            conn,
        })
    }

    pub(crate) struct HostWindow {
        id: Window,
        title: String,
    }

    impl HostWindow {
        pub fn new(title: &str, w: u32, h: u32) -> Result<Self, crate::error::PluginError> {
            let x = x().ok_or(crate::error::PluginError::Window(
                crate::error::WindowProblem::NeedsX11,
            ))?;
            let err = |e: &dyn std::fmt::Display| {
                crate::error::PluginError::Window(crate::error::WindowProblem::HostWindow(
                    e.to_string(),
                ))
            };
            let id = x.conn.generate_id().map_err(|e| err(&e))?;
            let aux = CreateWindowAux::new().event_mask(EventMask::STRUCTURE_NOTIFY);
            let (w, h) = (w.clamp(1, 16_384) as u16, h.clamp(1, 16_384) as u16);
            let pos = remembered(title);
            let (px, py) = pos.map_or((0, 0), |(x, y)| {
                (
                    x.clamp(-16_384, 16_384) as i16,
                    y.clamp(-16_384, 16_384) as i16,
                )
            });
            x.conn
                .create_window(
                    x11rb::COPY_DEPTH_FROM_PARENT,
                    id,
                    x.root,
                    px,
                    py,
                    w,
                    h,
                    0,
                    WindowClass::INPUT_OUTPUT,
                    x.visual,
                    &aux,
                )
                .map_err(|e| err(&e))?;
            let _ = x.conn.change_property8(
                PropMode::REPLACE,
                id,
                AtomEnum::WM_NAME,
                AtomEnum::STRING,
                title.as_bytes(),
            );
            let _ = x.conn.change_property8(
                PropMode::REPLACE,
                id,
                x.net_wm_name,
                x.utf8,
                title.as_bytes(),
            );
            let _ = x.conn.change_property8(
                PropMode::REPLACE,
                id,
                AtomEnum::WM_CLASS,
                AtomEnum::STRING,
                b"studiocraft\0StudioCraft\0",
            );
            let _ = x.conn.change_property32(
                PropMode::REPLACE,
                id,
                x.wm_protocols,
                AtomEnum::ATOM,
                &[x.wm_delete],
            );
            if pos.is_some() {
                // WM_NORMAL_HINTS: vom Benutzer gewählte Position (USPosition), Rahmen oben links
                // dort (NorthWest), damit der Fenstermanager sie nicht selbst wählt.
                const US_POSITION: u32 = 1;
                const P_WIN_GRAVITY: u32 = 1 << 9;
                const NORTH_WEST: u32 = 1;
                let mut hints = [0u32; 18];
                hints[0] = US_POSITION | P_WIN_GRAVITY;
                hints[1] = px as i32 as u32;
                hints[2] = py as i32 as u32;
                hints[3] = u32::from(w);
                hints[4] = u32::from(h);
                hints[17] = NORTH_WEST;
                let _ = x.conn.change_property32(
                    PropMode::REPLACE,
                    id,
                    AtomEnum::WM_NORMAL_HINTS,
                    AtomEnum::WM_SIZE_HINTS,
                    &hints,
                );
            }
            match main_window() {
                Some(MainWindow::X11(main)) => {
                    let _ = x.conn.change_property32(
                        PropMode::REPLACE,
                        id,
                        AtomEnum::WM_TRANSIENT_FOR,
                        AtomEnum::WINDOW,
                        &[main],
                    );
                }
                _ if wayland_session() => {
                    // Vor dem Anzeigen gesetzt, gilt _NET_WM_STATE als Anfangszustand (EWMH).
                    let _ = x.conn.change_property32(
                        PropMode::REPLACE,
                        id,
                        x.net_wm_state,
                        AtomEnum::ATOM,
                        &[x.net_wm_state_above],
                    );
                }
                _ => {}
            }
            // Einmal hin und zurück, damit das Fenster existiert, bevor das Plugin
            // (mit eigener Verbindung) es als Elternfenster benutzt.
            x.conn.sync().map_err(|e| err(&e))?;
            Ok(Self {
                id,
                title: title.to_owned(),
            })
        }

        /// Linke obere Ecke des Rahmens auf dem Bildschirm (Inhalt minus Rahmenbreiten).
        pub fn position(&self) -> Option<(i32, i32)> {
            let x = x()?;
            let at = x
                .conn
                .translate_coordinates(self.id, x.root, 0, 0)
                .ok()?
                .reply()
                .ok()?;
            let (left, top) = x
                .conn
                .get_property(
                    false,
                    self.id,
                    x.net_frame_extents,
                    AtomEnum::CARDINAL,
                    0,
                    4,
                )
                .ok()
                .and_then(|c| c.reply().ok())
                .and_then(|r| r.value32().map(|v| v.collect::<Vec<u32>>()))
                .filter(|v| v.len() == 4)
                .map_or((0, 0), |v| {
                    (
                        i32::try_from(v[0]).unwrap_or(0),
                        i32::try_from(v[2]).unwrap_or(0),
                    )
                });
            Some((
                i32::from(at.dst_x).saturating_sub(left),
                i32::from(at.dst_y).saturating_sub(top),
            ))
        }

        /// Ob der Fenstermanager das Fenster „immer oben“ hält (für Tests).
        #[cfg(test)]
        pub fn keeps_above(&self) -> bool {
            let Some(x) = x() else { return false };
            x.conn
                .get_property(false, self.id, x.net_wm_state, AtomEnum::ATOM, 0, 32)
                .ok()
                .and_then(|c| c.reply().ok())
                .and_then(|r| {
                    r.value32()
                        .map(|mut v| v.any(|a| a == x.net_wm_state_above))
                })
                .unwrap_or(false)
        }

        /// Verschiebt das Fenster (Inhalt oben links; für Tests).
        #[cfg(test)]
        pub fn move_to(&self, left: i32, top: i32) {
            if let Some(x) = x() {
                let aux = ConfigureWindowAux::new().x(left).y(top);
                let _ = x.conn.configure_window(self.id, &aux);
                let _ = x.conn.sync();
            }
        }

        pub fn id(&self) -> u32 {
            self.id
        }

        /// Elternfenster für das Plugin (X11: die Fenster-Id).
        pub fn parent(&self) -> *mut std::ffi::c_void {
            self.id as usize as *mut std::ffi::c_void
        }

        pub fn show(&self) {
            if let Some(x) = x() {
                let _ = x.conn.map_window(self.id);
                let _ = x.conn.flush();
            }
        }

        pub fn resize(&self, w: u32, h: u32) {
            if let Some(x) = x() {
                let aux = ConfigureWindowAux::new()
                    .width(w.clamp(1, 16_384))
                    .height(h.clamp(1, 16_384));
                let _ = x.conn.configure_window(self.id, &aux);
                let _ = x.conn.flush();
            }
        }

        /// Liest anstehende Ereignisse; `true`, wenn das Fenster geschlossen wurde.
        pub fn pump_closed(&self) -> bool {
            let Some(x) = x() else { return false };
            let mut closed = false;
            while let Ok(Some(ev)) = x.conn.poll_for_event() {
                if let Event::ClientMessage(m) = ev
                    && m.window == self.id
                    && m.type_ == x.wm_protocols
                    && m.format == 32
                    && m.data.as_data32().first() == Some(&x.wm_delete)
                {
                    closed = true;
                }
            }
            closed
        }

        pub fn close(self) {
            if let Some(pos) = self.position() {
                remember(&self.title, pos);
            }
            if let Some(x) = x() {
                let _ = x.conn.destroy_window(self.id);
                let _ = x.conn.flush();
            }
        }
    }
}

/// Ein schlichtes Win32-Fenster als Elternfenster für eingebettete Plugin-Editoren. Seine
/// Nachrichten verteilt die Ereignisschleife des Programms (gleicher Thread); Schließen blendet
/// es aus und meldet es über [`HostWindow::pump_closed`].
#[cfg(windows)]
pub(crate) mod win32win {
    use std::{
        ffi::c_void,
        ptr,
        sync::{
            OnceLock,
            atomic::{AtomicBool, Ordering},
        },
    };

    use windows_sys::Win32::{
        Foundation::{HWND, LPARAM, LRESULT, RECT, WPARAM},
        System::LibraryLoader::GetModuleHandleW,
        UI::WindowsAndMessaging::{
            AdjustWindowRectEx, CW_USEDEFAULT, CreateWindowExW, DefWindowProcW, DestroyWindow,
            GWLP_USERDATA, GetWindowLongPtrW, IDC_ARROW, LoadCursorW, RegisterClassExW, SW_HIDE,
            SW_SHOW, SWP_NOMOVE, SWP_NOZORDER, SetWindowLongPtrW, SetWindowPos, ShowWindow,
            WM_CLOSE, WNDCLASSEXW, WS_MAXIMIZEBOX, WS_OVERLAPPEDWINDOW, WS_THICKFRAME,
        },
    };

    const STYLE: u32 = WS_OVERLAPPEDWINDOW & !(WS_THICKFRAME | WS_MAXIMIZEBOX);

    fn wide(s: &str) -> Vec<u16> {
        s.encode_utf16().chain(std::iter::once(0)).collect()
    }

    unsafe extern "system" fn wndproc(hwnd: HWND, msg: u32, wp: WPARAM, lp: LPARAM) -> LRESULT {
        if msg == WM_CLOSE {
            // SAFETY: GWLP_USERDATA hält null oder unser `closed`-Flag (lebt mit dem Fenster).
            unsafe {
                let flag = GetWindowLongPtrW(hwnd, GWLP_USERDATA) as *const AtomicBool;
                if !flag.is_null() {
                    (*flag).store(true, Ordering::Release);
                }
                ShowWindow(hwnd, SW_HIDE);
            }
            return 0;
        }
        // SAFETY: Standardbehandlung für alle anderen Nachrichten.
        unsafe { DefWindowProcW(hwnd, msg, wp, lp) }
    }

    /// Fensterklasse einmal anmelden; ihr Name.
    fn class() -> Option<&'static [u16]> {
        static CLASS: OnceLock<Option<Vec<u16>>> = OnceLock::new();
        CLASS
            .get_or_init(|| {
                let name = wide("StudioCraftPluginHost");
                // SAFETY: Struktur vollständig gefüllt; Name und Prozedur leben bis Programmende.
                let ok = unsafe {
                    let wc = WNDCLASSEXW {
                        cbSize: std::mem::size_of::<WNDCLASSEXW>() as u32,
                        style: 0,
                        lpfnWndProc: Some(wndproc),
                        cbClsExtra: 0,
                        cbWndExtra: 0,
                        hInstance: GetModuleHandleW(ptr::null()),
                        hIcon: ptr::null_mut(),
                        hCursor: LoadCursorW(ptr::null_mut(), IDC_ARROW),
                        hbrBackground: ptr::null_mut(),
                        lpszMenuName: ptr::null(),
                        lpszClassName: name.as_ptr(),
                        hIconSm: ptr::null_mut(),
                    };
                    RegisterClassExW(&wc) != 0
                };
                ok.then_some(name)
            })
            .as_deref()
    }

    /// Fenstergröße für eine gewünschte Innengröße.
    fn outer(w: u32, h: u32) -> (i32, i32) {
        let mut r = RECT {
            left: 0,
            top: 0,
            right: w.clamp(1, 16_384) as i32,
            bottom: h.clamp(1, 16_384) as i32,
        };
        // SAFETY: gültiges Rechteck.
        unsafe { AdjustWindowRectEx(&mut r, STYLE, 0, 0) };
        (r.right - r.left, r.bottom - r.top)
    }

    pub(crate) struct HostWindow {
        hwnd: HWND,
        closed: Box<AtomicBool>,
    }

    impl HostWindow {
        pub fn new(title: &str, w: u32, h: u32) -> Result<Self, crate::error::PluginError> {
            let class = class().ok_or(crate::error::PluginError::Window(
                crate::error::WindowProblem::HostClass,
            ))?;
            let title = wide(title);
            let (ow, oh) = outer(w, h);
            // SAFETY: Klasse angemeldet; Zeichenketten nullterminiert und während des Aufrufs gültig.
            let hwnd = unsafe {
                CreateWindowExW(
                    0,
                    class.as_ptr(),
                    title.as_ptr(),
                    STYLE,
                    CW_USEDEFAULT,
                    CW_USEDEFAULT,
                    ow,
                    oh,
                    ptr::null_mut(),
                    ptr::null_mut(),
                    GetModuleHandleW(ptr::null()),
                    ptr::null(),
                )
            };
            if hwnd.is_null() {
                return Err(crate::error::PluginError::Window(
                    crate::error::WindowProblem::HostCreate,
                ));
            }
            let closed = Box::new(AtomicBool::new(false));
            // SAFETY: Fenster gehört uns; das Flag lebt so lange wie das Fenster (close setzt
            // den Zeiger vor dem Zerstören zurück).
            unsafe {
                SetWindowLongPtrW(hwnd, GWLP_USERDATA, &*closed as *const AtomicBool as isize)
            };
            Ok(Self { hwnd, closed })
        }

        /// Elternfenster für das Plugin (HWND).
        pub fn parent(&self) -> *mut c_void {
            self.hwnd
        }

        pub fn show(&self) {
            // SAFETY: Fenster lebt.
            unsafe { ShowWindow(self.hwnd, SW_SHOW) };
        }

        pub fn resize(&self, w: u32, h: u32) {
            let (ow, oh) = outer(w, h);
            // SAFETY: Fenster lebt.
            unsafe {
                SetWindowPos(
                    self.hwnd,
                    ptr::null_mut(),
                    0,
                    0,
                    ow,
                    oh,
                    SWP_NOMOVE | SWP_NOZORDER,
                )
            };
        }

        /// `true`, wenn das Fenster geschlossen wurde.
        pub fn pump_closed(&self) -> bool {
            self.closed.load(Ordering::Acquire)
        }

        pub fn close(self) {
            // SAFETY: Fenster lebt; Flag-Zeiger zuerst lösen, dann zerstören.
            unsafe {
                SetWindowLongPtrW(self.hwnd, GWLP_USERDATA, 0);
                DestroyWindow(self.hwnd);
            }
        }
    }
}

/// Ein schlichtes Cocoa-Fenster als Elternfenster für eingebettete Plugin-Editoren (NSView).
/// Nur im Hauptthread. Schließen blendet es aus (es wird nicht freigegeben), das meldet
/// [`HostWindow::pump_closed`].
#[cfg(target_os = "macos")]
pub(crate) mod cocoawin {
    use std::ffi::c_void;

    use objc2::{MainThreadMarker, rc::Retained};
    use objc2_app_kit::{NSBackingStoreType, NSWindow, NSWindowStyleMask};
    use objc2_foundation::{NSPoint, NSRect, NSSize, NSString};

    pub(crate) struct HostWindow {
        win: Retained<NSWindow>,
    }

    impl HostWindow {
        pub fn new(title: &str, w: u32, h: u32) -> Result<Self, crate::error::PluginError> {
            let mtm = MainThreadMarker::new().ok_or(crate::error::PluginError::Window(
                crate::error::WindowProblem::MainThreadOnly,
            ))?;
            let size = NSSize::new(f64::from(w.clamp(1, 16_384)), f64::from(h.clamp(1, 16_384)));
            let rect = NSRect::new(NSPoint::new(200.0, 200.0), size);
            let style = NSWindowStyleMask::Titled
                | NSWindowStyleMask::Closable
                | NSWindowStyleMask::Miniaturizable;
            // SAFETY: Hauptthread (Marker); gültige Größe und Stil.
            let win = unsafe {
                NSWindow::initWithContentRect_styleMask_backing_defer(
                    mtm.alloc(),
                    rect,
                    style,
                    NSBackingStoreType::Buffered,
                    false,
                )
            };
            // SAFETY: Wir halten das Fenster selbst; Schließen darf es nicht freigeben.
            unsafe { win.setReleasedWhenClosed(false) };
            win.setTitle(&NSString::from_str(title));
            Ok(Self { win })
        }

        /// Elternfenster für das Plugin (die Inhaltsansicht, NSView).
        pub fn parent(&self) -> *mut c_void {
            // Das Fenster hält seine Inhaltsansicht; der Zeiger bleibt gültig.
            self.win.contentView().map_or(std::ptr::null_mut(), |v| {
                Retained::as_ptr(&v) as *mut c_void
            })
        }

        pub fn show(&self) {
            self.win.makeKeyAndOrderFront(None);
        }

        pub fn resize(&self, w: u32, h: u32) {
            self.win.setContentSize(NSSize::new(
                f64::from(w.clamp(1, 16_384)),
                f64::from(h.clamp(1, 16_384)),
            ));
        }

        /// `true`, wenn das Fenster geschlossen wurde (nicht mehr sichtbar und nicht im Dock).
        pub fn pump_closed(&self) -> bool {
            !self.win.isVisible() && !self.win.isMiniaturized()
        }

        pub fn close(self) {
            self.win.close();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Die Tests ändern das gemeldete Hauptfenster (global): nacheinander.
    static SESSION: std::sync::Mutex<()> = std::sync::Mutex::new(());

    /// Unter Wayland: schwebend nativ, dann eingebettet (XWayland), zuletzt schwebend X11;
    /// unter X11 schwebend vor eingebettet.
    #[cfg(all(unix, not(target_os = "macos")))]
    #[test]
    fn the_window_plan_follows_the_session() {
        use clap_sys::ext::gui::{CLAP_WINDOW_API_WAYLAND, CLAP_WINDOW_API_X11};
        let all = |_: &CStr, _: bool| true;
        assert_eq!(
            plan(true, all),
            Some(Mode::Floating(CLAP_WINDOW_API_WAYLAND))
        );
        assert_eq!(plan(false, all), Some(Mode::Floating(CLAP_WINDOW_API_X11)));
        // Ein Plugin nur mit X11: unter Wayland lieber unser Fenster (bleibt oben).
        let x11_only = |api: &CStr, _: bool| api == CLAP_WINDOW_API_X11;
        assert_eq!(
            plan(true, x11_only),
            Some(Mode::Embedded(CLAP_WINDOW_API_X11))
        );
        assert_eq!(
            plan(false, x11_only),
            Some(Mode::Floating(CLAP_WINDOW_API_X11))
        );
        // Nur schwebend X11: bleibt auch unter Wayland möglich.
        let floating_x11 = |api: &CStr, floating: bool| floating && api == CLAP_WINDOW_API_X11;
        assert_eq!(
            plan(true, floating_x11),
            Some(Mode::Floating(CLAP_WINDOW_API_X11))
        );
        assert_eq!(plan(true, |_: &CStr, _: bool| false), None);
    }

    #[test]
    fn the_app_tells_which_session_it_runs_in() {
        let _one = SESSION.lock().unwrap_or_else(|e| e.into_inner());
        set_main_window(MainWindow::Wayland);
        assert!(wayland_session());
        set_main_window(MainWindow::X11(42));
        assert!(!wayland_session());
        assert_eq!(main_window(), Some(MainWindow::X11(42)));
        set_main_window(MainWindow::Other);
        assert!(!wayland_session());
    }

    /// Öffnet echte Fenster (XWayland oder X11):
    /// `STUDIOCRAFT_GUI_TEST=1 cargo test -p studiocraft-plugin-host host_window`
    #[cfg(all(unix, not(target_os = "macos")))]
    #[test]
    fn host_window_remembers_its_position_and_stays_on_top() {
        if std::env::var_os("STUDIOCRAFT_GUI_TEST").is_none() {
            return;
        }
        let _one = SESSION.lock().unwrap_or_else(|e| e.into_inner());
        set_main_window(MainWindow::Wayland);
        let pump = |w: &HostWindow| {
            for _ in 0..20 {
                w.pump_closed();
                std::thread::sleep(std::time::Duration::from_millis(25));
            }
        };
        let title = "StudioCraft-Test – Position";
        let w = HostWindow::new(title, 300, 200).expect("X11");
        w.show();
        pump(&w);
        assert!(w.keeps_above(), "immer oben unter Wayland");
        w.move_to(321, 234);
        pump(&w);
        let moved = w.position().expect("Position");
        w.close();
        let again = HostWindow::new(title, 300, 200).expect("X11");
        again.show();
        pump(&again);
        let back = again.position().expect("Position");
        again.close();
        assert!(
            (back.0 - moved.0).abs() <= 2 && (back.1 - moved.1).abs() <= 2,
            "vorher {moved:?}, nachher {back:?}"
        );
    }
}
