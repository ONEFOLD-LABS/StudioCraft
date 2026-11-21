//! The helper process: loads one plugin, answers the app's control requests on its main
//! thread and processes the blocks the app hands over on an audio thread of its own. It ends
//! when the app closes the connection (or dies).

use std::{
    io::{BufRead, BufReader, Read, Write},
    path::PathBuf,
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
        mpsc,
    },
};

use super::{
    BridgeArgs, BridgedAudio, BridgedMain, Hosted, MAX_STATE,
    shm::{EVENT_MIDI, EVENT_NOTE, EVENT_PARAM, MAX_FRAMES, Shared},
    wire::{self, Stream},
};
use crate::info::{PluginFormat, PluginInfo};

/// Longest request line (a state as hex, with room).
const MAX_REQUEST: u64 = 140 << 20;

fn parse(args: &[String]) -> Result<BridgeArgs, String> {
    let arg = |i: usize| args.get(i).ok_or("missing argument");
    let format = PluginFormat::from_code(arg(4)?).ok_or("unknown plugin format")?;
    let num =
        |i: usize| -> Result<u32, String> { arg(i)?.parse().map_err(|_| "bad number".to_owned()) };
    let sample_rate = num(7)?;
    let max_frames = num(8)?;
    if !(1_000..=1_536_000).contains(&sample_rate) || max_frames == 0 {
        return Err("bad sample rate or block size".into());
    }
    Ok(BridgeArgs {
        socket: PathBuf::from(arg(2)?),
        shm: PathBuf::from(arg(3)?),
        info: PluginInfo {
            format,
            path: PathBuf::from(arg(5)?),
            id: arg(6)?.clone(),
            ..PluginInfo::default()
        },
        sample_rate,
        max_frames: max_frames.min(MAX_FRAMES as u32),
    })
}

#[cfg(unix)]
fn connect(path: &std::path::Path) -> Result<Stream, String> {
    Stream::connect(path).map_err(|e| format!("{}: {e}", path.display()))
}

#[cfg(not(unix))]
fn connect(_: &std::path::Path) -> Result<Stream, String> {
    Err("the plugin bridge is not available on this system yet".into())
}

pub(super) fn run(
    args: &[String],
    load: impl FnOnce(&BridgeArgs) -> Result<Hosted, String>,
) -> Result<(), String> {
    let a = parse(args)?;
    let control = connect(&a.socket)?;
    let wake = connect(&a.socket)?;
    let shm = Shared::open(&a.shm)?;
    let mut out = control.try_clone().map_err(|e| e.to_string())?;
    let (params, audio, mut main) = match load(&a) {
        Ok(h) => h,
        Err(e) => {
            let _ = writeln!(out, "ERR\t{}", wire::escape(&e));
            return Err(e);
        }
    };
    for p in &params {
        writeln!(out, "{}", wire::param_line(p)).map_err(|e| e.to_string())?;
    }
    writeln!(out, "GUI\t{}", u8::from(main.has_gui())).map_err(|e| e.to_string())?;
    writeln!(out, "END").map_err(|e| e.to_string())?;
    let shm = Arc::new(shm);
    shm.region()
        .latency
        .store(main.latency(), Ordering::Relaxed);

    // Main-thread parameter changes for the audio thread (`set`).
    let (to_audio, from_main) = mpsc::channel::<(u32, f64)>();
    let quit = Arc::new(AtomicBool::new(false));
    let unblock = wake.try_clone().map_err(|e| e.to_string())?;
    let audio_thread = {
        let shm = Arc::clone(&shm);
        let quit = Arc::clone(&quit);
        std::thread::Builder::new()
            .name("bridge-audio".into())
            .spawn(move || audio_loop(&shm, wake, audio, &from_main, &quit, a.max_frames))
            .map_err(|e| e.to_string())?
    };
    let result = control_loop(control, &mut out, main.as_mut(), &to_audio, &shm);
    quit.store(true, Ordering::Release);
    // The handle goes before the instance (it ends with the audio thread).
    drop(main);
    drop(to_audio);
    let _ = unblock.shutdown(std::net::Shutdown::Both);
    let _ = audio_thread.join();
    result
}

fn audio_loop(
    shm: &Shared,
    mut wake: Stream,
    mut audio: Box<dyn BridgedAudio>,
    from_main: &mpsc::Receiver<(u32, f64)>,
    quit: &AtomicBool,
    max_frames: u32,
) {
    let max = max_frames as usize;
    let (mut left, mut right) = (vec![0.0f32; max], vec![0.0f32; max]);
    let mut byte = [0u8; 64];
    let r = shm.region();
    let mut done = r.done.load(Ordering::Acquire);
    while !quit.load(Ordering::Acquire) {
        match wake.read(&mut byte) {
            Ok(0) | Err(_) => return, // the app went away
            Ok(_) => {}
        }
        if r.stop.swap(0, Ordering::AcqRel) != 0 {
            audio.stop_processing();
        }
        while let Ok((id, v)) = from_main.try_recv() {
            audio.set_param(id, v);
        }
        let req = r.req.load(Ordering::Acquire);
        if req == done {
            continue;
        }
        let n = shm.read_block(&mut left, &mut right, |e| match e.kind {
            EVENT_PARAM => audio.set_param_at(e.time, e.a, e.value),
            EVENT_MIDI => {
                let b = e.a.to_le_bytes();
                audio.midi(e.time, [b[0], b[1], b[2]]);
            }
            EVENT_NOTE => audio.note(
                e.time,
                u8::try_from(e.a.min(127)).unwrap_or(0),
                e.value as f32,
                e.b != 0,
            ),
            _ => {}
        });
        audio.process(&mut left[..n], &mut right[..n]);
        shm.write_output(&left[..n], &right[..n]);
        done = req;
        r.done.store(req, Ordering::Release);
    }
}

fn control_loop(
    control: Stream,
    out: &mut Stream,
    main: &mut dyn BridgedMain,
    to_audio: &mpsc::Sender<(u32, f64)>,
    shm: &Shared,
) -> Result<(), String> {
    // Short reads, so the plugin's main-thread work (callbacks, timers) runs in between.
    control
        .set_read_timeout(Some(std::time::Duration::from_millis(15)))
        .map_err(|e| e.to_string())?;
    let mut reader = BufReader::new(control);
    let mut pending = String::new();
    // The plugin's main-thread work runs at least this often, also between busy requests.
    let every = std::time::Duration::from_millis(15);
    let mut last_idle = std::time::Instant::now();
    let mut open = false;
    loop {
        if last_idle.elapsed() >= every {
            open = main.idle();
            last_idle = std::time::Instant::now();
        }
        match (&mut reader).take(MAX_REQUEST).read_line(&mut pending) {
            Ok(0) => return Ok(()), // the app closed the connection
            Ok(_) if pending.ends_with('\n') => {}
            Ok(_) => return Err("request line too long".into()),
            Err(e)
                if matches!(
                    e.kind(),
                    std::io::ErrorKind::WouldBlock | std::io::ErrorKind::TimedOut
                ) =>
            {
                open = main.idle();
                last_idle = std::time::Instant::now();
                continue;
            }
            Err(e) => return Err(e.to_string()),
        }
        let full = std::mem::take(&mut pending);
        let line = full.trim_end_matches('\n');
        let (cmd, rest) = line.split_once('\t').unwrap_or((line, ""));
        let reply: Vec<String> = match cmd {
            "save" => vec![match main.save_state() {
                Some(s) if s.len() <= MAX_STATE => format!("STATE\t{}", wire::hex(&s)),
                _ => "NONE".into(),
            }],
            "load" => {
                let ok = wire::unhex(rest, MAX_STATE).is_some_and(|d| main.load_state(&d));
                vec![if ok { "OK" } else { "FAIL" }.into()]
            }
            "latency_changed" => {
                let changed = main.take_latency_changed();
                shm.region()
                    .latency
                    .store(main.latency(), Ordering::Relaxed);
                vec![if changed { "1" } else { "0" }.into()]
            }
            "values" => wire::values_lines(&main.param_values()),
            "changes" => wire::values_lines(&main.take_param_changes()),
            "dirty" => vec![if main.take_dirty() { "1" } else { "0" }.into()],
            // Once per frame of the app: window state, unsaved changes, knob moves.
            "poll" => {
                let mut l = vec![
                    format!("OPEN\t{}", u8::from(open && main.gui_open())),
                    format!("DIRTY\t{}", u8::from(main.take_dirty())),
                ];
                l.extend(wire::values_lines(&main.take_param_changes()));
                l
            }
            "gui_open" => {
                let (window, title) = rest.split_once('\t').unwrap_or(("", rest));
                crate::gui::set_main_window_code(window);
                match main.open_gui(&wire::unescape(title)) {
                    Ok(()) => {
                        open = true;
                        vec!["OK".into()]
                    }
                    // Not ERR: that would count as a broken helper.
                    Err(e) => vec![format!("FAIL\t{}", wire::escape(&e))],
                }
            }
            "gui_close" => {
                main.close_gui();
                open = false;
                vec!["OK".into()]
            }
            "ui" => {
                let mut f = rest.split('\t');
                if let (Some(Ok(id)), Some(Ok(v))) = (
                    f.next().map(str::parse::<u32>),
                    f.next().map(str::parse::<f64>),
                ) && v.is_finite()
                {
                    main.set_ui_value(id, v);
                }
                continue; // no reply
            }
            "set" => {
                let mut f = rest.split('\t');
                if let (Some(Ok(id)), Some(Ok(v))) = (
                    f.next().map(str::parse::<u32>),
                    f.next().map(str::parse::<f64>),
                ) && v.is_finite()
                {
                    let _ = to_audio.send((id, v));
                }
                continue; // no reply
            }
            other => {
                writeln!(out, "ERR\tunknown request {}", wire::escape(other))
                    .map_err(|e| e.to_string())?;
                continue;
            }
        };
        for l in reply {
            writeln!(out, "{l}").map_err(|e| e.to_string())?;
        }
        writeln!(out, "END").map_err(|e| e.to_string())?;
    }
}

// The usual instance hosted in a helper: both halves are the existing types.
impl BridgedAudio for crate::PluginInstance {
    fn process(&mut self, left: &mut [f32], right: &mut [f32]) {
        crate::PluginInstance::process(self, left, right);
    }
    fn set_param(&mut self, id: u32, value: f64) {
        crate::PluginInstance::set_param(self, id, value);
    }
    fn set_param_at(&mut self, time: u32, id: u32, value: f64) {
        crate::PluginInstance::set_param_at(self, time, id, value);
    }
    fn midi(&mut self, time: u32, data: [u8; 3]) {
        crate::PluginInstance::midi(self, time, data);
    }
    fn note(&mut self, time: u32, key: u8, velocity: f32, on: bool) {
        crate::PluginInstance::note(self, time, key, velocity, on);
    }
    fn stop_processing(&mut self) {
        crate::PluginInstance::stop_processing(self);
    }
}

impl BridgedMain for crate::PluginHandle {
    fn save_state(&self) -> Option<Vec<u8>> {
        crate::PluginHandle::save_state(self)
    }
    fn load_state(&self, data: &[u8]) -> bool {
        crate::PluginHandle::load_state(self, data)
    }
    fn latency(&self) -> u32 {
        crate::PluginHandle::latency(self)
    }
    fn take_latency_changed(&self) -> bool {
        crate::PluginHandle::take_latency_changed(self)
    }
    fn param_values(&self) -> Vec<(u32, f64)> {
        crate::PluginHandle::param_values(self)
    }
    fn take_param_changes(&self) -> Vec<(u32, f64)> {
        crate::PluginHandle::take_param_changes(self)
    }
    fn take_dirty(&self) -> bool {
        crate::PluginHandle::take_dirty(self)
    }
    fn idle(&mut self) -> bool {
        crate::PluginHandle::idle(self)
    }
    fn has_gui(&self) -> bool {
        crate::PluginHandle::has_gui(self)
    }
    fn gui_open(&self) -> bool {
        crate::PluginHandle::gui_open(self)
    }
    fn open_gui(&mut self, title: &str) -> Result<(), String> {
        crate::PluginHandle::open_gui(self, title)
    }
    fn close_gui(&mut self) {
        crate::PluginHandle::close_gui(self);
    }
    fn set_ui_value(&self, id: u32, value: f64) {
        crate::PluginHandle::set_ui_value(self, id, value);
    }
}
