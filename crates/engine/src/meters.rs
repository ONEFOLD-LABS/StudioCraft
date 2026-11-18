//! Pegelanzeige: der Audio-Thread schreibt Spitzenwerte, die Oberfläche
//! holt sie ab. Lock-frei über atomare Werte (positive f32 sind als Bits
//! monoton, daher genügt `fetch_max`).

use std::sync::atomic::{AtomicU32, Ordering};

/// Höchstzahl Spuren mit eigener Pegelanzeige.
pub const MAX_METERED_TRACKS: usize = 1024;

#[derive(Debug)]
pub struct Meters {
    /// Slot 0/1: Ausgang L/R; danach je Spur L/R.
    slots: Box<[AtomicU32]>,
}

impl Default for Meters {
    fn default() -> Self {
        Self {
            slots: (0..2 + 2 * MAX_METERED_TRACKS)
                .map(|_| AtomicU32::new(0))
                .collect(),
        }
    }
}

impl Meters {
    fn feed(&self, slot: usize, peak: f32) {
        if let Some(s) = self.slots.get(slot) {
            s.fetch_max(peak.abs().to_bits(), Ordering::Relaxed);
        }
    }

    pub(crate) fn feed_master(&self, l: f32, r: f32) {
        self.feed(0, l);
        self.feed(1, r);
    }

    pub(crate) fn feed_track(&self, track: usize, l: f32, r: f32) {
        self.feed(2 + 2 * track, l);
        self.feed(3 + 2 * track, r);
    }

    fn take(&self, slot: usize) -> f32 {
        self.slots
            .get(slot)
            .map_or(0.0, |s| f32::from_bits(s.swap(0, Ordering::Relaxed)))
    }

    /// Spitzenwerte (linear) seit dem letzten Abholen.
    pub fn take_master(&self) -> [f32; 2] {
        [self.take(0), self.take(1)]
    }

    /// Spitzenwerte einer Projektspur (linear) seit dem letzten Abholen.
    pub fn take_track(&self, track: usize) -> [f32; 2] {
        [self.take(2 + 2 * track), self.take(3 + 2 * track)]
    }
}
