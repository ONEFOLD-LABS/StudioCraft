//! Shared helpers for the fuzz tests of every crate (project, engine, ui): a seeded random
//! generator, the cases of one test and byte mutations. Included with `#[path]` where a crate
//! needs it; `json.rs` beside it adds JSON mutations.
//!
//! `STUDIOCRAFT_FUZZ_ITERS=<n>` runs longer, `STUDIOCRAFT_FUZZ_SEED=<n>` starts elsewhere,
//! `STUDIOCRAFT_FUZZ_CASE=<n>` runs only the case a failure printed.

#![allow(dead_code)]

/// xorshift64*: small, fast, good enough to pick mutations.
pub struct Rng(pub u64);

impl Rng {
    pub fn next(&mut self) -> u64 {
        let mut x = self.0;
        x ^= x >> 12;
        x ^= x << 25;
        x ^= x >> 27;
        self.0 = x;
        x.wrapping_mul(0x2545_f491_4f6c_dd1d)
    }

    pub fn below(&mut self, n: usize) -> usize {
        if n == 0 {
            0
        } else {
            (self.next() % n as u64) as usize
        }
    }

    pub fn chance(&mut self, one_in: u64) -> bool {
        self.next() % one_in == 0
    }
}

pub fn env_number(name: &str) -> Option<u64> {
    std::env::var(name).ok().and_then(|v| v.trim().parse().ok())
}

/// The seeds of one test's cases: `default` many (or `STUDIOCRAFT_FUZZ_ITERS`), derived from
/// the base seed and `salt`; only `STUDIOCRAFT_FUZZ_CASE` when that is set.
pub fn cases(salt: u64, default: usize) -> Vec<u64> {
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
pub struct Case(pub &'static str, pub u64);

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

const INTERESTING: [u8; 12] = [
    0x00, 0x01, 0x7f, 0x80, 0xff, 0xfe, 0xc0, 0x40, b'"', b'-', b'9', b'{',
];

/// One random change of a byte string: flip, overwrite, insert, delete, duplicate, cut.
pub fn mutate_bytes(rng: &mut Rng, b: &mut Vec<u8>) {
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
