//! Gemeinsame monotone Uhr für Ein- und Ausgabe (Nanosekunden seit Programmstart).
//! Damit lassen sich Abspiel- und Aufnahmezeitpunkte vergleichen, um die
//! Aufnahme latenzfrei an die richtige Projektposition zu legen.

use std::sync::{
    OnceLock,
    atomic::{AtomicU64, Ordering},
};

use web_time::Instant;

static EPOCH: OnceLock<Instant> = OnceLock::new();

/// Jetzt, in Nanosekunden seit dem ersten Aufruf.
pub fn now_nanos() -> u64 {
    EPOCH.get_or_init(Instant::now).elapsed().as_nanos() as u64
}

/// Zuordnung „Eingangs-Frame Nr. `frame` wurde zur Zeit `nanos` erfasst“,
/// geschrieben vom Eingangs-Callback, gelesen vom Recorder. Sequenz-Lock:
/// ungerade Sequenz = Schreiben läuft.
#[derive(Debug, Default)]
pub struct CaptureClock {
    seq: AtomicU64,
    frame: AtomicU64,
    nanos: AtomicU64,
}

impl CaptureClock {
    pub fn publish(&self, frame: u64, nanos: u64) {
        self.seq.fetch_add(1, Ordering::AcqRel);
        self.frame.store(frame, Ordering::Release);
        self.nanos.store(nanos, Ordering::Release);
        self.seq.fetch_add(1, Ordering::AcqRel);
    }

    /// Letzte vollständige Zuordnung, `None` solange noch nichts erfasst wurde.
    pub fn read(&self) -> Option<(u64, u64)> {
        for _ in 0..100 {
            let a = self.seq.load(Ordering::Acquire);
            if a % 2 == 1 {
                std::hint::spin_loop();
                continue;
            }
            let f = self.frame.load(Ordering::Acquire);
            let n = self.nanos.load(Ordering::Acquire);
            if self.seq.load(Ordering::Acquire) == a {
                return (a > 0).then_some((f, n));
            }
        }
        None
    }

    /// Erfassungszeit von Frame `frame` bei `rate`, abgeleitet aus der letzten Zuordnung.
    pub fn time_of(&self, frame: u64, rate: u32) -> Option<u64> {
        let (f, n) = self.read()?;
        let diff_ns = (f as i128 - frame as i128) * 1_000_000_000 / i128::from(rate.max(1));
        u64::try_from(n as i128 - diff_ns).ok()
    }
}

/// Projektposition des ersten aufgenommenen Frames.
///
/// `cursor` wurde zur Zeit `played_at` hörbar; der erste aufgenommene Frame
/// wurde zur Zeit `captured_at` erfasst. Gespielt wird zu dem, was man hört,
/// also gehört der Frame an `cursor + (captured_at − played_at) · rate`.
/// Rückgabe: (Startposition, Frames am Anfang der Aufnahme, die vor Projektbeginn liegen).
pub fn compensated_start(cursor: u64, played_at: u64, captured_at: u64, rate: u32) -> (u64, u64) {
    let delta = (captured_at as i128 - played_at as i128) * i128::from(rate) / 1_000_000_000;
    let pos = cursor as i128 + delta;
    if pos >= 0 {
        (pos as u64, 0)
    } else {
        (0, (-pos) as u64)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn capture_clock_extrapolates() {
        let c = CaptureClock::default();
        assert_eq!(c.read(), None);
        c.publish(48_000, 2_000_000_000); // Frame 48 000 bei 2 s
        assert_eq!(c.time_of(48_000, 48_000), Some(2_000_000_000));
        assert_eq!(c.time_of(24_000, 48_000), Some(1_500_000_000));
        assert_eq!(c.time_of(96_000, 48_000), Some(3_000_000_000));
    }

    #[test]
    fn compensation_moves_take_earlier_by_latency() {
        // Cursor 96 000 hörbar bei 1,010 s; erstes Sample erfasst bei 1,000 s
        // (Eingangslatenz + Startverzögerung): Take beginnt 10 ms = 480 Frames früher.
        assert_eq!(
            compensated_start(96_000, 1_010_000_000, 1_000_000_000, 48_000),
            (95_520, 0)
        );
        // Am Projektanfang: die ersten Frames fallen weg.
        assert_eq!(
            compensated_start(0, 1_010_000_000, 1_000_000_000, 48_000),
            (0, 480)
        );
        // Recorder startet etwas später als der Cursor hörbar wird.
        assert_eq!(
            compensated_start(0, 1_000_000_000, 1_005_000_000, 48_000),
            (240, 0)
        );
    }
}
