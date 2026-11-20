//! Host-Seite von CLAP: was ein Plugin bei uns aufrufen darf.
//!
//! Je Instanz gibt es gemeinsame Daten ([`HostShared`]); `clap_host.host_data`
//! zeigt darauf. Timer und Dateideskriptoren werden im UI-Takt bedient
//! (siehe `ClapHandle::idle`), wie es Plugin-GUIs unter Linux brauchen.

use std::{
    ffi::{CStr, c_char, c_void},
    ptr,
    sync::{
        Mutex, PoisonError,
        atomic::{AtomicBool, AtomicU32, AtomicU64, Ordering},
    },
    time::{Duration, Instant},
};

use clap_sys::{
    ext::{
        gui::{CLAP_EXT_GUI, clap_host_gui},
        thread_check::{CLAP_EXT_THREAD_CHECK, clap_host_thread_check},
        timer_support::{CLAP_EXT_TIMER_SUPPORT, clap_host_timer_support},
    },
    host::clap_host,
    id::clap_id,
};

/// Höchstzahl Timer bzw. Dateideskriptoren je Plugin.
const MAX_ENTRIES: usize = 256;

pub(crate) struct Timer {
    pub id: clap_id,
    pub every: Duration,
    pub next: Instant,
}

/// Von Plugin-Rückrufen gesetzt, vom Hauptthread abgeholt.
#[derive(Default)]
pub(crate) struct HostShared {
    /// Plugin möchte `on_main_thread` aufgerufen bekommen.
    pub callback: AtomicBool,
    /// Das Plugin hat sein schwebendes Fenster geschlossen.
    pub gui_closed: AtomicBool,
    /// Gewünschte Fenstergröße (Breite << 32 | Höhe), 0 = keine.
    pub resize: AtomicU64,
    pub timers: Mutex<Vec<Timer>>,
    next_timer: AtomicU32,
    /// Angemeldete Dateideskriptoren (`clap.posix-fd-support`, nur Unix).
    #[cfg(unix)]
    pub fds: Mutex<Vec<(i32, u32)>>,
    /// Das Plugin meldet geänderten Zustand (z. B. Einstellung im eigenen Fenster).
    pub state_dirty: AtomicBool,
    /// Das Plugin meldet eine geänderte Latenz (`clap.latency`): neu abfragen.
    pub latency_changed: AtomicBool,
}

impl HostShared {
    pub fn lock_timers(&self) -> std::sync::MutexGuard<'_, Vec<Timer>> {
        self.timers.lock().unwrap_or_else(PoisonError::into_inner)
    }
    #[cfg(unix)]
    pub fn lock_fds(&self) -> std::sync::MutexGuard<'_, Vec<(i32, u32)>> {
        self.fds.lock().unwrap_or_else(PoisonError::into_inner)
    }
}

/// Die UI läuft im Thread „main“ (Rust benennt den ersten Thread so).
pub(crate) fn on_main_thread() -> bool {
    std::thread::current().name() == Some("main")
}

/// # Safety
/// `host` muss ein von uns erzeugter `clap_host` sein.
unsafe fn shared<'a>(host: *const clap_host) -> Option<&'a HostShared> {
    // SAFETY: `host_data` zeigt auf das `HostShared` der Instanz, das länger lebt als sie.
    unsafe { ((*host).host_data as *const HostShared).as_ref() }
}

pub(crate) unsafe extern "C" fn host_request_callback(host: *const clap_host) {
    // SAFETY: von CLAP mit unserem Host aufgerufen.
    if let Some(s) = unsafe { shared(host) } {
        s.callback.store(true, Ordering::Release);
    }
}
pub(crate) unsafe extern "C" fn host_request_nothing(_: *const clap_host) {}

// --- clap.gui (Host) ---
unsafe extern "C" fn gui_resize_hints_changed(_: *const clap_host) {}
unsafe extern "C" fn gui_request_resize(host: *const clap_host, w: u32, h: u32) -> bool {
    // SAFETY: wie oben.
    match unsafe { shared(host) } {
        Some(s) if w > 0 && h > 0 => {
            s.resize
                .store((u64::from(w) << 32) | u64::from(h), Ordering::Release);
            true
        }
        _ => false,
    }
}
unsafe extern "C" fn gui_request_show(_: *const clap_host) -> bool {
    false
}
unsafe extern "C" fn gui_request_hide(_: *const clap_host) -> bool {
    false
}
unsafe extern "C" fn gui_closed(host: *const clap_host, _was_destroyed: bool) {
    // SAFETY: wie oben.
    if let Some(s) = unsafe { shared(host) } {
        s.gui_closed.store(true, Ordering::Release);
    }
}
static HOST_GUI: clap_host_gui = clap_host_gui {
    resize_hints_changed: Some(gui_resize_hints_changed),
    request_resize: Some(gui_request_resize),
    request_show: Some(gui_request_show),
    request_hide: Some(gui_request_hide),
    closed: Some(gui_closed),
};

// --- clap.timer-support (Host) ---
unsafe extern "C" fn register_timer(
    host: *const clap_host,
    period_ms: u32,
    id: *mut clap_id,
) -> bool {
    // SAFETY: wie oben; `id` ist ein gültiger Ausgabezeiger.
    let Some(s) = (unsafe { shared(host) }) else {
        return false;
    };
    let mut t = s.lock_timers();
    if t.len() >= MAX_ENTRIES || id.is_null() {
        return false;
    }
    let new = s.next_timer.fetch_add(1, Ordering::Relaxed);
    let every = Duration::from_millis(u64::from(period_ms.clamp(1, 3_600_000)));
    t.push(Timer {
        id: new,
        every,
        next: Instant::now() + every,
    });
    // SAFETY: siehe oben.
    unsafe { *id = new };
    true
}
unsafe extern "C" fn unregister_timer(host: *const clap_host, id: clap_id) -> bool {
    // SAFETY: wie oben.
    let Some(s) = (unsafe { shared(host) }) else {
        return false;
    };
    let mut t = s.lock_timers();
    let before = t.len();
    t.retain(|x| x.id != id);
    t.len() != before
}
static HOST_TIMER: clap_host_timer_support = clap_host_timer_support {
    register_timer: Some(register_timer),
    unregister_timer: Some(unregister_timer),
};

// --- clap.posix-fd-support (Host), nur Unix ---
#[cfg(unix)]
mod fd {
    use super::*;
    use clap_sys::ext::posix_fd_support::clap_host_posix_fd_support;

    unsafe extern "C" fn register_fd(host: *const clap_host, fd: i32, flags: u32) -> bool {
        // SAFETY: wie oben.
        let Some(s) = (unsafe { shared(host) }) else {
            return false;
        };
        let mut f = s.lock_fds();
        if fd < 0 || f.len() >= MAX_ENTRIES || f.iter().any(|&(x, _)| x == fd) {
            return false;
        }
        f.push((fd, flags));
        true
    }
    unsafe extern "C" fn modify_fd(host: *const clap_host, fd: i32, flags: u32) -> bool {
        // SAFETY: wie oben.
        let Some(s) = (unsafe { shared(host) }) else {
            return false;
        };
        match s.lock_fds().iter_mut().find(|(x, _)| *x == fd) {
            Some(e) => {
                e.1 = flags;
                true
            }
            None => false,
        }
    }
    unsafe extern "C" fn unregister_fd(host: *const clap_host, fd: i32) -> bool {
        // SAFETY: wie oben.
        let Some(s) = (unsafe { shared(host) }) else {
            return false;
        };
        let mut f = s.lock_fds();
        let before = f.len();
        f.retain(|&(x, _)| x != fd);
        f.len() != before
    }
    pub(super) static HOST_FD: clap_host_posix_fd_support = clap_host_posix_fd_support {
        register_fd: Some(register_fd),
        modify_fd: Some(modify_fd),
        unregister_fd: Some(unregister_fd),
    };
}

// --- clap.state (Host) ---
unsafe extern "C" fn state_mark_dirty(host: *const clap_host) {
    // SAFETY: wie oben.
    if let Some(s) = unsafe { shared(host) } {
        s.state_dirty.store(true, Ordering::Release);
    }
}
static HOST_STATE: clap_sys::ext::state::clap_host_state = clap_sys::ext::state::clap_host_state {
    mark_dirty: Some(state_mark_dirty),
};

// --- clap.latency (Host) ---
unsafe extern "C" fn latency_changed(host: *const clap_host) {
    // SAFETY: wie oben.
    if let Some(s) = unsafe { shared(host) } {
        s.latency_changed.store(true, Ordering::Release);
    }
}
static HOST_LATENCY: clap_sys::ext::latency::clap_host_latency =
    clap_sys::ext::latency::clap_host_latency {
        changed: Some(latency_changed),
    };

// --- clap.thread-check (Host) ---
unsafe extern "C" fn is_main_thread(_: *const clap_host) -> bool {
    on_main_thread()
}
unsafe extern "C" fn is_audio_thread(_: *const clap_host) -> bool {
    !on_main_thread()
}
static HOST_THREAD_CHECK: clap_host_thread_check = clap_host_thread_check {
    is_main_thread: Some(is_main_thread),
    is_audio_thread: Some(is_audio_thread),
};

pub(crate) unsafe extern "C" fn get_extension(
    _: *const clap_host,
    id: *const c_char,
) -> *const c_void {
    if id.is_null() {
        return ptr::null();
    }
    // SAFETY: CLAP übergibt eine nullterminierte Kennung.
    let id = unsafe { CStr::from_ptr(id) };
    if id == CLAP_EXT_GUI {
        &HOST_GUI as *const _ as *const c_void
    } else if id == CLAP_EXT_TIMER_SUPPORT {
        &HOST_TIMER as *const _ as *const c_void
    } else if id == clap_sys::ext::state::CLAP_EXT_STATE {
        &HOST_STATE as *const _ as *const c_void
    } else if id == clap_sys::ext::latency::CLAP_EXT_LATENCY {
        &HOST_LATENCY as *const _ as *const c_void
    } else if id == CLAP_EXT_THREAD_CHECK {
        &HOST_THREAD_CHECK as *const _ as *const c_void
    } else {
        #[cfg(unix)]
        if id == clap_sys::ext::posix_fd_support::CLAP_EXT_POSIX_FD_SUPPORT {
            return &fd::HOST_FD as *const _ as *const c_void;
        }
        ptr::null()
    }
}

/// Dateideskriptoren, die bereit sind (ohne zu warten), mit passenden CLAP-Flags.
#[cfg(unix)]
pub(crate) fn ready_fds(fds: &[(i32, u32)]) -> Vec<(i32, u32)> {
    use clap_sys::ext::posix_fd_support::{
        CLAP_POSIX_FD_ERROR, CLAP_POSIX_FD_READ, CLAP_POSIX_FD_WRITE,
    };
    if fds.is_empty() {
        return Vec::new();
    }
    let mut polled: Vec<libc::pollfd> = fds
        .iter()
        .map(|&(fd, flags)| {
            let mut events = 0;
            if flags & CLAP_POSIX_FD_READ != 0 {
                events |= libc::POLLIN;
            }
            if flags & CLAP_POSIX_FD_WRITE != 0 {
                events |= libc::POLLOUT;
            }
            libc::pollfd {
                fd,
                events,
                revents: 0,
            }
        })
        .collect();
    // SAFETY: gültiges Array; Zeitlimit 0 blockiert nie.
    if unsafe { libc::poll(polled.as_mut_ptr(), polled.len() as libc::nfds_t, 0) } <= 0 {
        return Vec::new();
    }
    polled
        .iter()
        .filter(|p| p.revents != 0)
        .map(|p| {
            let mut f = 0;
            if p.revents & libc::POLLIN != 0 {
                f |= CLAP_POSIX_FD_READ;
            }
            if p.revents & libc::POLLOUT != 0 {
                f |= CLAP_POSIX_FD_WRITE;
            }
            if p.revents & (libc::POLLERR | libc::POLLHUP | libc::POLLNVAL) != 0 {
                f |= CLAP_POSIX_FD_ERROR;
            }
            (p.fd, f)
        })
        .collect()
}
