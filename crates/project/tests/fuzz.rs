//! Randomised robustness tests for everything that reads foreign files (AGENTS.md, Robustness):
//! project files and Standard MIDI Files, changed at random, must load or fail with an error,
//! never panic; what loads must survive editing and saving. No extra tools: a seeded generator
//! makes every run repeatable (see `fuzzkit/mod.rs` for the environment variables).

use studiocraft_project::{
    AutoParam, AutoPoint, AutomationLane, Note, PoolClip, Project, Quantize, TrackKind, file, smf,
};

#[path = "fuzzkit/json.rs"]
mod fuzzjson;
mod fuzzkit;

use fuzzjson::mutate_json;
use fuzzkit::{Case, Rng, cases, mutate_bytes};

/// A project that uses most of the file format.
fn rich_project() -> Project {
    let mut p = Project::demo(3);
    let c = p.add_clip(PoolClip {
        name: "take".into(),
        path: Some("Audio/take.wav".into()),
        sample_rate: 48_000,
        frames: 96_000,
    });
    let a = p.add_track(TrackKind::Audio, "Gesang");
    if let Some(id) = p.add_audio_event(a, 960, c) {
        p.set_fades(id, 480, 960);
    }
    let i = p.add_track(TrackKind::Instrument, "Synth");
    if let Some(part) = p.create_event(i, 0, 3840)
        && let Some(e) = p.event_mut(part)
    {
        e.notes = (0..8)
            .map(|k| Note {
                pitch: 60 + k,
                velocity: 100,
                start: i64::from(k) * 240,
                length: 240,
                spelling: 0,
            })
            .collect();
    }
    p.tracks[a].automation.push(AutomationLane {
        param: AutoParam::Volume,
        points: vec![
            AutoPoint {
                tick: 0,
                value: 0.0,
            },
            AutoPoint {
                tick: 960,
                value: -6.0,
            },
        ],
        muted: false,
        visible: true,
        writing: false,
    });
    p.add_track(TrackKind::Folder, "Ordner");
    p.add_track(TrackKind::Fx, "Hall");
    p.add_marker(7680);
    p.add_cycle_marker(0, 3840);
    p
}

/// What the program does with a loaded project; none of it may panic.
fn exercise(mut p: Project) {
    let mut again = p.clone();
    assert_eq!(again.sanitize(), 0, "sanitize is stable after loading");
    let _ = p.length();
    let _ = p.tempo.signature.bar_ticks();
    let _ = p.tempo.frames(p.length(), 48_000);
    let _ = p.tempo.frames(u64::MAX, 192_000);
    for t in &p.tracks {
        let _ = t.lanes();
        let _ = t.audible_parts();
        for e in &t.events {
            let _ = e.end();
            let _ = e.contains(e.start.0);
            for n in &e.notes {
                let _ = n.end();
            }
        }
        for l in &t.automation {
            let _ = l.value_at(0);
            let _ = l.value_at(u64::MAX);
        }
    }
    let ids: Vec<_> = p
        .tracks
        .iter()
        .flat_map(|t| &t.events)
        .map(|e| e.id)
        .collect();
    for id in ids {
        p.quantize_event(id, &Quantize::default());
    }
    if !p.tracks.is_empty() {
        let _ = p.create_event(0, 0, 960);
    }
    let _ = p.add_track(TrackKind::Audio, "neu");
    p.add_marker(0);
    let _ = p.export_midi();
    // Saving and loading again gives the same project.
    let text = file::to_json(&p);
    let back = file::from_json(&text).map(|b| file::to_json(&b));
    assert_eq!(back.as_deref().ok(), Some(text.as_str()), "round trip");
}

#[test]
fn project_files_survive_structural_changes() {
    let seed_doc: serde_json::Value =
        serde_json::from_str(&file::to_json(&rich_project())).unwrap_or_default();
    let mut loaded = 0;
    let seeds = cases(0x57, 400);
    let n = seeds.len();
    for seed in seeds {
        let _case = Case("project-structure", seed);
        let mut rng = Rng(seed | 1);
        let mut doc = seed_doc.clone();
        // Inside the project: changing the header only tests the first check.
        if let Some(project) = doc.get_mut("project") {
            for _ in 0..1 + rng.below(3) {
                mutate_json(&mut rng, project);
            }
        }
        let text = serde_json::to_string(&doc).unwrap_or_default();
        if let Ok(p) = file::from_json(&text) {
            loaded += 1;
            exercise(p);
        }
    }
    // Most changes break the file's types (an error, as wanted); enough must still load for
    // the test to reach the code behind the parser.
    assert!(loaded * 10 > n, "only {loaded} of {n} variants loaded");
}

#[test]
fn project_files_survive_byte_changes() {
    let seed_text = file::to_json(&rich_project()).into_bytes();
    for seed in cases(0xb17e, 400) {
        let _case = Case("project-bytes", seed);
        let mut rng = Rng(seed | 1);
        let mut bytes = seed_text.clone();
        for _ in 0..1 + rng.below(3) {
            mutate_bytes(&mut rng, &mut bytes);
        }
        if let Ok(p) = file::from_json(&String::from_utf8_lossy(&bytes)) {
            exercise(p);
        }
    }
}

#[test]
fn midi_files_survive_byte_changes() {
    let seed_file = rich_project().export_midi().unwrap_or_default();
    assert!(seed_file.len() > 40, "the seed is a real MIDI file");
    let mut read = 0;
    let seeds = cases(0x5f, 1500);
    let n = seeds.len();
    for seed in seeds {
        let _case = Case("midi-file", seed);
        let mut rng = Rng(seed | 1);
        let mut bytes = seed_file.clone();
        for _ in 0..1 + rng.below(3) {
            mutate_bytes(&mut rng, &mut bytes);
        }
        if let Ok(midi) = smf::read(&bytes) {
            read += 1;
            let mut p = Project::default();
            let _ = p.import_midi(&midi, 0, true);
            let mut q = rich_project();
            let _ = q.import_midi(&midi, 1920, false);
            exercise(p);
            exercise(q);
        }
    }
    assert!(read * 10 > n, "only {read} of {n} variants were read");
}

#[test]
fn typed_positions_and_lengths_never_panic() {
    let tempo = rich_project().tempo;
    const PARTS: [&str; 14] = [
        "1",
        "0",
        "-1",
        ".",
        ":",
        " ",
        "999999999999999999999",
        "4294967296",
        "1.1.1.0",
        "x",
        "١",
        "\u{0}",
        "+",
        "1e9",
    ];
    for seed in cases(0x71, 3000) {
        let _case = Case("positions", seed);
        let mut rng = Rng(seed | 1);
        let text: String = (0..rng.below(8))
            .map(|_| PARTS[rng.below(PARTS.len())])
            .collect();
        let _ = tempo.parse_position(&text);
        let _ = tempo.parse_length(&text);
    }
}

#[test]
fn drum_map_files_survive_changes() {
    let mut map = studiocraft_project::DrumMap::general_midi();
    if let Some(r) = map.rows.first_mut() {
        r.mute = true;
    }
    let seed_doc: serde_json::Value = map
        .to_file_text()
        .ok()
        .and_then(|t| serde_json::from_str(&t).ok())
        .unwrap_or_default();
    let mut loaded = 0;
    let all = cases(0xd7, 600);
    let n = all.len();
    for seed in all {
        let _case = Case("drum-map", seed);
        let mut rng = Rng(seed | 1);
        let mut doc = seed_doc.clone();
        for _ in 0..1 + rng.below(3) {
            mutate_json(&mut rng, &mut doc);
        }
        let mut text = serde_json::to_string(&doc).unwrap_or_default().into_bytes();
        if rng.below(4) == 0 {
            mutate_bytes(&mut rng, &mut text);
        }
        let Ok(mut m) =
            studiocraft_project::DrumMap::from_file_text(&String::from_utf8_lossy(&text))
        else {
            continue;
        };
        loaded += 1;
        assert!(!m.sanitize(), "sanitize is stable after loading");
        for pitch in [0, 36, 127, 128, 255] {
            let _ = m.row(pitch);
            let _ = m.name_of(pitch);
            let _ = m.audible(pitch);
            assert!(m.output_of(pitch) < 128 || pitch >= 128);
        }
        let _ = m.remaps();
        let _ = m.filters();
        if let Some(i) = m.add_row(60) {
            let _ = m.move_row(i, true);
            let _ = m.set_pitch(i, 61);
        }
        let again = m
            .to_file_text()
            .ok()
            .and_then(|t| studiocraft_project::DrumMap::from_file_text(&t).ok());
        assert!(again.is_some(), "a loaded map saves and loads again");
    }
    assert!(loaded * 10 > n, "only {loaded} of {n} variants loaded");
}
