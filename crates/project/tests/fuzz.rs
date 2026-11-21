//! Randomised robustness tests for everything that reads foreign files (AGENTS.md, Robustness):
//! project files and Standard MIDI Files, changed at random, must load or fail with an error,
//! never panic; what loads must survive editing and saving. No extra tools: a seeded generator
//! makes every run repeatable. `STUDIOCRAFT_FUZZ_ITERS=<n>` runs longer (e.g. nightly),
//! `STUDIOCRAFT_FUZZ_SEED=<n>` starts elsewhere, and `STUDIOCRAFT_FUZZ_CASE=<n>` runs only the
//! case a failure printed.

use studiocraft_project::{
    AutoParam, AutoPoint, AutomationLane, Note, PoolClip, Project, Quantize, TrackKind, file, smf,
};

/// xorshift64*: small, fast, good enough to pick mutations.
struct Rng(u64);

impl Rng {
    fn next(&mut self) -> u64 {
        let mut x = self.0;
        x ^= x >> 12;
        x ^= x << 25;
        x ^= x >> 27;
        self.0 = x;
        x.wrapping_mul(0x2545_f491_4f6c_dd1d)
    }

    fn below(&mut self, n: usize) -> usize {
        if n == 0 {
            0
        } else {
            (self.next() % n as u64) as usize
        }
    }

    fn chance(&mut self, one_in: u64) -> bool {
        self.next() % one_in == 0
    }
}

fn env_number(name: &str) -> Option<u64> {
    std::env::var(name).ok().and_then(|v| v.trim().parse().ok())
}

/// The seeds of one test's cases: `default` many (or `STUDIOCRAFT_FUZZ_ITERS`), derived from
/// the base seed and `salt`; only `STUDIOCRAFT_FUZZ_CASE` when that is set.
fn cases(salt: u64, default: usize) -> Vec<u64> {
    if let Some(case) = env_number("STUDIOCRAFT_FUZZ_CASE") {
        return vec![case];
    }
    let base = env_number("STUDIOCRAFT_FUZZ_SEED").unwrap_or(0x5eed_0f57_d10c_4a00) ^ salt;
    let n = env_number("STUDIOCRAFT_FUZZ_ITERS").map_or(default, |n| n as usize);
    (0..n as u64)
        .map(|k| base ^ k.wrapping_mul(0x9e37_79b9_7f4a_7c15))
        .collect()
}

/// Prints the case that was running when a panic unwinds through it.
struct Case(&'static str, u64);

impl Drop for Case {
    fn drop(&mut self) {
        if std::thread::panicking() {
            eprintln!(
                "fuzz case {} failed: rerun with STUDIOCRAFT_FUZZ_CASE={}",
                self.0, self.1
            );
        }
    }
}

const INTERESTING: [u8; 10] = [0x00, 0x01, 0x7f, 0x80, 0xff, 0xfe, b'"', b'-', b'9', b'{'];

/// One random change of a byte string: flip, overwrite, insert, delete, duplicate, cut.
fn mutate_bytes(rng: &mut Rng, b: &mut Vec<u8>) {
    let len = b.len();
    match rng.below(6) {
        0 if len > 0 => {
            let i = rng.below(len);
            b[i] ^= 1 << rng.below(8);
        }
        1 if len > 0 => {
            let i = rng.below(len);
            b[i] = INTERESTING[rng.below(INTERESTING.len())];
        }
        2 => {
            let i = rng.below(len + 1);
            let n = 1 + rng.below(8);
            for _ in 0..n {
                b.insert(i, rng.next() as u8);
            }
        }
        3 if len > 0 => {
            let i = rng.below(len);
            let n = (1 + rng.below(16)).min(len - i);
            b.drain(i..i + n);
        }
        4 if len > 0 => {
            let i = rng.below(len);
            let n = (1 + rng.below(32)).min(len - i);
            let chunk: Vec<u8> = b[i..i + n].to_vec();
            let at = rng.below(len + 1);
            b.splice(at..at, chunk);
        }
        _ => {
            b.truncate(rng.below(len + 1));
        }
    }
}

/// A replacement for `old`: mostly a value of the same kind that no writer would produce
/// (extreme numbers, odd strings), sometimes a value of another type.
fn odd_value(rng: &mut Rng, old: &serde_json::Value) -> serde_json::Value {
    use serde_json::{Value, json};
    if !rng.chance(4) {
        match old {
            Value::Number(n) => {
                let x = n.as_i64().unwrap_or(0);
                let unsigned = x >= 0;
                return match rng.below(12) {
                    0 | 1 => json!(x.saturating_add(1)),
                    2 | 3 if unsigned => json!(x.saturating_sub(1).max(0)),
                    4 if unsigned => json!(x.saturating_mul(1000)),
                    5 => json!(0),
                    6 if unsigned => json!(u32::MAX),
                    7 if unsigned => json!(u64::MAX),
                    8 => json!(i64::MIN),
                    9 => json!(1e308),
                    10 => json!(-1),
                    _ => json!(x / 2),
                };
            }
            Value::String(_) => {
                return match rng.below(4) {
                    0 => json!(""),
                    1 => json!("\u{0}\u{202e}x"),
                    2 => json!("../../etc/passwd"),
                    _ => json!("x".repeat(10_000)),
                };
            }
            Value::Bool(b) => return Value::Bool(!b),
            _ => {}
        }
    }
    match rng.below(6) {
        0 => Value::Null,
        1 => Value::Bool(rng.chance(2)),
        2 => json!(u64::MAX),
        3 => json!(""),
        4 => json!([]),
        _ => json!({}),
    }
}

/// One random change of the JSON tree: replace, remove, duplicate, swap, nudge a number.
fn mutate_json(rng: &mut Rng, v: &mut serde_json::Value) {
    use serde_json::Value;
    // Walk down a random path; stop at a leaf or by chance.
    let mut node = v;
    loop {
        let go_on = !rng.chance(4);
        let next = match node {
            Value::Array(a) if !a.is_empty() && go_on => {
                let i = rng.below(a.len());
                Some(i)
            }
            Value::Object(o) if !o.is_empty() && go_on => Some(rng.below(o.len())),
            _ => None,
        };
        let Some(i) = next else { break };
        // Change this container sometimes instead of going deeper.
        if rng.chance(3) {
            match node {
                Value::Array(a) => match rng.below(3) {
                    0 => {
                        a.remove(i);
                    }
                    1 => {
                        let copy = a[i].clone();
                        a.insert(i, copy);
                    }
                    _ => {
                        let j = rng.below(a.len());
                        a.swap(i, j);
                    }
                },
                Value::Object(o) => {
                    if let Some(key) = o.keys().nth(i).cloned() {
                        if rng.chance(2) {
                            o.remove(&key);
                        } else {
                            let old = o.get(&key).cloned().unwrap_or_default();
                            o.insert(key, odd_value(rng, &old));
                        }
                    }
                }
                _ => {}
            }
            return;
        }
        node = match node {
            Value::Array(a) => &mut a[i],
            Value::Object(o) => match o.values_mut().nth(i) {
                Some(x) => x,
                None => return,
            },
            _ => return,
        };
    }
    let old = node.clone();
    *node = odd_value(rng, &old);
}

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
