//! VST3-Plugin-Fenster (Issues #32, #36): die Ansicht des Controllers, eingebettet in ein eigenes
//! Host-Fenster (X11, Win32 oder Cocoa, siehe `gui.rs`). Unter Linux erwarten Plugins vom Host
//! zusätzlich eine Ereignisschleife (Dateideskriptoren und Timer), die der Host im UI-Takt
//! bedient; der Plugin-Rahmen nimmt auf allen Systemen Größenwünsche an. Alles im Hauptthread.

pub(crate) use embed::Vst3Gui;

mod embed {
    use std::{
        ffi::{CStr, c_char, c_void},
        ptr,
        time::{Duration, Instant},
    };

    use crate::{
        gui::HostWindow,
        vst3::{
            EditControllerVt, Obj, RESULT_OK, Tresult, Tuid, UnknownVt, host_add_ref, host_query,
            host_release, query, release, uid, vt,
        },
    };

    const IID_PLUG_VIEW: Tuid = uid(0x5BC3_2507, 0xD060_49EA, 0xA615_1B52, 0x2B75_5B29);
    const IID_PLUG_FRAME: Tuid = uid(0x367F_AF01, 0xAFA9_4693, 0x8D4D_A2A0, 0xED08_82A3);
    const IID_RUN_LOOP: Tuid = uid(0x18C3_5366, 0x9776_4F1A, 0x9C5B_8385, 0x7A87_1389);
    const IID_EVENT_HANDLER: Tuid = uid(0x561E_65C9, 0x13A0_496F, 0x813A_2C35, 0x654D_7983);
    const IID_TIMER_HANDLER: Tuid = uid(0x10BD_D94F, 0x4142_4774, 0x821F_AD8F, 0xECA7_2CA9);
    /// Plattform-Typ der Ansicht auf diesem System.
    #[cfg(all(unix, not(target_os = "macos")))]
    const PLATFORM: &CStr = c"X11EmbedWindowID";
    #[cfg(windows)]
    const PLATFORM: &CStr = c"HWND";
    #[cfg(target_os = "macos")]
    const PLATFORM: &CStr = c"NSView";
    const EDITOR: &CStr = c"editor";

    #[repr(C)]
    #[derive(Default, Clone, Copy)]
    struct ViewRect {
        left: i32,
        top: i32,
        right: i32,
        bottom: i32,
    }

    #[repr(C)]
    struct PlugViewVt {
        unknown: UnknownVt,
        is_platform_type_supported: unsafe extern "system" fn(Obj, *const c_char) -> Tresult,
        attached: unsafe extern "system" fn(Obj, *mut c_void, *const c_char) -> Tresult,
        removed: unsafe extern "system" fn(Obj) -> Tresult,
        on_wheel: unsafe extern "system" fn(Obj, f32) -> Tresult,
        on_key_down: unsafe extern "system" fn(Obj, u16, i16, i16) -> Tresult,
        on_key_up: unsafe extern "system" fn(Obj, u16, i16, i16) -> Tresult,
        get_size: unsafe extern "system" fn(Obj, *mut ViewRect) -> Tresult,
        on_size: unsafe extern "system" fn(Obj, *mut ViewRect) -> Tresult,
        on_focus: unsafe extern "system" fn(Obj, u8) -> Tresult,
        set_frame: unsafe extern "system" fn(Obj, Obj) -> Tresult,
        can_resize: unsafe extern "system" fn(Obj) -> Tresult,
        check_size_constraint: unsafe extern "system" fn(Obj, *mut ViewRect) -> Tresult,
    }

    /// Nur unter Unix gebraucht (Dateideskriptoren werden nur dort bedient).
    #[cfg(unix)]
    #[repr(C)]
    struct EventHandlerVt {
        unknown: UnknownVt,
        on_fd_is_set: unsafe extern "system" fn(Obj, i32),
    }

    #[repr(C)]
    struct TimerHandlerVt {
        unknown: UnknownVt,
        on_timer: unsafe extern "system" fn(Obj),
    }

    /// Angemeldete Dateideskriptoren und Timer, gewünschte Größe.
    #[derive(Default)]
    struct LoopState {
        /// (angemeldetes Objekt, typisierte Schnittstelle mit eigener Referenz, Deskriptor)
        fds: Vec<(Obj, Obj, i32)>,
        timers: Vec<(Obj, Obj, Duration, Instant)>,
        resize: Option<ViewRect>,
    }

    #[repr(C)]
    struct PlugFrameVt {
        unknown: UnknownVt,
        resize_view: unsafe extern "system" fn(Obj, Obj, *mut ViewRect) -> Tresult,
    }

    #[repr(C)]
    struct RunLoopVt {
        unknown: UnknownVt,
        register_event_handler: unsafe extern "system" fn(Obj, Obj, i32) -> Tresult,
        unregister_event_handler: unsafe extern "system" fn(Obj, Obj) -> Tresult,
        register_timer: unsafe extern "system" fn(Obj, Obj, u64) -> Tresult,
        unregister_timer: unsafe extern "system" fn(Obj, Obj) -> Tresult,
    }

    /// Plugin-Rahmen und Ereignisschleife; beide Objekte zeigen auf denselben Zustand.
    #[repr(C)]
    struct FrameObj {
        vt: *const PlugFrameVt,
        state: *mut LoopState,
        run_loop: *mut RunLoopObj,
    }

    #[repr(C)]
    struct RunLoopObj {
        vt: *const RunLoopVt,
        state: *mut LoopState,
    }

    unsafe extern "system" fn frame_query(this: Obj, iid: *const Tuid, out: *mut Obj) -> Tresult {
        if !iid.is_null() && !out.is_null() {
            // SAFETY: `this` ist unser Rahmen; Zeiger laut Vertrag gültig.
            unsafe {
                if *iid == IID_RUN_LOOP {
                    *out = (*(this as *const FrameObj)).run_loop.cast();
                    return RESULT_OK;
                }
            }
        }
        // SAFETY: wie oben.
        unsafe { host_query(this, iid, out, &IID_PLUG_FRAME) }
    }

    unsafe extern "system" fn frame_resize(this: Obj, _view: Obj, rect: *mut ViewRect) -> Tresult {
        if rect.is_null() {
            return -1;
        }
        // SAFETY: `this` ist unser Rahmen, sein Zustand lebt mit dem Fenster; `rect` gültig.
        unsafe { (*(*(this as *const FrameObj)).state).resize = Some(*rect) };
        RESULT_OK
    }

    static FRAME_VT: PlugFrameVt = PlugFrameVt {
        unknown: UnknownVt {
            query_interface: frame_query,
            add_ref: host_add_ref,
            release: host_release,
        },
        resize_view: frame_resize,
    };

    unsafe extern "system" fn loop_query(this: Obj, iid: *const Tuid, out: *mut Obj) -> Tresult {
        // SAFETY: Aufruf durch das Plugin mit gültigen Zeigern.
        unsafe { host_query(this, iid, out, &IID_RUN_LOOP) }
    }

    /// # Safety
    /// `this` muss unser Ereignisschleifen-Objekt sein.
    unsafe fn loop_state<'a>(this: Obj) -> &'a mut LoopState {
        // SAFETY: laut Vertrag; der Zustand lebt so lange wie das Fenster.
        unsafe { &mut *(*(this as *const RunLoopObj)).state }
    }

    /// Typisierte Schnittstelle eines angemeldeten Handlers (mit eigener Referenz).
    ///
    /// # Safety
    /// `handler` muss ein lebendes Objekt des Plugins sein.
    unsafe fn typed(handler: Obj, iid: &Tuid) -> Obj {
        // SAFETY: laut Vertrag. Kennt das Objekt die Schnittstelle nicht über queryInterface,
        // gilt der übergebene Zeiger selbst (dann mit eigener Referenz).
        unsafe {
            query(handler, iid).unwrap_or_else(|| {
                (vt::<UnknownVt>(handler).add_ref)(handler);
                handler
            })
        }
    }

    unsafe extern "system" fn loop_add_fd(this: Obj, handler: Obj, fd: i32) -> Tresult {
        if handler.is_null() || fd < 0 {
            return -1;
        }
        // SAFETY: Handler gehört dem Plugin; wir halten eine Referenz, solange er angemeldet ist.
        unsafe {
            let h = typed(handler, &IID_EVENT_HANDLER);
            loop_state(this).fds.push((handler, h, fd));
        }
        RESULT_OK
    }

    unsafe extern "system" fn loop_remove_fd(this: Obj, handler: Obj) -> Tresult {
        // SAFETY: wie oben; jede Referenz wird genau einmal freigegeben.
        unsafe {
            let s = loop_state(this);
            s.fds.retain(|&(h, typed, _)| {
                if h == handler {
                    release(typed);
                    false
                } else {
                    true
                }
            });
        }
        RESULT_OK
    }

    unsafe extern "system" fn loop_add_timer(this: Obj, handler: Obj, ms: u64) -> Tresult {
        if handler.is_null() {
            return -1;
        }
        let every = Duration::from_millis(ms.clamp(1, 60_000));
        // SAFETY: wie bei den Dateideskriptoren.
        unsafe {
            let h = typed(handler, &IID_TIMER_HANDLER);
            loop_state(this)
                .timers
                .push((handler, h, every, Instant::now()));
        }
        RESULT_OK
    }

    unsafe extern "system" fn loop_remove_timer(this: Obj, handler: Obj) -> Tresult {
        // SAFETY: wie oben.
        unsafe {
            loop_state(this).timers.retain(|&(h, typed, _, _)| {
                if h == handler {
                    release(typed);
                    false
                } else {
                    true
                }
            });
        }
        RESULT_OK
    }

    static LOOP_VT: RunLoopVt = RunLoopVt {
        unknown: UnknownVt {
            query_interface: loop_query,
            add_ref: host_add_ref,
            release: host_release,
        },
        register_event_handler: loop_add_fd,
        unregister_event_handler: loop_remove_fd,
        register_timer: loop_add_timer,
        unregister_timer: loop_remove_timer,
    };

    /// Ein offenes VST3-Plugin-Fenster.
    pub(crate) struct Vst3Gui {
        view: Obj,
        window: HostWindow,
        // Rückrufziele des Plugins; leben bis nach `removed`.
        state: Box<LoopState>,
        frame: Box<FrameObj>,
        run_loop: Box<RunLoopObj>,
    }

    impl Vst3Gui {
        /// Hat der Controller eine Ansicht, die sich unter X11 einbetten lässt?
        pub(crate) fn supported(controller: Obj) -> bool {
            if controller.is_null() {
                return false;
            }
            // SAFETY: Controller lebt; Ansicht wird nur geprüft und wieder freigegeben.
            unsafe {
                let view =
                    (vt::<EditControllerVt>(controller).create_view)(controller, EDITOR.as_ptr());
                if view.is_null() {
                    return false;
                }
                let ok =
                    (vt::<PlugViewVt>(view).is_platform_type_supported)(view, PLATFORM.as_ptr())
                        == RESULT_OK;
                release(view);
                ok
            }
        }

        pub(crate) fn open(controller: Obj, title: &str) -> Result<Self, String> {
            if controller.is_null() {
                return Err("Plugin hat keinen Controller".into());
            }
            // SAFETY: Controller lebt; Aufrufe im Hauptthread in der vorgesehenen Reihenfolge.
            unsafe {
                let view =
                    (vt::<EditControllerVt>(controller).create_view)(controller, EDITOR.as_ptr());
                if view.is_null() {
                    return Err("Plugin hat kein eigenes Fenster".into());
                }
                // Manche Plugins liefern die Ansicht nur über queryInterface korrekt typisiert.
                let view = match query(view, &IID_PLUG_VIEW) {
                    Some(v) => {
                        release(view);
                        v
                    }
                    None => view,
                };
                let pv = vt::<PlugViewVt>(view);
                if (pv.is_platform_type_supported)(view, PLATFORM.as_ptr()) != RESULT_OK {
                    release(view);
                    return Err("Plugin-Fenster lässt sich hier nicht einbetten".into());
                }
                let mut state = Box::<LoopState>::default();
                let sp: *mut LoopState = &mut *state;
                let mut run_loop = Box::new(RunLoopObj {
                    vt: &LOOP_VT,
                    state: sp,
                });
                let mut frame = Box::new(FrameObj {
                    vt: &FRAME_VT,
                    state: sp,
                    run_loop: &mut *run_loop,
                });
                (pv.set_frame)(view, (&mut *frame as *mut FrameObj).cast());
                let mut rect = ViewRect::default();
                let (w, h) = if (pv.get_size)(view, &mut rect) == RESULT_OK {
                    (rect.right - rect.left, rect.bottom - rect.top)
                } else {
                    (640, 480)
                };
                let (w, h) = (w.clamp(16, 16_384) as u32, h.clamp(16, 16_384) as u32);
                let window = match HostWindow::new(title, w, h) {
                    Ok(win) => win,
                    Err(e) => {
                        (pv.set_frame)(view, ptr::null_mut());
                        release(view);
                        return Err(e);
                    }
                };
                let parent = window.parent();
                if (pv.attached)(view, parent, PLATFORM.as_ptr()) != RESULT_OK {
                    (pv.set_frame)(view, ptr::null_mut());
                    release(view);
                    window.close();
                    return Err("Plugin-Fenster ließ sich nicht einbetten".into());
                }
                window.show();
                let _ = &mut run_loop;
                Ok(Self {
                    view,
                    window,
                    state,
                    frame,
                    run_loop,
                })
            }
        }

        /// Im UI-Takt: Dateideskriptoren und Timer des Plugins bedienen, Größenwünsche umsetzen.
        /// `false`, wenn das Fenster geschlossen wurde (dann [`Self::close`] aufrufen).
        pub(crate) fn idle(&mut self) -> bool {
            // Dateideskriptoren (nur Linux/BSD): ohne Warten prüfen, ob etwas zu lesen ist.
            #[cfg(unix)]
            self.poll_fds();
            let now = Instant::now();
            let due: Vec<Obj> = self
                .state
                .timers
                .iter_mut()
                .filter(|(_, _, every, last)| now.duration_since(*last) >= *every)
                .map(|(_, h, _, last)| {
                    *last = now;
                    *h
                })
                .collect();
            for h in due {
                // SAFETY: wie oben.
                unsafe { (vt::<TimerHandlerVt>(h).on_timer)(h) };
            }
            if let Some(mut r) = self.state.resize.take() {
                let (w, h) = (r.right - r.left, r.bottom - r.top);
                self.window
                    .resize(w.clamp(16, 16_384) as u32, h.clamp(16, 16_384) as u32);
                // SAFETY: Ansicht lebt; Hauptthread.
                unsafe { (vt::<PlugViewVt>(self.view).on_size)(self.view, &mut r) };
            }
            !self.window.pump_closed()
        }

        #[cfg(unix)]
        fn poll_fds(&mut self) {
            let mut fds: Vec<libc::pollfd> = self
                .state
                .fds
                .iter()
                .map(|&(_, _, fd)| libc::pollfd {
                    fd,
                    events: libc::POLLIN,
                    revents: 0,
                })
                .collect();
            if !fds.is_empty() {
                // SAFETY: Array mit `len` gültigen Einträgen; Zeitlimit 0 (blockiert nicht).
                let n = unsafe { libc::poll(fds.as_mut_ptr(), fds.len() as libc::nfds_t, 0) };
                if n > 0 {
                    let ready: Vec<(Obj, i32)> = self
                        .state
                        .fds
                        .iter()
                        .zip(&fds)
                        .filter(|(_, p)| p.revents != 0)
                        .map(|(&(_, h, fd), _)| (h, fd))
                        .collect();
                    for (h, fd) in ready {
                        // SAFETY: Handler angemeldet (wir halten eine Referenz); Hauptthread.
                        unsafe { (vt::<EventHandlerVt>(h).on_fd_is_set)(h, fd) };
                    }
                }
            }
        }

        pub(crate) fn close(mut self) {
            // SAFETY: Ansicht lebt; danach keine Rückrufe mehr; jede Referenz einmal freigeben.
            unsafe {
                let pv = vt::<PlugViewVt>(self.view);
                (pv.removed)(self.view);
                (pv.set_frame)(self.view, ptr::null_mut());
                release(self.view);
                for (_, h, _) in self.state.fds.drain(..) {
                    release(h);
                }
                for (_, h, _, _) in self.state.timers.drain(..) {
                    release(h);
                }
            }
            let _ = (&self.frame, &self.run_loop);
            self.window.close();
        }
    }
}
