//! The plugin bridge (Issue #146) with fake plugins in real helper processes: this test
//! program starts itself as the helper, as the app does.
//!
//! - `test:gain` multiplies by parameter 0 (state: the gain as 8 bytes; latency 7 frames)
//! - `test:crash` aborts in its third block
//! - `test:hang` stops answering in its third block
//! - `test:fail` cannot be loaded
//! - `test:slow` is a gain whose state takes two seconds to save (the control connection stays
//!   busy meanwhile)
//! - `test:window` has a fake window: once open it reports a knob move and unsaved changes,
//!   echoes host values as moves of parameter 1 and closes itself after 40 idle passes (as if
//!   the user closed it); the title `refuse` makes opening fail

use std::{
    path::PathBuf,
    sync::{
        Arc, Mutex,
        atomic::{AtomicU64, Ordering},
    },
    time::{Duration, Instant},
};

use studiocraft_plugin_host::{
    BRIDGE_FLAG, BridgeArgs, BridgeInstance, BridgedAudio, BridgedMain, Hosted, ParamInfo,
    PluginFormat, PluginInfo, default_paths, run_bridge_child, run_bridge_child_with, scan,
};

struct Fake {
    kind: String,
    gain: Arc<AtomicU64>,
    blocks: u32,
}

impl BridgedAudio for Fake {
    fn process(&mut self, left: &mut [f32], right: &mut [f32]) {
        self.blocks += 1;
        if self.blocks == 3 {
            match self.kind.as_str() {
                "test:crash" => std::process::abort(),
                "test:hang" => loop {
                    std::thread::sleep(Duration::from_secs(60));
                },
                _ => {}
            }
        }
        let g = f64::from_bits(self.gain.load(Ordering::Relaxed)) as f32;
        for s in left.iter_mut().chain(right.iter_mut()) {
            *s *= g;
        }
    }
    fn set_param(&mut self, _id: u32, value: f64) {
        self.gain.store(value.to_bits(), Ordering::Relaxed);
    }
    fn set_param_at(&mut self, _time: u32, _id: u32, value: f64) {
        self.gain.store(value.to_bits(), Ordering::Relaxed);
    }
    fn midi(&mut self, _: u32, _: [u8; 3]) {}
    fn note(&mut self, _: u32, _: u8, _: f32, _: bool) {}
    fn stop_processing(&mut self) {}
}

struct FakeMain {
    gain: Arc<AtomicU64>,
    slow: bool,
    window: bool,
    win: Mutex<FakeWindow>,
}

#[derive(Default)]
struct FakeWindow {
    open: bool,
    ticks: u32,
    dirty: bool,
    changes: Vec<(u32, f64)>,
}

impl BridgedMain for FakeMain {
    fn save_state(&self) -> Option<Vec<u8>> {
        if self.slow {
            std::thread::sleep(Duration::from_secs(2));
        }
        Some(self.gain.load(Ordering::Relaxed).to_le_bytes().to_vec())
    }
    fn load_state(&self, data: &[u8]) -> bool {
        let Ok(b) = <[u8; 8]>::try_from(data) else {
            return false;
        };
        self.gain.store(u64::from_le_bytes(b), Ordering::Relaxed);
        true
    }
    fn latency(&self) -> u32 {
        7
    }
    fn take_latency_changed(&self) -> bool {
        false
    }
    fn param_values(&self) -> Vec<(u32, f64)> {
        vec![(0, f64::from_bits(self.gain.load(Ordering::Relaxed)))]
    }
    fn take_param_changes(&self) -> Vec<(u32, f64)> {
        std::mem::take(&mut self.win.lock().unwrap().changes)
    }
    fn take_dirty(&self) -> bool {
        std::mem::take(&mut self.win.lock().unwrap().dirty)
    }
    fn idle(&mut self) -> bool {
        let w = self.win.get_mut().unwrap();
        if w.open {
            w.ticks += 1;
            if w.ticks == 3 {
                w.changes.push((0, 0.5));
                w.dirty = true;
            }
            if w.ticks >= 40 {
                w.open = false;
            }
        }
        w.open
    }
    fn has_gui(&self) -> bool {
        self.window
    }
    fn gui_open(&self) -> bool {
        self.win.lock().unwrap().open
    }
    fn open_gui(&mut self, title: &str) -> Result<(), String> {
        if title == "refuse" {
            return Err("no display for the window".into());
        }
        *self.win.get_mut().unwrap() = FakeWindow {
            open: true,
            ..FakeWindow::default()
        };
        Ok(())
    }
    fn close_gui(&mut self) {
        self.win.get_mut().unwrap().open = false;
    }
    fn set_ui_value(&self, _id: u32, value: f64) {
        self.win.lock().unwrap().changes.push((1, value));
    }
}

fn load(a: &BridgeArgs) -> Result<Hosted, String> {
    if a.info.id == "test:fail" {
        return Err("this plugin does not load".into());
    }
    let gain = Arc::new(AtomicU64::new(1.0f64.to_bits()));
    let param = ParamInfo {
        id: 0,
        name: "Gain".into(),
        module: String::new(),
        min: 0.0,
        max: 4.0,
        default: 1.0,
        value: 1.0,
        stepped: false,
    };
    Ok((
        vec![param],
        Box::new(Fake {
            kind: a.info.id.clone(),
            gain: Arc::clone(&gain),
            blocks: 0,
        }),
        Box::new(FakeMain {
            gain,
            slow: a.info.id == "test:slow",
            window: a.info.id == "test:window",
            win: Mutex::default(),
        }),
    ))
}

fn info(id: &str) -> PluginInfo {
    PluginInfo {
        format: PluginFormat::Clap,
        id: id.into(),
        name: id.into(),
        path: PathBuf::from("/nonexistent/fake.clap"),
        ..PluginInfo::default()
    }
}

fn exe() -> PathBuf {
    std::env::current_exe().expect("own path")
}

/// One block of `value`, then time for the helper to finish it.
fn block(b: &mut BridgeInstance, value: f32, frames: usize) -> (Vec<f32>, Vec<f32>) {
    let (mut l, mut r) = (vec![value; frames], vec![value; frames]);
    b.process(&mut l, &mut r);
    std::thread::sleep(Duration::from_millis(40));
    (l, r)
}

fn gain_round_trip() {
    let mut b = BridgeInstance::spawn(&info("test:gain"), 48_000, 512, &exe()).expect("spawn");
    let h = b.handle();
    assert_eq!(b.params.len(), 1);
    assert_eq!(b.params[0].name, "Gain");

    // One block behind: the first block comes back silent, then each block's result follows.
    let (l, _) = block(&mut b, 1.0, 64);
    assert!(l.iter().all(|&s| s == 0.0), "first block silent");
    b.set_param_at(0, 0, 2.0);
    let (l, r) = block(&mut b, 0.5, 64);
    assert!(
        l.iter().all(|&s| s == 1.0) && r.iter().all(|&s| s == 1.0),
        "{:?}",
        &l[..4]
    );
    let (l, _) = block(&mut b, 0.0, 64);
    assert!(l.iter().all(|&s| s == 1.0), "0.5 × 2: {:?}", &l[..4]);
    assert_eq!(h.latency(), 7 + 64, "plugin latency plus one block");

    // State through the bridge.
    let state = h.save_state().expect("state");
    assert_eq!(state, 2.0f64.to_le_bytes());
    assert!(h.load_state(&0.25f64.to_le_bytes()));
    assert_eq!(h.param_values(), [(0, 0.25)]);
    assert!(!h.load_state(b"short"));
    assert!(!h.crashed());
    println!("gain: one block behind, parameters, state and latency through the bridge");
}

fn offline_waits_for_every_block() {
    let mut b = BridgeInstance::spawn(&info("test:gain"), 48_000, 512, &exe()).expect("spawn");
    let h = b.handle();
    b.set_offline(true);
    b.set_param_at(0, 0, 3.0);
    // Faster than real time and without pauses: every block comes back at once, none dropped.
    for k in 0..200 {
        let v = (k % 7) as f32 * 0.1;
        let (mut l, mut r) = (vec![v; 256], vec![v; 256]);
        b.process(&mut l, &mut r);
        assert!(
            l.iter().chain(&r).all(|&s| (s - v * 3.0).abs() < 1e-6),
            "block {k}: {:?}",
            &l[..2]
        );
    }
    assert_eq!(h.latency(), 7, "no extra block offline");
    assert!(!h.crashed());
    println!("offline: 200 blocks without a pause, each result in its own block");
}

/// Poll like the app's frames until `done` holds (at most two seconds).
fn poll_until(h: &studiocraft_plugin_host::BridgeHandle, done: impl Fn(bool) -> bool) -> bool {
    let t = Instant::now();
    while t.elapsed() < Duration::from_secs(2) {
        if done(h.idle()) {
            return true;
        }
        std::thread::sleep(Duration::from_millis(16));
    }
    false
}

/// Parameter changes reach the helper through the block's event buffer, never the control
/// connection: they do not wait while the main thread talks to the helper, a fast knob move
/// keeps only its last value, and a full buffer is counted instead of blocking.
fn set_param_never_waits() {
    let mut b = BridgeInstance::spawn(&info("test:slow"), 48_000, 512, &exe()).expect("spawn");
    let h = b.handle();
    block(&mut b, 1.0, 64);
    // The main thread holds the control connection for two seconds.
    let saver = {
        let h = h.clone();
        std::thread::spawn(move || h.save_state())
    };
    std::thread::sleep(Duration::from_millis(200));
    let t = Instant::now();
    for k in 0..10_000 {
        b.set_param(0, f64::from(k % 3));
    }
    b.set_param(0, 2.0);
    let took = t.elapsed();
    assert!(
        took < Duration::from_millis(100),
        "set_param waited for the control connection: {took:?}"
    );
    assert_eq!(h.dropped_events(), 0, "one queued value per parameter");
    block(&mut b, 0.5, 64);
    let (l, _) = block(&mut b, 0.0, 64);
    assert!(l.iter().all(|&s| s == 1.0), "0.5 × 2: {:?}", &l[..4]);
    // More distinct events than a block holds: the rest is dropped and counted.
    for k in 0..5_000u32 {
        b.set_param_at(k % 64, 0, 1.0);
    }
    assert!(h.dropped_events() > 0, "overflow is counted");
    assert!(saver.join().expect("saver").is_some());
    assert!(!h.crashed());
    println!("set_param: no wait while the main thread talks to the helper, overflow counted");
}

fn window_through_the_bridge() {
    let b = BridgeInstance::spawn(&info("test:window"), 48_000, 512, &exe()).expect("spawn");
    let h = b.handle();
    assert!(h.has_gui() && !h.gui_open());
    let e = h.open_gui("refuse").expect_err("refused");
    assert!(e.contains("no display"), "{e}");
    assert!(!h.crashed(), "a refused window is not a crash");

    h.open_gui("Spur – Fenster").expect("open");
    assert!(h.gui_open());
    // The knob move and the unsaved change arrive with the polls.
    assert!(poll_until(&h, |_| h
        .take_param_changes()
        .contains(&(0, 0.5))));
    assert!(h.take_dirty() || poll_until(&h, |_| h.take_dirty()));
    h.set_ui_value(0, 0.75);
    assert!(poll_until(&h, |_| h
        .take_param_changes()
        .contains(&(1, 0.75))));
    // Closed in the helper (by the user): the app sees it.
    assert!(poll_until(&h, |open| !open), "closed by the user");
    assert!(!h.gui_open());
    h.open_gui("again").expect("open again");
    h.close_gui();
    assert!(!h.gui_open() && !h.idle());

    let plain = BridgeInstance::spawn(&info("test:gain"), 48_000, 512, &exe()).expect("spawn");
    assert!(!plain.handle().has_gui());
    println!("window: opened, knob moves and host values both ways, closed in the helper");
}

fn crash_silences_only_the_insert() {
    let mut b = BridgeInstance::spawn(&info("test:crash"), 48_000, 512, &exe()).expect("spawn");
    let h = b.handle();
    for _ in 0..2 {
        block(&mut b, 1.0, 64);
    }
    // Third block: the helper aborts.
    block(&mut b, 1.0, 64);
    let t = Instant::now();
    while !h.crashed() && t.elapsed() < Duration::from_secs(5) {
        std::thread::sleep(Duration::from_millis(20));
    }
    assert!(h.crashed(), "the crash is noticed");
    let (l, r) = block(&mut b, 1.0, 64);
    assert!(
        l.iter().chain(&r).all(|&s| s == 0.0),
        "silence after the crash"
    );
    assert!(h.save_state().is_none());
    println!("crash: noticed, the insert is silent, the host keeps running");
}

fn hang_counts_as_crash() {
    // 1 kHz: one second is 1000 frames, so 16 late blocks of 64 frames.
    let mut b = BridgeInstance::spawn(&info("test:hang"), 1_000, 64, &exe()).expect("spawn");
    let h = b.handle();
    let mut silent_and_quick = true;
    for _ in 0..40 {
        let t = Instant::now();
        let (mut l, mut r) = (vec![1.0; 64], vec![1.0; 64]);
        b.process(&mut l, &mut r);
        // The callback never waits for the helper.
        silent_and_quick &= t.elapsed() < Duration::from_millis(20);
        std::thread::sleep(Duration::from_millis(5));
    }
    assert!(silent_and_quick, "process never blocks");
    assert!(h.crashed(), "a helper late for a second counts as crashed");
    println!("hang: the callback did not wait; the insert went silent after one second");
}

fn load_error_is_reported() {
    let e = BridgeInstance::spawn(&info("test:fail"), 48_000, 512, &exe())
        .err()
        .expect("error");
    assert!(e.contains("does not load"), "{e}");
    println!("fail: the helper's error reaches the app");
}

fn main() {
    let args: Vec<String> = std::env::args().collect();
    if args.get(1).map(String::as_str) == Some(BRIDGE_FLAG) {
        // Fake plugins by id; anything else is a real plugin, loaded as the app does.
        let fake = args.get(6).is_some_and(|id| id.starts_with("test:"));
        let code = if fake {
            run_bridge_child_with(&args, load)
        } else {
            run_bridge_child(&args)
        };
        std::process::exit(code.unwrap_or(1));
    }
    gain_round_trip();
    offline_waits_for_every_block();
    set_param_never_waits();
    window_through_the_bridge();
    crash_silences_only_the_insert();
    hang_counts_as_crash();
    load_error_is_reported();
    if let Ok(name) = std::env::var("STUDIOCRAFT_BRIDGE_PLUGIN") {
        real_plugin(&name);
    }
}

/// An installed plugin through the bridge (`STUDIOCRAFT_BRIDGE_PLUGIN=<name as listed>`):
/// signal goes through, state comes back, the helper is stopped with the instance.
fn real_plugin(name: &str) {
    let plugins = scan(&default_paths()).0;
    let Some(info) = plugins.iter().find(|p| p.display_name() == name) else {
        println!("{name}: not installed");
        return;
    };
    let mut b = BridgeInstance::spawn(info, 48_000, 512, &exe()).expect("spawn");
    let h = b.handle();
    let mut energy = 0.0f32;
    for i in 0..50 {
        let mut l: Vec<f32> = (0..256)
            .map(|n| ((i * 256 + n) as f32 * 0.05).sin() * 0.3)
            .collect();
        let mut r = l.clone();
        b.process(&mut l, &mut r);
        energy += l.iter().map(|s| s * s).sum::<f32>();
        std::thread::sleep(Duration::from_millis(6));
    }
    assert!(energy > 0.0, "{name}: signal through the bridge");
    let state = h.save_state();
    println!(
        "{name}: {} parameters, latency {}, state {} bytes, energy {energy:.1}, window {}",
        b.params.len(),
        h.latency(),
        state.as_ref().map_or(0, Vec::len),
        h.has_gui()
    );
    if let Some(s) = state {
        assert!(h.load_state(&s), "{name}: state loads back");
    }
    // With a display (`STUDIOCRAFT_GUI_TEST=1`): the editor opens from the helper.
    if std::env::var_os("STUDIOCRAFT_GUI_TEST").is_some() && h.has_gui() {
        h.open_gui(&format!("{name} (bridge)"))
            .expect("editor opens");
        let shown = poll_until(&h, |open| !open);
        assert!(!shown, "{name}: the editor stays open");
        h.close_gui();
        assert!(!h.idle(), "{name}: closed");
        println!("{name}: editor opened and closed through the bridge");
    }
    assert!(!h.crashed());
}
