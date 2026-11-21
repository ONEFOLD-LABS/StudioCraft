//! Parameter edits a plugin reports itself (VST3 `performEdit`), from any thread.
//!
//! Plugins should report edits from their window on the main thread, but many also do it from
//! `process` on the audio thread. So recording an edit takes no lock and never allocates: a
//! fixed table keeps the latest value per parameter. A parameter claims a slot once (compare
//! and swap on its id) and keeps it; later edits only overwrite the value. When every slot is
//! taken by other parameters the edit is dropped and counted.

use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};

/// Different parameters the table can hold.
pub(crate) const SLOTS: usize = 1024;

#[derive(Default)]
struct Slot {
    /// Parameter id + 1; 0 = free.
    key: AtomicU64,
    /// `f64` bits of the latest value.
    value: AtomicU64,
    /// A value not yet taken.
    dirty: AtomicBool,
}

pub(crate) struct Edits {
    slots: Box<[Slot]>,
    dropped: AtomicU64,
}

impl Default for Edits {
    fn default() -> Self {
        Self::with_slots(SLOTS)
    }
}

impl Edits {
    fn with_slots(n: usize) -> Self {
        Self {
            slots: (0..n.max(1)).map(|_| Slot::default()).collect(),
            dropped: AtomicU64::new(0),
        }
    }

    /// Records the latest value of `id`. Any thread: no lock, no allocation, bounded work.
    pub(crate) fn record(&self, id: u32, value: f64) {
        let key = u64::from(id) + 1;
        let n = self.slots.len();
        // Spread neighbouring ids; probe linearly from there.
        let start = (u64::from(id).wrapping_mul(0x9e37_79b9_7f4a_7c15) >> 32) as usize % n;
        for k in 0..n {
            let Some(slot) = self.slots.get((start + k) % n) else {
                break;
            };
            let current = slot.key.load(Ordering::Acquire);
            let mine = current == key
                || (current == 0
                    && match slot
                        .key
                        .compare_exchange(0, key, Ordering::AcqRel, Ordering::Acquire)
                    {
                        Ok(_) => true,
                        Err(other) => other == key,
                    });
            if mine {
                slot.value.store(value.to_bits(), Ordering::Relaxed);
                slot.dirty.store(true, Ordering::Release);
                return;
            }
        }
        self.dropped.fetch_add(1, Ordering::Relaxed);
    }

    /// The edits since the last call, one (the latest) per parameter. Main thread.
    pub(crate) fn take(&self) -> Vec<(u32, f64)> {
        let mut out = Vec::new();
        for slot in self.slots.iter() {
            let key = slot.key.load(Ordering::Acquire);
            if key == 0 || !slot.dirty.swap(false, Ordering::Acquire) {
                continue;
            }
            let value = f64::from_bits(slot.value.load(Ordering::Relaxed));
            out.push(((key - 1) as u32, value));
        }
        out
    }

    /// Edits dropped so far because the table was full.
    pub(crate) fn dropped(&self) -> u64 {
        self.dropped.load(Ordering::Relaxed)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Arc;

    #[test]
    fn keeps_the_latest_value_per_parameter() {
        let e = Edits::default();
        for k in 0..1000 {
            e.record(7, f64::from(k));
        }
        e.record(3, 0.25);
        e.record(u32::MAX, 0.5);
        let mut got = e.take();
        got.sort_by_key(|g| g.0);
        assert_eq!(got, [(3, 0.25), (7, 999.0), (u32::MAX, 0.5)]);
        assert!(e.take().is_empty(), "taken once");
        e.record(7, 1.0);
        assert_eq!(e.take(), [(7, 1.0)], "the slot stays with its parameter");
        assert_eq!(e.dropped(), 0);
    }

    #[test]
    fn a_full_table_drops_and_counts() {
        let e = Edits::with_slots(4);
        for id in 0..6 {
            e.record(id, 1.0);
        }
        assert_eq!(e.take().len(), 4);
        assert_eq!(e.dropped(), 2);
        // Parameters that hold a slot still get through.
        e.record(0, 2.0);
        assert_eq!(e.dropped(), 2);
    }

    #[test]
    fn threads_record_at_once_and_the_last_value_wins() {
        // Each thread owns 200 parameters and writes rising values, so the last value written
        // per parameter is known; the main thread takes while they write.
        const PER: u32 = 200;
        const WRITES: u32 = 10_000;
        let e = Arc::new(Edits::default());
        let workers: Vec<_> = (0..4u32)
            .map(|t| {
                let e = Arc::clone(&e);
                std::thread::spawn(move || {
                    for k in 0..WRITES {
                        e.record(t * PER + k % PER, f64::from(k));
                    }
                })
            })
            .collect();
        let mut latest = std::collections::HashMap::new();
        let take = |latest: &mut std::collections::HashMap<u32, f64>| {
            for (id, v) in e.take() {
                latest.insert(id, v);
            }
        };
        while workers.iter().any(|w| !w.is_finished()) {
            take(&mut latest);
        }
        for w in workers {
            w.join().expect("worker");
        }
        take(&mut latest);
        assert_eq!(
            latest.len(),
            4 * PER as usize,
            "every parameter came through"
        );
        for (&id, &v) in &latest {
            let j = id % PER;
            let last = j + PER * ((WRITES - 1 - j) / PER);
            assert_eq!(v, f64::from(last), "parameter {id}: the last value written");
        }
        assert_eq!(e.dropped(), 0);
        // Each parameter holds exactly one slot.
        let claimed: Vec<u64> = e
            .slots
            .iter()
            .map(|s| s.key.load(Ordering::Relaxed))
            .filter(|&k| k != 0)
            .collect();
        let unique: std::collections::HashSet<_> = claimed.iter().collect();
        assert_eq!((unique.len(), claimed.len()), (800, 800));
    }
}
