//! Randomised robustness tests for audio files and MIDI input (AGENTS.md, Robustness): files
//! of every supported format, changed at random, must decode or fail with an error, never
//! panic; what decodes holds only finite samples at a supported rate. Seeded and repeatable,
//! like crates/project/tests/fuzz.rs: `STUDIOCRAFT_FUZZ_ITERS=<n>` runs longer,
//! `STUDIOCRAFT_FUZZ_SEED=<n>` starts elsewhere, `STUDIOCRAFT_FUZZ_CASE=<n>` repeats one case.

use studiocraft_engine::clip::AudioClip;
use studiocraft_engine::export::{encode_audio, wav_f32_bytes};
use studiocraft_engine::midi;

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

const INTERESTING: [u8; 8] = [0x00, 0x01, 0x7f, 0x80, 0xff, 0xfe, 0xc0, 0x40];

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

/// A short stereo signal: a tone with a click.
fn signal() -> Vec<f32> {
    (0..2400)
        .flat_map(|i| {
            let s = ((i as f32) * 0.05).sin() * 0.5;
            let s = if i == 1200 { 1.0 } else { s };
            [s, -s]
        })
        .collect()
}

fn check(clip: &AudioClip) {
    assert!(
        clip.samples.iter().all(|s| s.is_finite()),
        "decoded samples are finite"
    );
    assert!(
        (1_000..=768_000).contains(&clip.sample_rate),
        "{}",
        clip.sample_rate
    );
}

#[test]
fn audio_files_survive_byte_changes() {
    let s = signal();
    let mut seeds: Vec<(Vec<u8>, &str)> = Vec::new();
    for ext in ["wav", "flac", "aif"] {
        if let Ok((bytes, _)) = encode_audio(&s, 48_000, ext) {
            seeds.push((bytes, ext));
        }
    }
    if let Ok(bytes) = wav_f32_bytes(&s, 44_100) {
        seeds.push((bytes, "wav"));
    }
    assert_eq!(seeds.len(), 4, "every seed format encodes");
    for (bytes, ext) in &seeds {
        match AudioClip::from_bytes(bytes, Some(ext)) {
            Ok(clip) => check(&clip),
            Err(e) => panic!("{ext}: {e}"),
        }
    }
    let mut decoded = 0;
    let all = cases(0xa0d1, 600);
    let n = all.len();
    for seed in all {
        let _case = Case("audio-file", seed);
        let mut rng = Rng(seed | 1);
        let (seed_bytes, ext) = &seeds[rng.below(seeds.len())];
        let mut bytes = seed_bytes.clone();
        for _ in 0..1 + rng.below(3) {
            mutate_bytes(&mut rng, &mut bytes);
        }
        // The extension is only a hint: also none or a wrong one.
        let hint = match rng.below(4) {
            0 => None,
            1 => Some("mp3"),
            _ => Some(*ext),
        };
        if let Ok(clip) = AudioClip::from_bytes(&bytes, hint) {
            decoded += 1;
            check(&clip);
        }
        if let Ok(clip) = AudioClip::from_bytes_channel(&bytes, hint, Some(rng.below(4))) {
            check(&clip);
        }
    }
    assert!(decoded * 10 > n, "only {decoded} of {n} variants decoded");
}

/// Float WAV files may hold NaN and infinity; they must not reach the mix.
#[test]
fn non_finite_samples_in_float_files_become_silence() {
    let s = [0.5, f32::NAN, f32::INFINITY, f32::NEG_INFINITY, -0.25, 1e30];
    let bytes = wav_f32_bytes(&s, 48_000).unwrap_or_default();
    let clip = match AudioClip::from_bytes(&bytes, Some("wav")) {
        Ok(c) => c,
        Err(e) => panic!("{e}"),
    };
    check(&clip);
    assert_eq!(clip.samples[0], 0.5);
    assert_eq!(clip.samples[1], 0.0, "NaN becomes silence");
    assert_eq!(clip.samples[4], -0.25);
}

#[test]
fn raw_pcm_with_any_layout_never_panics() {
    for seed in cases(0x9c3, 2000) {
        let _case = Case("pcm", seed);
        let mut rng = Rng(seed | 1);
        let data: Vec<u8> = (0..rng.below(256)).map(|_| rng.next() as u8).collect();
        let channels = [0, 1, 2, 3, 8, 255, u16::MAX][rng.below(7)];
        let bits = [0, 8, 12, 16, 24, 32, 64][rng.below(7)];
        let rate = [0, 1, 44_100, 48_000, 96_000, u32::MAX][rng.below(6)];
        let channel = [None, Some(0), Some(1), Some(7), Some(usize::MAX)][rng.below(5)];
        if let Ok(clip) = AudioClip::from_pcm(&data, channels, bits, rate, channel) {
            check(&clip);
        }
    }
}

#[test]
fn midi_messages_never_panic() {
    for seed in cases(0x3d1, 5000) {
        let _case = Case("midi-message", seed);
        let mut rng = Rng(seed | 1);
        let msg: Vec<u8> = (0..rng.below(5)).map(|_| rng.next() as u8).collect();
        if let Some(midi::MidiMsg::Note { key, velocity, .. }) = midi::parse(&msg) {
            assert!(key < 128 && velocity < 128);
        }
    }
}
