//! The audio callback must not allocate (AGENTS.md, real-time audio rules).
//!
//! A counting global allocator (only in this test binary) counts allocations, reallocations
//! and frees made on this thread while `EngineCore::process` runs. The project uses the
//! features that reach the audio thread: audio events with fades, a musical (stretched) event,
//! an audio part, a video track with sound, MIDI notes, groups, sends, EQ, automation, a tempo
//! change, metronome, cycle, locating, starting and stopping, and swapping the song.

// The helpers outside the #[test] functions may unwrap and panic like the tests themselves
// (Cargo.toml lints, clippy.toml).
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use std::alloc::{GlobalAlloc, Layout, System};
use std::cell::Cell;
use std::collections::HashMap;
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};

use studiocraft_engine::{Command, Song, engine};
use studiocraft_project::{
    AutoParam, AutoPoint, AutomationLane, ClipId, PoolClip, Project, SendSlot, TempoEvent,
    TrackKind,
};

struct Counting;

static COUNT: AtomicUsize = AtomicUsize::new(0);

thread_local! {
    static ARMED: Cell<bool> = const { Cell::new(false) };
}

fn note() {
    if ARMED.with(Cell::get) {
        COUNT.fetch_add(1, Ordering::Relaxed);
    }
}

// SAFETY: forwards every call unchanged to the system allocator; counting touches only an
// atomic and a const-initialised thread-local, neither of which allocates.
unsafe impl GlobalAlloc for Counting {
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        note();
        // SAFETY: same contract as the caller's.
        unsafe { System.alloc(layout) }
    }

    unsafe fn alloc_zeroed(&self, layout: Layout) -> *mut u8 {
        note();
        // SAFETY: same contract as the caller's.
        unsafe { System.alloc_zeroed(layout) }
    }

    unsafe fn dealloc(&self, ptr: *mut u8, layout: Layout) {
        note();
        // SAFETY: same contract as the caller's.
        unsafe { System.dealloc(ptr, layout) }
    }

    unsafe fn realloc(&self, ptr: *mut u8, layout: Layout, new_size: usize) -> *mut u8 {
        note();
        // SAFETY: same contract as the caller's.
        unsafe { System.realloc(ptr, layout, new_size) }
    }
}

#[global_allocator]
static GLOBAL: Counting = Counting;

/// Allocations, reallocations and frees on this thread while `f` runs.
fn count(f: impl FnOnce()) -> usize {
    let before = COUNT.load(Ordering::Relaxed);
    ARMED.with(|a| a.set(true));
    f();
    ARMED.with(|a| a.set(false));
    COUNT.load(Ordering::Relaxed) - before
}

const RATE: u32 = 48_000;
const Q: u64 = 960;

fn tone(frames: usize, hz: f32) -> Arc<[f32]> {
    (0..frames)
        .flat_map(|i| {
            let v = (i as f32 * hz * std::f32::consts::TAU / RATE as f32).sin() * 0.5;
            [v, v]
        })
        .collect()
}

fn pool(
    p: &mut Project,
    clips: &mut HashMap<ClipId, Arc<[f32]>>,
    name: &str,
    frames: usize,
) -> ClipId {
    let c = p.add_clip(PoolClip {
        name: name.into(),
        path: None,
        sample_rate: RATE,
        frames: frames as u64,
    });
    clips.insert(c, tone(frames, 220.0));
    c
}

/// A project that exercises every path of the audio thread that needs no plugin or device.
fn busy_project() -> (Project, HashMap<ClipId, Arc<[f32]>>) {
    let mut p = Project::default();
    let mut clips = HashMap::new();
    let c = pool(&mut p, &mut clips, "a", 4 * RATE as usize);

    // Group and FX channel.
    let group = p.add_track(TrackKind::Group, "Group");
    let fx = p.add_track(TrackKind::Fx, "FX");
    let (group_id, fx_id) = (p.tracks[group].id, p.tracks[fx].id);

    // Audio with fades, EQ, a send, automation, inside the group.
    let a = p.add_track(TrackKind::Audio, "Audio");
    let ev = p.add_audio_event(a, 0, c).unwrap();
    {
        let e = p.event_mut(ev).unwrap();
        e.fade_in = Q;
        e.fade_out = Q;
    }
    let t = &mut p.tracks[a];
    t.parent = Some(group_id);
    t.eq.on = true;
    t.eq.bands[1].gain = 6.0;
    t.sends.push(SendSlot {
        target: fx_id,
        level_db: -6.0,
        pre_fader: false,
        on: true,
    });
    let mut vol = AutomationLane::new(AutoParam::Volume);
    vol.points = vec![
        AutoPoint {
            tick: 0,
            value: 0.2,
        },
        AutoPoint {
            tick: 8 * Q,
            value: 0.9,
        },
    ];
    let mut pan = AutomationLane::new(AutoParam::Pan);
    pan.points = vec![
        AutoPoint {
            tick: 0,
            value: 0.0,
        },
        AutoPoint {
            tick: 8 * Q,
            value: 1.0,
        },
    ];
    t.automation = vec![vol, pan];
    t.read = true;

    // Musical event (time-stretched while the song is built).
    let m = p.add_track(TrackKind::Audio, "Musical");
    let loop_clip = pool(&mut p, &mut clips, "loop", RATE as usize);
    let me = p.add_audio_event(m, 0, loop_clip).unwrap();
    p.set_musical(me, Some(100.0));

    // Audio part of two events.
    let ap = p.add_track(TrackKind::Audio, "Part");
    let x = p.add_audio_event(ap, 0, c).unwrap();
    let y = p.add_audio_event(ap, 4 * Q, c).unwrap();
    p.event_mut(x).unwrap().length = 2 * Q;
    p.event_mut(y).unwrap().length = 2 * Q;
    p.group_audio(&[x, y]).unwrap();

    // Video track with sound.
    let film = pool(&mut p, &mut clips, "film", 2 * RATE as usize);
    p.add_video_track("Film", "/film.webm", 4 * Q, Some(film));

    // MIDI notes (no instrument: they go to the MIDI output queue, if any).
    let midi = p.add_track(TrackKind::Midi, "MIDI");
    let part = p.create_event(midi, 0, 8 * Q).unwrap();
    p.event_mut(part).unwrap().notes = (0..32)
        .map(|i| studiocraft_project::Note {
            pitch: 48 + (i % 24) as u8,
            velocity: 90,
            start: i64::from(i) * 240,
            length: 200,
            spelling: 0,
        })
        .collect();

    // Tempo change.
    let mut tempo = p.tempo.clone();
    tempo.events = vec![TempoEvent {
        tick: 4 * Q,
        bpm: 140.0,
        ramp: true,
    }];
    tempo.use_track = true;
    p.set_tempo_map(tempo);
    (p, clips)
}

#[test]
fn the_counter_sees_allocations() {
    let n = count(|| {
        std::hint::black_box(vec![1u8; 16]);
    });
    assert!(n >= 2, "alloc and free counted: {n}");
}

#[test]
fn the_audio_callback_does_not_allocate() {
    let (p, clips) = busy_project();
    let (mut core, mut ctl) = engine(RATE, p.tempo.clone());
    let song = Arc::new(Song::build(&p, &clips, RATE));
    assert!(
        song.tracks.len() >= 5,
        "the project reaches the engine: {} tracks",
        song.tracks.len()
    );
    let mut out = vec![0.0f32; 512 * 2];

    // Warm up once: first-use work outside the measured region is allowed only if it does
    // not repeat, so every phase below is measured on its own.
    ctl.send(Command::SetSong(song.clone()));
    ctl.send(Command::SetMetronome(true));
    ctl.send(Command::SetLocators(0, 4 * u64::from(RATE)));
    ctl.send(Command::ToggleCycle);
    ctl.send(Command::Play);
    core.process(&mut out);
    ctl.collect_garbage();

    let mut report = Vec::new();
    let mut phase = |name: &str, n: usize, core: &mut studiocraft_engine::EngineCore| {
        let k = count(|| {
            for _ in 0..n {
                core.process(&mut out);
            }
        });
        report.push((name.to_owned(), k));
    };

    phase("playing, cycle, metronome", 2_000, &mut core);

    ctl.send(Command::Locate(2 * u64::from(RATE)));
    phase("locate", 50, &mut core);

    ctl.send(Command::Stop);
    phase("stop", 50, &mut core);

    ctl.send(Command::Play);
    phase("play again", 50, &mut core);

    ctl.send(Command::SetSong(Arc::new(Song::build(&p, &clips, RATE))));
    phase("swap song", 50, &mut core);
    ctl.collect_garbage();

    // A full command queue of song swaps in one block (e.g. after a stall): every replaced
    // song goes back to the main thread, none is freed here.
    for _ in 0..254 {
        assert!(ctl.send(Command::SetSong(Arc::new(Song::default()))));
    }
    assert!(ctl.send(Command::SetSong(Arc::new(Song::build(&p, &clips, RATE)))));
    phase("255 song swaps in one block", 1, &mut core);
    ctl.collect_garbage();

    ctl.send(Command::Preview(Some(tone(RATE as usize, 440.0))));
    phase("preview", 200, &mut core);
    ctl.collect_garbage();

    ctl.send(Command::Stop);
    ctl.send(Command::SetMetronome(false));
    phase("stopped", 200, &mut core);

    // Varying block sizes, as some drivers deliver.
    let k = count(|| {
        for n in [1usize, 7, 64, 333, 512] {
            core.process(&mut out[..n * 2]);
        }
    });
    report.push(("odd block sizes".to_owned(), k));

    assert_eq!(
        ctl.status.garbage_overflow(),
        0,
        "no return queue overflowed"
    );
    let bad: Vec<_> = report.iter().filter(|(_, k)| *k > 0).collect();
    assert!(
        bad.is_empty(),
        "allocations on the audio thread (phase, count): {bad:?}"
    );
}
