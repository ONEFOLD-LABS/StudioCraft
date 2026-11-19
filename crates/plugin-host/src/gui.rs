//! Plugin-Fenster: bevorzugt schwebend (das Plugin öffnet sein eigenes
//! Fenster, das klappt auf allen Systemen), sonst eingebettet in ein eigenes
//! Host-Fenster. Eingebettet geht bisher unter X11 (Linux, BSD; unter Wayland
//! über XWayland). Windows und macOS folgen.

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
    #[cfg(all(unix, not(target_os = "macos")))]
    window: Option<x11win::HostWindow>,
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
    pub fn open_gui(&mut self, title: &str) -> Result<(), String> {
        if !on_main_thread() {
            return Err("Plugin-Fenster nur im Hauptthread".into());
        }
        let vt = self.gui_ext().ok_or("Plugin hat kein eigenes Fenster")?;
        let p = self.plugin;
        if self.gui.created {
            if let Some(show) = vt.show {
                // SAFETY: GUI existiert; Hauptthread.
                unsafe { show(p) };
            }
            return Ok(());
        }
        let api =
            window_api().ok_or("Plugin-Fenster werden auf diesem System nicht unterstützt")?;
        let (Some(supported), Some(create)) = (vt.is_api_supported, vt.create) else {
            return Err("unvollständige clap.gui-Erweiterung".into());
        };
        // SAFETY: Plugin lebt; `api` ist statisch und nullterminiert; Hauptthread.
        let floating = unsafe { supported(p, api.as_ptr(), true) };
        if !floating {
            // SAFETY: wie oben.
            if unsafe { supported(p, api.as_ptr(), false) } {
                return self.open_embedded(vt, api, title);
            }
            return Err("Plugin bietet kein passendes Fenster an".into());
        }
        self.shared.gui_closed.store(false, Ordering::Relaxed);
        // SAFETY: wie oben; noch keine GUI.
        if !unsafe { create(p, api.as_ptr(), true) } {
            return Err("Plugin konnte sein Fenster nicht anlegen".into());
        }
        self.gui.created = true;
        if let (Some(f), Ok(t)) = (vt.suggest_title, CString::new(title)) {
            // SAFETY: GUI existiert; `t` lebt während des Aufrufs.
            unsafe { f(p, t.as_ptr()) };
        }
        // SAFETY: GUI existiert.
        if !vt.show.is_some_and(|f| unsafe { f(p) }) {
            self.close_gui();
            return Err("Plugin konnte sein Fenster nicht zeigen".into());
        }
        Ok(())
    }

    #[cfg(all(unix, not(target_os = "macos")))]
    fn open_embedded(
        &mut self,
        vt: clap_plugin_gui,
        api: &CStr,
        title: &str,
    ) -> Result<(), String> {
        let p = self.plugin;
        let (Some(create), Some(set_parent)) = (vt.create, vt.set_parent) else {
            return Err("unvollständige clap.gui-Erweiterung".into());
        };
        self.shared.resize.store(0, Ordering::Relaxed);
        // SAFETY: Plugin lebt; Hauptthread; noch keine GUI.
        if !unsafe { create(p, api.as_ptr(), false) } {
            return Err("Plugin konnte sein Fenster nicht anlegen".into());
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
        let win = match x11win::HostWindow::new(title, w, h) {
            Ok(win) => win,
            Err(e) => {
                self.close_gui();
                return Err(e);
            }
        };
        let parent = clap_window {
            api: api.as_ptr(),
            specific: clap_window_handle {
                x11: win.id().into(),
            },
        };
        // SAFETY: GUI existiert; `parent` benennt unser lebendes Fenster.
        if !unsafe { set_parent(p, &parent) } {
            win.close();
            self.close_gui();
            return Err("Plugin-Fenster ließ sich nicht einbetten".into());
        }
        // SAFETY: GUI existiert und ist eingebettet.
        if vt.show.is_some_and(|f| !unsafe { f(p) }) {
            win.close();
            self.close_gui();
            return Err("Plugin konnte sein Fenster nicht zeigen".into());
        }
        win.show();
        self.gui.window = Some(win);
        Ok(())
    }

    #[cfg(not(all(unix, not(target_os = "macos"))))]
    fn open_embedded(&mut self, _: clap_plugin_gui, _: &CStr, _: &str) -> Result<(), String> {
        Err("Dieses Plugin bietet nur ein eingebettetes Fenster; das folgt auf diesem System später".into())
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
        #[cfg(all(unix, not(target_os = "macos")))]
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
        #[cfg(all(unix, not(target_os = "macos")))]
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

#[cfg(all(unix, not(target_os = "macos")))]
mod x11win {
    //! Ein schlichtes X11-Fenster als Elternfenster für eingebettete Plugin-Editoren.

    use std::sync::OnceLock;

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
            .ok_or("kein X11-Bildschirm")?;
        let (root, visual) = (s.root, s.root_visual);
        let atom = |name: &[u8]| -> Result<u32, Box<dyn std::error::Error>> {
            Ok(conn.intern_atom(false, name)?.reply()?.atom)
        };
        Ok(X {
            wm_protocols: atom(b"WM_PROTOCOLS")?,
            wm_delete: atom(b"WM_DELETE_WINDOW")?,
            net_wm_name: atom(b"_NET_WM_NAME")?,
            utf8: atom(b"UTF8_STRING")?,
            root,
            visual,
            conn,
        })
    }

    pub(super) struct HostWindow {
        id: Window,
    }

    impl HostWindow {
        pub fn new(title: &str, w: u32, h: u32) -> Result<Self, String> {
            let x = x().ok_or("Plugin-Fenster brauchen X11 (unter Wayland: XWayland)")?;
            let err = |e: &dyn std::fmt::Display| format!("Fenster für das Plugin: {e}");
            let id = x.conn.generate_id().map_err(|e| err(&e))?;
            let aux = CreateWindowAux::new().event_mask(EventMask::STRUCTURE_NOTIFY);
            x.conn
                .create_window(
                    x11rb::COPY_DEPTH_FROM_PARENT,
                    id,
                    x.root,
                    0,
                    0,
                    w.clamp(1, 16_384) as u16,
                    h.clamp(1, 16_384) as u16,
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
            // Einmal hin und zurück, damit das Fenster existiert, bevor das Plugin
            // (mit eigener Verbindung) es als Elternfenster benutzt.
            x.conn.sync().map_err(|e| err(&e))?;
            Ok(Self { id })
        }

        pub fn id(&self) -> u32 {
            self.id
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
            if let Some(x) = x() {
                let _ = x.conn.destroy_window(self.id);
                let _ = x.conn.flush();
            }
        }
    }
}
