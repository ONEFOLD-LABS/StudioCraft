//! Randomised robustness tests for audio files and MIDI input (AGENTS.md, Robustness): files
//! of every supported format, changed at random, must decode or fail with an error, never
//! panic; what decodes holds only finite samples at a supported rate. Seeded and repeatable,
//! like crates/project/tests/fuzz.rs: `STUDIOCRAFT_FUZZ_ITERS=<n>` runs longer,
//! `STUDIOCRAFT_FUZZ_SEED=<n>` starts elsewhere, `STUDIOCRAFT_FUZZ_CASE=<n>` repeats one case.

use studiocraft_engine::clip::AudioClip;
use studiocraft_engine::export::{encode_audio, wav_f32_bytes};
use studiocraft_engine::midi;

#[path = "../../project/tests/fuzzkit/mod.rs"]
mod fuzzkit;

use fuzzkit::{Case, Rng, cases, mutate_bytes};

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
