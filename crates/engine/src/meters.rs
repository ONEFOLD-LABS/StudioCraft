//! Pegelanzeige: der Audio-Thread schreibt Spitzenwerte, die Oberfläche
//! holt sie ab. Lock-frei über atomare Werte (positive f32 sind als Bits
//! monoton, daher genügt `fetch_max`).

use std::sync::atomic::{AtomicU32, Ordering};

/// Höchstzahl Spuren mit eigener Pegelanzeige.
pub const MAX_METERED_TRACKS: usize = 1024;

/// Erster Platz der Ausgangsbusse (hinter den Spuren), je Bus L/R.
const OUTPUT_BASE: usize = 2 + 2 * MAX_METERED_TRACKS;

#[derive(Debug)]
pub struct Meters {
    /// Slot 0/1: Ausgang L/R; danach je Spur L/R.
    slots: Box<[AtomicU32]>,
}

impl Default for Meters {
    fn default() -> Self {
        Self {
            slots: (0..OUTPUT_BASE + 2 * studiocraft_project::MAX_IO_BUSES)
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

    /// Ausgangsbus `k` (ab 1; 0 ist der Hauptmix, siehe [`Self::feed_master`]).
    pub(crate) fn feed_output(&self, k: usize, l: f32, r: f32) {
        if k > 0 && k < studiocraft_project::MAX_IO_BUSES {
            self.feed(OUTPUT_BASE + 2 * k, l);
            self.feed(OUTPUT_BASE + 2 * k + 1, r);
        }
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

    /// Spitzenwerte des Ausgangsbusses `k` (ab 1, linear) seit dem letzten Abholen.
    pub fn take_output(&self, k: usize) -> [f32; 2] {
        if k == 0 || k >= studiocraft_project::MAX_IO_BUSES {
            return [0.0; 2];
        }
        [
            self.take(OUTPUT_BASE + 2 * k),
            self.take(OUTPUT_BASE + 2 * k + 1),
        ]
    }

    /// Spitzenwerte einer Projektspur (linear) seit dem letzten Abholen.
    pub fn take_track(&self, track: usize) -> [f32; 2] {
        [self.take(2 + 2 * track), self.take(3 + 2 * track)]
    }
}

/// Höchstzahl Gerätekanäle am Eingang mit eigenem Pegel und eigener Anzeige.
pub const MAX_INPUT_CHANNELS: usize = 64;

/// Eingangskanäle des Geräts (Kanalzüge der Eingangsbusse): Pegel, den der Eingangs-Callback
/// vor Aufnahme und Mithören anwendet, und Spitzenwerte für die Anzeige. Lock-frei.
#[derive(Debug)]
pub struct InputLevels {
    gains: Box<[AtomicU32]>,
    peaks: Box<[AtomicU32]>,
}

impl Default for InputLevels {
    fn default() -> Self {
        Self {
            gains: (0..MAX_INPUT_CHANNELS)
                .map(|_| AtomicU32::new(1.0f32.to_bits()))
                .collect(),
            peaks: (0..MAX_INPUT_CHANNELS).map(|_| AtomicU32::new(0)).collect(),
        }
    }
}

impl InputLevels {
    /// Pegel (linear) des Gerätekanals `ch` (ab 0); ungültige Werte gelten als 1.
    pub fn set_gain(&self, ch: usize, gain: f32) {
        let gain = if gain.is_finite() {
            gain.clamp(0.0, 4.0)
        } else {
            1.0
        };
        if let Some(g) = self.gains.get(ch) {
            g.store(gain.to_bits(), Ordering::Relaxed);
        }
    }

    pub fn gain(&self, ch: usize) -> f32 {
        self.gains
            .get(ch)
            .map_or(1.0, |g| f32::from_bits(g.load(Ordering::Relaxed)))
    }

    pub(crate) fn feed(&self, ch: usize, peak: f32) {
        if let Some(s) = self.peaks.get(ch) {
            s.fetch_max(peak.abs().to_bits(), Ordering::Relaxed);
        }
    }

    /// Spitzenwert (linear, nach dem Pegel) des Gerätekanals `ch` seit dem letzten Abholen.
    pub fn take(&self, ch: usize) -> f32 {
        self.peaks
            .get(ch)
            .map_or(0.0, |s| f32::from_bits(s.swap(0, Ordering::Relaxed)))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn input_levels_hold_gains_and_peaks_per_channel() {
        let l = InputLevels::default();
        assert_eq!(l.gain(3), 1.0);
        l.set_gain(3, 0.5);
        l.set_gain(4, f32::NAN);
        l.set_gain(MAX_INPUT_CHANNELS, 0.1);
        assert_eq!((l.gain(3), l.gain(4)), (0.5, 1.0));
        assert_eq!(l.gain(MAX_INPUT_CHANNELS), 1.0, "out of range is ignored");
        l.feed(2, 0.3);
        l.feed(2, -0.6);
        assert_eq!(l.take(2), 0.6);
        assert_eq!(l.take(2), 0.0, "taken is taken");
    }

    #[test]
    fn output_meters_are_separate_from_tracks() {
        let m = Meters::default();
        m.feed_output(1, 0.2, 0.4);
        m.feed_output(0, 1.0, 1.0);
        m.feed_output(studiocraft_project::MAX_IO_BUSES, 1.0, 1.0);
        assert_eq!(m.take_output(1), [0.2, 0.4]);
        assert_eq!(m.take_master(), [0.0, 0.0]);
        assert_eq!(m.take_track(MAX_METERED_TRACKS - 1), [0.0, 0.0]);
    }
}
