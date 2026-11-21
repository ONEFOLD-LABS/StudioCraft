//! Plugin bridge (Issue #146): one plugin instance in a helper process, so a plugin that
//! crashes or hangs while processing silences only its own insert instead of taking the app
//! down.
//!
//! The helper is the app's own binary started with [`BRIDGE_FLAG`] (as for the scan); it calls
//! [`run_bridge_child`] and hosts the plugin in any format with the usual [`PluginInstance`].
//!
//! - **Audio** goes through shared memory ([`shm`]), one block behind: in each callback the app
//!   takes the result of the previous block and hands over the current one, then wakes the
//!   helper by writing one byte to a non-blocking socket. The callback never waits: if the
//!   helper is still busy, the insert outputs silence for that block. The extra block is part
//!   of the reported latency.
//! - **Offline** (export, rendering): [`BridgeInstance::set_offline`] makes every block a round
//!   trip that waits for the helper, so no block is dropped and no latency is added.
//! - **Main-thread calls** (state, parameters, latency, the editor window) go as text lines over
//!   a second socket, with a timeout. The helper shows the plugin's window itself; the app
//!   polls once per frame for its state, unsaved changes and knob moves.
//! - **Crash detection**: the helper exits, a main-thread call times out, or the helper misses
//!   its blocks for a second: the insert falls silent for good and [`BridgeHandle::crashed`]
//!   says so; the app and the other tracks keep running.
//!
//! Linux, macOS and FreeBSD use a socket file; Windows uses loopback TCP with a token
//! ([`wire`]). The web build has no bridge.

pub(crate) mod shm;
mod wire;

use std::{
    path::{Path, PathBuf},
    process::{Child, Command, Stdio},
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, AtomicU32, AtomicU64, Ordering},
    },
    time::{Duration, Instant},
};

use crate::info::{MAX_LATENCY, ParamInfo, PluginInfo};
use shm::{EVENT_MIDI, EVENT_NOTE, EVENT_PARAM, Event, MAX_EVENTS, MAX_FRAMES, Shared};

/// First argument of a helper process.
pub const BRIDGE_FLAG: &str = "--studiocraft-plugin-bridge";

/// How long the helper may take to start and load its plugin.
const START_TIMEOUT: Duration = Duration::from_secs(20);
/// How long a main-thread call may take.
const CALL_TIMEOUT: Duration = Duration::from_secs(5);
/// Largest plugin state carried (bytes).
const MAX_STATE: usize = 64 << 20;

/// What the app and both halves of a bridged instance share.
struct Link {
    shm: Shared,
    /// The helper crashed, hung or was stopped: silence from now on.
    dead: AtomicBool,
    /// Frames of the last block (the extra latency of the bridge).
    last_frames: AtomicU32,
    /// The plugin has a window of its own.
    has_gui: bool,
    /// Its window is open (as of the last poll).
    gui_open: AtomicBool,
    /// Unsaved changes and knob moves from the last poll, until taken.
    polled: Mutex<Polled>,
    /// Main thread only: the control connection and the process.
    main: Mutex<MainSide>,
    /// Events (parameters, MIDI, notes) dropped because a block's event buffer was full.
    dropped: AtomicU64,
}

#[derive(Default)]
struct Polled {
    dirty: bool,
    changes: Vec<(u32, f64)>,
}

/// Most knob moves kept between two takes.
const MAX_CHANGES: usize = 4096;

struct MainSide {
    control: Option<wire::Conn>,
    child: Child,
}

impl Link {
    fn kill(&self) {
        self.dead.store(true, Ordering::Release);
        if let Ok(mut m) = self.main.lock() {
            m.control = None;
            let _ = m.child.kill();
            let _ = m.child.wait();
        }
    }
}

impl Drop for Link {
    fn drop(&mut self) {
        self.kill();
    }
}

/// The audio side of a bridged plugin: lives in the engine like any instance.
pub struct BridgeInstance {
    pub info: PluginInfo,
    pub params: Vec<ParamInfo>,
    link: Arc<Link>,
    wake: wire::Wake,
    /// Number of the last block handed over.
    req: u32,
    /// Whether the helper holds a block whose result has not been taken yet.
    pending: bool,
    /// Frames of that block.
    pending_frames: usize,
    /// Events collected for the next block (allocated once).
    events: Box<[Event]>,
    count: usize,
    /// Frames the helper has been late in a row.
    late: u64,
    /// After this many late frames the helper counts as hung (one second).
    late_limit: u64,
    /// Offline rendering: wait for each block instead of running one behind.
    offline: bool,
}

/// The main-thread side of a bridged plugin.
#[derive(Clone)]
pub struct BridgeHandle {
    link: Arc<Link>,
}

/// Where the helper's files go: the user's runtime folder (private, in memory on Linux),
/// else the temporary folder.
fn runtime_dir() -> PathBuf {
    std::env::var_os("XDG_RUNTIME_DIR")
        .map(PathBuf::from)
        .filter(|d| d.is_dir())
        .unwrap_or_else(std::env::temp_dir)
}

/// A name for this process's next bridge files.
fn next_name() -> String {
    static NEXT: AtomicU32 = AtomicU32::new(0);
    format!(
        "studiocraft-bridge-{}-{}",
        std::process::id(),
        NEXT.fetch_add(1, Ordering::Relaxed)
    )
}

impl BridgeInstance {
    /// Start `exe` (the app's binary) as helper for the plugin `info` and wait until it is
    /// ready. Main thread only.
    pub fn spawn(
        info: &PluginInfo,
        sample_rate: u32,
        max_frames: u32,
        exe: &Path,
    ) -> Result<Self, String> {
        let dir = runtime_dir();
        let name = next_name();
        let shm_path = dir.join(format!("{name}.shm"));
        let sock_path = dir.join(format!("{name}.sock"));
        let shm = Shared::create(&shm_path)?;
        let cleanup = || {
            let _ = std::fs::remove_file(&shm_path);
            let _ = std::fs::remove_file(&sock_path);
        };
        let listener = match wire::Listener::bind(&sock_path) {
            Ok(l) => l,
            Err(e) => {
                cleanup();
                return Err(e);
            }
        };
        let mut cmd = Command::new(exe);
        #[cfg(not(unix))]
        cmd.env(wire::TOKEN_ENV, listener.token());
        cmd.arg(BRIDGE_FLAG)
            .arg(listener.address())
            .arg(&shm_path)
            .arg(info.format.code())
            .arg(&info.path)
            .arg(&info.id)
            .arg(sample_rate.to_string())
            .arg(max_frames.min(MAX_FRAMES as u32).to_string())
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::inherit());
        let mut child = match cmd.spawn() {
            Ok(c) => c,
            Err(e) => {
                cleanup();
                return Err(format!("helper process: {e}"));
            }
        };
        let deadline = Instant::now() + START_TIMEOUT;
        let connected = listener
            .accept(&mut child, deadline)
            .and_then(|control| Ok((control, listener.accept(&mut child, deadline)?)));
        // Both processes have the files open (or failed): the names are no longer needed.
        cleanup();
        let (control, wake) = match connected {
            Ok(c) => c,
            Err(e) => {
                let _ = child.kill();
                let _ = child.wait();
                return Err(e);
            }
        };
        let mut control = wire::Conn::new(control)?;
        let hello = control.read_reply(&mut child, deadline);
        let hello =
            hello.and_then(|lines| Ok((wire::parse_hello(&lines)?, wire::parse_gui(&lines))));
        let (params, has_gui) = match hello {
            Ok(p) => p,
            Err(e) => {
                let _ = child.kill();
                let _ = child.wait();
                return Err(e);
            }
        };
        let wake = wire::Wake::new(wake)?;
        let link = Arc::new(Link {
            shm,
            dead: AtomicBool::new(false),
            last_frames: AtomicU32::new(0),
            has_gui,
            gui_open: AtomicBool::new(false),
            polled: Mutex::new(Polled::default()),
            main: Mutex::new(MainSide {
                control: Some(control),
                child,
            }),
            dropped: AtomicU64::new(0),
        });
        Ok(Self {
            info: info.clone(),
            params,
            link,
            wake,
            req: 0,
            pending: false,
            pending_frames: 0,
            events: vec![Event::default(); MAX_EVENTS].into_boxed_slice(),
            count: 0,
            late: 0,
            late_limit: u64::from(sample_rate.max(1)),
            offline: false,
        })
    }

    pub fn handle(&self) -> BridgeHandle {
        BridgeHandle {
            link: Arc::clone(&self.link),
        }
    }

    /// Into the event buffer of the next block; a full buffer drops the event and counts it
    /// ([`BridgeHandle::dropped_events`]). Never waits, never allocates.
    fn push(&mut self, e: Event) {
        if let Some(slot) = self.events.get_mut(self.count) {
            *slot = e;
            self.count += 1;
        } else {
            self.link.dropped.fetch_add(1, Ordering::Relaxed);
        }
    }

    /// Parameter value from the start of the next block. Called on the audio thread (the
    /// engine's parameter commands) and on the main thread before the instance goes to the
    /// engine, so it only writes the preallocated event buffer: no lock, no allocation, no
    /// socket. A second value for the same parameter before that block replaces the first,
    /// so a fast knob move cannot fill the buffer.
    pub fn set_param(&mut self, id: u32, value: f64) {
        let queued = self
            .events
            .iter_mut()
            .take(self.count)
            .find(|e| e.kind == EVENT_PARAM && e.a == id && e.time == 0);
        match queued {
            Some(e) => e.value = value,
            None => self.set_param_at(0, id, value),
        }
    }

    pub fn set_param_at(&mut self, time: u32, id: u32, value: f64) {
        self.push(Event {
            kind: EVENT_PARAM,
            time,
            a: id,
            b: 0,
            value,
        });
    }

    pub fn midi(&mut self, time: u32, data: [u8; 3]) {
        self.push(Event {
            kind: EVENT_MIDI,
            time,
            a: u32::from_le_bytes([data[0], data[1], data[2], 0]),
            b: 0,
            value: 0.0,
        });
    }

    pub fn note(&mut self, time: u32, key: u8, velocity: f32, on: bool) {
        self.push(Event {
            kind: EVENT_NOTE,
            time,
            a: u32::from(key),
            b: u32::from(on),
            value: f64::from(velocity),
        });
    }

    /// One stereo block in place (audio thread): the result of the previous block comes out,
    /// this block goes to the helper. Never waits, never allocates.
    pub fn process(&mut self, left: &mut [f32], right: &mut [f32]) {
        if self.offline {
            self.process_offline(left, right);
            return;
        }
        let n = left.len().min(right.len()).min(MAX_FRAMES);
        let silence = |l: &mut [f32], r: &mut [f32]| {
            l.fill(0.0);
            r.fill(0.0);
        };
        if self.link.dead.load(Ordering::Acquire) {
            self.count = 0;
            silence(left, right);
            return;
        }
        let r = self.link.shm.region();
        if r.done.load(Ordering::Acquire) != self.req {
            // The helper is still on the last block: silence now, this block is dropped.
            self.count = 0;
            silence(left, right);
            self.late += n as u64;
            if self.late > self.late_limit {
                self.link.dead.store(true, Ordering::Release);
            }
            return;
        }
        self.late = 0;
        self.link.shm.write_input(&left[..n], &right[..n]);
        self.link.shm.write_events(&self.events[..self.count]);
        self.count = 0;
        if self.pending {
            let m = self.pending_frames.min(n);
            self.link.shm.read_output(&mut left[..m], &mut right[..m]);
            silence(&mut left[m..], &mut right[m..]);
        } else {
            silence(left, right);
        }
        r.frames.store(n as u32, Ordering::Relaxed);
        self.req = self.req.wrapping_add(1);
        r.req.store(self.req, Ordering::Release);
        self.pending = true;
        self.pending_frames = n;
        self.link.last_frames.store(n as u32, Ordering::Relaxed);
        self.wake.ring();
    }

    pub fn stop_processing(&mut self) {
        self.link.shm.region().stop.store(1, Ordering::Release);
        self.pending = false;
    }

    /// Offline rendering (export, clip processing; not on the audio thread): every block
    /// waits for the helper's result, so nothing is dropped and the bridge adds no latency.
    /// Call before the first block.
    pub fn set_offline(&mut self, offline: bool) {
        self.offline = offline;
        self.pending = false;
        self.link.last_frames.store(0, Ordering::Relaxed);
    }

    fn process_offline(&mut self, left: &mut [f32], right: &mut [f32]) {
        let n = left.len().min(right.len()).min(MAX_FRAMES);
        let r = self.link.shm.region();
        let ready = |req: u32| -> bool {
            let deadline = Instant::now() + CALL_TIMEOUT;
            while r.done.load(Ordering::Acquire) != req {
                if self.link.dead.load(Ordering::Acquire) || Instant::now() > deadline {
                    return false;
                }
                std::thread::yield_now();
            }
            true
        };
        // A block still running from before the switch is taken first.
        if self.link.dead.load(Ordering::Acquire) || !ready(self.req) {
            self.fail(left, right);
            return;
        }
        self.link.shm.write_input(&left[..n], &right[..n]);
        self.link.shm.write_events(&self.events[..self.count]);
        self.count = 0;
        r.frames.store(n as u32, Ordering::Relaxed);
        self.req = self.req.wrapping_add(1);
        r.req.store(self.req, Ordering::Release);
        self.wake.ring();
        if !ready(self.req) {
            self.fail(left, right);
            return;
        }
        self.link.shm.read_output(&mut left[..n], &mut right[..n]);
        left[n..].fill(0.0);
        right[n..].fill(0.0);
    }

    /// The helper is gone or hung: silence from now on.
    fn fail(&mut self, left: &mut [f32], right: &mut [f32]) {
        self.link.dead.store(true, Ordering::Release);
        self.count = 0;
        left.fill(0.0);
        right.fill(0.0);
    }
}

impl BridgeHandle {
    /// Events dropped so far because a block had more than its buffer holds.
    pub fn dropped_events(&self) -> u64 {
        self.link.dropped.load(Ordering::Relaxed)
    }

    /// The helper crashed, hung or no longer answers: the insert is silent.
    pub fn crashed(&self) -> bool {
        self.check();
        self.link.dead.load(Ordering::Acquire)
    }

    /// Notice a helper that has exited.
    fn check(&self) {
        if self.link.dead.load(Ordering::Acquire) {
            return;
        }
        let exited = self
            .link
            .main
            .lock()
            .map(|mut m| !matches!(m.child.try_wait(), Ok(None)))
            .unwrap_or(true);
        if exited {
            self.link.kill();
        }
    }

    /// One request; its reply lines, or `None` (helper gone, which then counts as crashed).
    fn call(&self, request: &str) -> Option<Vec<String>> {
        if self.crashed() {
            return None;
        }
        let reply = {
            let mut m = self.link.main.lock().ok()?;
            let MainSide { control, child } = &mut *m;
            let c = control.as_mut()?;
            c.send(request)
                .and_then(|()| c.read_reply(child, Instant::now() + CALL_TIMEOUT))
        };
        match reply {
            Ok(lines) => Some(lines),
            Err(_) => {
                self.link.kill();
                None
            }
        }
    }

    pub fn save_state(&self) -> Option<Vec<u8>> {
        let lines = self.call("save")?;
        let hex = lines.first()?.strip_prefix("STATE\t")?;
        wire::unhex(hex, MAX_STATE)
    }

    pub fn load_state(&self, data: &[u8]) -> bool {
        if data.len() > MAX_STATE {
            return false;
        }
        self.call(&format!("load\t{}", wire::hex(data)))
            .is_some_and(|l| l.first().map(String::as_str) == Some("OK"))
    }

    /// The plugin's latency plus the block the bridge adds.
    pub fn latency(&self) -> u32 {
        let plugin = self
            .link
            .shm
            .region()
            .latency
            .load(Ordering::Relaxed)
            .min(MAX_LATENCY);
        plugin.saturating_add(self.link.last_frames.load(Ordering::Relaxed))
    }

    pub fn take_latency_changed(&self) -> bool {
        self.call("latency_changed")
            .is_some_and(|l| l.first().map(String::as_str) == Some("1"))
    }

    pub fn param_values(&self) -> Vec<(u32, f64)> {
        self.call("values")
            .map(|l| wire::parse_values(&l))
            .unwrap_or_default()
    }

    /// Knob moves in the plugin's window since the last take (gathered by [`Self::idle`]).
    pub fn take_param_changes(&self) -> Vec<(u32, f64)> {
        self.link
            .polled
            .lock()
            .map(|mut p| std::mem::take(&mut p.changes))
            .unwrap_or_default()
    }

    /// Unsaved changes reported since the last take (gathered by [`Self::idle`]).
    pub fn take_dirty(&self) -> bool {
        self.link
            .polled
            .lock()
            .map(|mut p| std::mem::take(&mut p.dirty))
            .unwrap_or(false)
    }

    pub fn has_gui(&self) -> bool {
        self.link.has_gui
    }

    pub fn gui_open(&self) -> bool {
        self.link.gui_open.load(Ordering::Relaxed)
    }

    /// Open the plugin's window; the helper shows it (above the app's main window where the
    /// system allows).
    pub fn open_gui(&self, title: &str) -> Result<(), String> {
        let window = crate::gui::main_window_code();
        let lines = self
            .call(&format!("gui_open\t{window}\t{}", wire::escape(title)))
            .ok_or("the plugin's process does not answer")?;
        match lines.first().map(String::as_str) {
            Some("OK") => {
                self.link.gui_open.store(true, Ordering::Relaxed);
                Ok(())
            }
            Some(l) => Err(wire::unescape(l.strip_prefix("FAIL\t").unwrap_or(l))),
            None => Err("no answer from the plugin's process".into()),
        }
    }

    pub fn close_gui(&self) {
        let _ = self.call("gui_close");
        self.link.gui_open.store(false, Ordering::Relaxed);
    }

    /// Once per frame: window state, unsaved changes and knob moves from the helper. `true`
    /// while the window is open.
    pub fn idle(&self) -> bool {
        let Some(lines) = self.call("poll") else {
            self.link.gui_open.store(false, Ordering::Relaxed);
            return false;
        };
        let flag = |name: &str| lines.iter().any(|l| *l == format!("{name}\t1"));
        let open = flag("OPEN");
        self.link.gui_open.store(open, Ordering::Relaxed);
        if let Ok(mut p) = self.link.polled.lock() {
            p.dirty |= flag("DIRTY");
            let room = MAX_CHANGES.saturating_sub(p.changes.len());
            p.changes
                .extend(wire::parse_values(&lines).into_iter().take(room));
        }
        open
    }

    /// A value the host changed, for the open window (no answer awaited).
    pub fn set_ui_value(&self, id: u32, value: f64) {
        if !value.is_finite() || self.crashed() {
            return;
        }
        if let Ok(mut m) = self.link.main.lock()
            && let Some(c) = m.control.as_mut()
        {
            let _ = c.send(&format!("ui\t{id}\t{value}"));
        }
    }
}

/// What a helper hosts: the audio half (moved to the helper's audio thread)…
pub trait BridgedAudio: Send {
    fn process(&mut self, left: &mut [f32], right: &mut [f32]);
    fn set_param(&mut self, id: u32, value: f64);
    fn set_param_at(&mut self, time: u32, id: u32, value: f64);
    fn midi(&mut self, time: u32, data: [u8; 3]);
    fn note(&mut self, time: u32, key: u8, velocity: f32, on: bool);
    fn stop_processing(&mut self);
}

/// …and the main-thread half.
pub trait BridgedMain {
    fn save_state(&self) -> Option<Vec<u8>>;
    fn load_state(&self, data: &[u8]) -> bool;
    fn latency(&self) -> u32;
    fn take_latency_changed(&self) -> bool;
    fn param_values(&self) -> Vec<(u32, f64)>;
    fn take_param_changes(&self) -> Vec<(u32, f64)>;
    fn take_dirty(&self) -> bool;
    /// Main-thread work of the plugin between requests (callbacks, timers, its window);
    /// `true` while its window is open.
    fn idle(&mut self) -> bool {
        false
    }
    fn has_gui(&self) -> bool {
        false
    }
    fn gui_open(&self) -> bool {
        false
    }
    fn open_gui(&mut self, _title: &str) -> Result<(), String> {
        Err("this plugin has no window".into())
    }
    fn close_gui(&mut self) {}
    /// A value the host changed, for the open window.
    fn set_ui_value(&self, _id: u32, _value: f64) {}
}

/// A loaded plugin for a helper: its parameters and both halves.
pub type Hosted = (Vec<ParamInfo>, Box<dyn BridgedAudio>, Box<dyn BridgedMain>);

/// The arguments a helper was started with.
pub struct BridgeArgs {
    pub info: PluginInfo,
    pub sample_rate: u32,
    pub max_frames: u32,
    socket: PathBuf,
    shm: PathBuf,
}

/// Helper process: if `args[1]` is [`BRIDGE_FLAG`], host the plugin named by the arguments
/// until the app goes away and return the exit code; else `None` (normal start).
pub fn run_bridge_child(args: &[String]) -> Option<i32> {
    run_bridge_child_with(args, load_plugin)
}

/// As [`run_bridge_child`] with another loader (tests host fake plugins this way).
pub fn run_bridge_child_with(
    args: &[String],
    load: impl FnOnce(&BridgeArgs) -> Result<Hosted, String>,
) -> Option<i32> {
    if args.get(1).map(String::as_str) != Some(BRIDGE_FLAG) {
        return None;
    }
    Some(match child::run(args, load) {
        Ok(()) => 0,
        Err(e) => {
            eprintln!("plugin bridge: {e}");
            1
        }
    })
}

/// The usual loader: the plugin from its file, in its format.
fn load_plugin(a: &BridgeArgs) -> Result<Hosted, String> {
    let instance = crate::PluginInstance::new(&a.info, a.sample_rate, a.max_frames)?;
    let params = instance.params().to_vec();
    let handle = instance.handle();
    Ok((params, Box::new(instance), Box::new(handle)))
}

mod child;
