//! Plugin-Fenster: bevorzugt schwebend (das Plugin öffnet sein eigenes
//! Fenster, das klappt auf allen Systemen), sonst eingebettet in ein eigenes
//! Host-Fenster: X11 (Linux, BSD; unter Wayland über XWayland), Win32 (Windows)
//! oder Cocoa (macOS).

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

    pub(crate) struct HostWindow {
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
        pub fn new(title: &str, w: u32, h: u32) -> Result<Self, String> {
            let class = class().ok_or("Fensterklasse für Plugins ließ sich nicht anmelden")?;
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
                return Err("Fenster für das Plugin ließ sich nicht anlegen".into());
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
        pub fn new(title: &str, w: u32, h: u32) -> Result<Self, String> {
            let mtm = MainThreadMarker::new().ok_or("Plugin-Fenster nur im Hauptthread")?;
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
