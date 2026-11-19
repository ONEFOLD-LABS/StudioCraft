//! Musikalische Zeit: Positionen in Ticks, Taktart und Tempo-Map.

/// Auflösung: Ticks pro Viertelnote.
pub const TICKS_PER_QUARTER: u64 = 960;

/// Tempo-unabhängige Position im Projekt, gemessen in Ticks ab Projektanfang.
#[derive(
    Debug,
    Clone,
    Copy,
    PartialEq,
    Eq,
    PartialOrd,
    Ord,
    Hash,
    Default,
    serde::Serialize,
    serde::Deserialize,
)]
pub struct Position(pub u64);

/// Taktart, z. B. 4/4 oder 6/8.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct TimeSignature {
    pub numerator: u32,
    pub denominator: u32,
}

impl TimeSignature {
    pub const COMMON: Self = Self {
        numerator: 4,
        denominator: 4,
    };

    /// Länge eines Schlags in Ticks.
    pub fn beat_ticks(self) -> u64 {
        TICKS_PER_QUARTER * 4 / u64::from(self.denominator)
    }

    /// Länge eines Takts in Ticks.
    pub fn bar_ticks(self) -> u64 {
        self.beat_ticks() * u64::from(self.numerator)
    }
}

/// Tempowechsel auf der Tempospur.
#[derive(Debug, Clone, Copy, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct TempoEvent {
    /// Position in Ticks (> 0; das Tempo am Anfang ist [`TempoMap::bpm`]).
    pub tick: u64,
    pub bpm: f64,
    /// Rampe: das Tempo gleitet vom vorigen Punkt linear hierher; sonst springt es hier.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub ramp: bool,
}

/// Tempo-Map: Tempo am Anfang, optional Tempowechsel (Tempospur) und die Taktart.
///
/// Positionen bleiben in Ticks (MIDI und Events „in Takten“); Frames und Sekunden ergeben sich
/// aus dem Integral über die Tempokurve. Ohne Tempospur (`use_track` aus) gilt `bpm` überall.
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct TempoMap {
    /// Tempo am Projektanfang bzw. festes Tempo.
    pub bpm: f64,
    pub signature: TimeSignature,
    /// Tempowechsel, nach `tick` sortiert, jeder Tick höchstens einmal.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub events: Vec<TempoEvent>,
    /// Tempospur aktiv (sonst festes Tempo `bpm`, die Punkte bleiben erhalten).
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub use_track: bool,
}

impl Default for TempoMap {
    fn default() -> Self {
        Self {
            bpm: 120.0,
            signature: TimeSignature::COMMON,
            events: Vec::new(),
            use_track: false,
        }
    }
}

/// Grenzen des Tempos in BPM (wie das Tempofeld).
pub const BPM_RANGE: (f64, f64) = (10.0, 999.0);

/// Ein Abschnitt der Tempokurve: ab `start` (Ticks, Sekunden), Tempo `b0` am Anfang, mit
/// Steigung `k` in BPM pro Tick bis `end` (`u64::MAX`: offen).
#[derive(Debug, Clone, Copy)]
struct Segment {
    start: u64,
    end: u64,
    secs: f64,
    b0: f64,
    k: f64,
}

/// Sekunden für `x` Ticks ab Segmentanfang.
fn seg_seconds(b0: f64, k: f64, x: f64) -> f64 {
    let q = TICKS_PER_QUARTER as f64;
    if k.abs() < 1e-12 {
        x * 60.0 / (q * b0)
    } else {
        60.0 / (q * k) * ((b0 + k * x) / b0).ln()
    }
}

/// Ticks nach `s` Sekunden ab Segmentanfang (Umkehrung von [`seg_seconds`]).
fn seg_ticks(b0: f64, k: f64, s: f64) -> f64 {
    let q = TICKS_PER_QUARTER as f64;
    if k.abs() < 1e-12 {
        s * q * b0 / 60.0
    } else {
        b0 * ((s * q * k / 60.0).exp() - 1.0) / k
    }
}

impl TempoMap {
    /// Festes Tempo mit 4/4.
    pub fn fixed(bpm: f64) -> Self {
        Self {
            bpm,
            ..Self::default()
        }
    }

    /// Gilt die Tempospur (mit mindestens einem Wechsel)?
    pub fn varies(&self) -> bool {
        self.use_track && !self.events.is_empty()
    }

    fn base_bpm(&self) -> f64 {
        if self.bpm.is_finite() && self.bpm > 0.0 {
            self.bpm
        } else {
            120.0
        }
    }

    /// Ruft `f` für jeden Abschnitt der Kurve auf, bis es `false` liefert. Keine Allokation:
    /// auch im Audio-Thread nutzbar.
    fn for_segments(&self, mut f: impl FnMut(&Segment) -> bool) {
        let base = self.base_bpm();
        if !self.varies() {
            f(&Segment {
                start: 0,
                end: u64::MAX,
                secs: 0.0,
                b0: base,
                k: 0.0,
            });
            return;
        }
        let mut seg_start = 0u64;
        let mut secs = 0.0;
        let mut b = base;
        for e in &self.events {
            let bpm = if e.bpm.is_finite() && e.bpm > 0.0 {
                e.bpm
            } else {
                b
            };
            if e.tick <= seg_start {
                // Doppelte oder unsortierte Punkte (fremde Datei): der Wechsel gilt sofort.
                b = bpm;
                continue;
            }
            let len = (e.tick - seg_start) as f64;
            let k = if e.ramp { (bpm - b) / len } else { 0.0 };
            let seg = Segment {
                start: seg_start,
                end: e.tick,
                secs,
                b0: b,
                k,
            };
            if !f(&seg) {
                return;
            }
            secs += seg_seconds(b, k, len);
            seg_start = e.tick;
            b = bpm;
        }
        f(&Segment {
            start: seg_start,
            end: u64::MAX,
            secs,
            b0: b,
            k: 0.0,
        });
    }

    /// Tempo in BPM an Position `tick`.
    pub fn bpm_at(&self, tick: u64) -> f64 {
        let mut out = self.base_bpm();
        self.for_segments(|s| {
            if tick < s.end {
                out = s.b0 + s.k * (tick - s.start) as f64;
                return false;
            }
            true
        });
        out
    }

    /// Position in Sekunden ab Projektanfang.
    pub fn seconds(&self, pos: Position) -> f64 {
        let mut out = 0.0;
        self.for_segments(|s| {
            if pos.0 < s.end {
                out = s.secs + seg_seconds(s.b0, s.k, (pos.0 - s.start) as f64);
                return false;
            }
            true
        });
        out
    }

    /// Position (in Sekunden ab Anfang) als Ticks, Umkehrung von [`Self::seconds`].
    pub fn ticks_at_seconds(&self, seconds: f64) -> f64 {
        let seconds = if seconds.is_finite() {
            seconds.max(0.0)
        } else {
            0.0
        };
        let mut out = 0.0;
        self.for_segments(|s| {
            let end_secs = if s.end == u64::MAX {
                f64::INFINITY
            } else {
                s.secs + seg_seconds(s.b0, s.k, (s.end - s.start) as f64)
            };
            if seconds < end_secs {
                out = s.start as f64 + seg_ticks(s.b0, s.k, seconds - s.secs);
                return false;
            }
            true
        });
        out
    }

    /// Position in Ticks als Frame-Position bei `rate` (ab Projektanfang).
    pub fn frames(&self, ticks: u64, rate: u32) -> u64 {
        let f = (self.seconds(Position(ticks)) * f64::from(rate)).round();
        if f.is_finite() && f >= 0.0 {
            f as u64
        } else {
            0
        }
    }

    /// Frame-Position bei `rate` als Position in Ticks.
    pub fn ticks(&self, frames: u64, rate: u32) -> u64 {
        if rate == 0 {
            return 0;
        }
        let t = self
            .ticks_at_seconds(frames as f64 / f64::from(rate))
            .round();
        if t.is_finite() && t >= 0.0 {
            t as u64
        } else {
            0
        }
    }

    /// Länge von `len` Ticks ab `start` in Frames (hängt bei Tempowechseln von der Lage ab).
    pub fn duration_frames(&self, start: u64, len: u64, rate: u32) -> u64 {
        self.frames(start.saturating_add(len), rate)
            .saturating_sub(self.frames(start, rate))
    }

    /// Setzt einen Tempowechsel bei `tick` (ersetzt einen am selben Tick); `tick` 0 setzt das
    /// Anfangstempo. Hält die Liste sortiert.
    pub fn set_event(&mut self, tick: u64, bpm: f64, ramp: bool) {
        let bpm = if bpm.is_finite() {
            bpm.clamp(BPM_RANGE.0, BPM_RANGE.1)
        } else {
            return;
        };
        if tick == 0 {
            self.bpm = bpm;
            return;
        }
        match self.events.binary_search_by_key(&tick, |e| e.tick) {
            Ok(i) => {
                self.events[i].bpm = bpm;
                self.events[i].ramp = ramp;
            }
            Err(i) => self.events.insert(i, TempoEvent { tick, bpm, ramp }),
        }
    }

    /// Entfernt den Tempowechsel bei `tick`.
    pub fn remove_event(&mut self, tick: u64) -> bool {
        let before = self.events.len();
        self.events.retain(|e| e.tick != tick);
        self.events.len() != before
    }

    /// Position aus Takt und Schlag, beide ab 1 gezählt wie im Lineal (1.1 = Anfang).
    pub fn position(&self, bar: u64, beat: u64) -> Position {
        let sig = self.signature;
        Position((bar - 1) * sig.bar_ticks() + (beat - 1) * sig.beat_ticks())
    }

    /// Takt.Schlag.Sechzehntel.Ticks für die Anzeige im Lineal.
    pub fn bars_beats(&self, pos: Position) -> (u64, u64, u64, u64) {
        let sig = self.signature;
        let sixteenth = TICKS_PER_QUARTER / 4;
        let bar = pos.0 / sig.bar_ticks();
        let rest = pos.0 % sig.bar_ticks();
        let beat = rest / sig.beat_ticks();
        let rest = rest % sig.beat_ticks();
        (bar + 1, beat + 1, rest / sixteenth + 1, rest % sixteenth)
    }

    /// Position als Text wie im Lineal: `Takt.Zählzeit.Sechzehntel.Ticks`, z. B. `3.2.1.000`.
    pub fn format_position(&self, pos: Position) -> String {
        let (bar, beat, six, tick) = self.bars_beats(pos);
        format!("{bar}.{beat}.{six}.{tick:03}")
    }

    /// Dauer als Text, jede Stelle ab 0 gezählt: ein Takt in 4/4 ist `1.0.0.000`.
    pub fn format_length(&self, ticks: u64) -> String {
        let (bar, beat, six, tick) = self.bars_beats(Position(ticks));
        format!("{}.{}.{}.{tick:03}", bar - 1, beat - 1, six - 1)
    }

    /// Liest eine Position im Format von [`Self::format_position`]. Fehlende Stellen hinten
    /// zählen als Anfang (`3` = `3.1.1.0`); Punkt, Komma, Doppelpunkt oder Leerzeichen trennen.
    /// Zählzeiten und Sechzehntel dürfen überlaufen (`1.5` in 4/4 = `2.1`).
    pub fn parse_position(&self, text: &str) -> Option<Position> {
        let [bar, beat, six, tick] = parse_parts(text, [1, 1, 1, 0])?;
        if bar == 0 || beat == 0 || six == 0 {
            return None;
        }
        self.parts_to_ticks([bar - 1, beat - 1, six - 1, tick])
            .map(Position)
    }

    /// Liest eine Dauer im Format von [`Self::format_length`] (fehlende Stellen hinten sind 0).
    pub fn parse_length(&self, text: &str) -> Option<u64> {
        self.parts_to_ticks(parse_parts(text, [0; 4])?)
    }

    fn parts_to_ticks(&self, [bars, beats, sixes, ticks]: [u64; 4]) -> Option<u64> {
        let sig = self.signature;
        bars.checked_mul(sig.bar_ticks())?
            .checked_add(beats.checked_mul(sig.beat_ticks())?)?
            .checked_add(sixes.checked_mul(TICKS_PER_QUARTER / 4)?)?
            .checked_add(ticks)
    }
}

/// Zerlegt `1.2.3.4` in bis zu vier Zahlen; fehlende Stellen hinten nimmt es aus `defaults`.
fn parse_parts(text: &str, defaults: [u64; 4]) -> Option<[u64; 4]> {
    let parts: Vec<&str> = text
        .split(['.', ',', ':', ' '])
        .filter(|p| !p.is_empty())
        .collect();
    if parts.is_empty() || parts.len() > 4 {
        return None;
    }
    let mut out = defaults;
    for (slot, p) in out.iter_mut().zip(&parts) {
        *slot = p.parse().ok()?;
    }
    Some(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn one_bar_at_120_bpm_is_two_seconds() {
        let map = TempoMap::default();
        assert_eq!(map.seconds(map.position(2, 1)), 2.0);
    }

    #[test]
    fn bars_beats_roundtrip() {
        let map = TempoMap::default();
        let pos = map.position(3, 2);
        assert_eq!(map.bars_beats(pos), (3, 2, 1, 0));
        assert_eq!(map.bars_beats(Position(0)), (1, 1, 1, 0));
    }

    #[test]
    fn positions_and_lengths_roundtrip_through_text() {
        let map = TempoMap::default();
        for ticks in [0, 1, 239, 240, 959, 960, 3840, 3841, 123_456] {
            let pos = Position(ticks);
            assert_eq!(map.parse_position(&map.format_position(pos)), Some(pos));
            assert_eq!(map.parse_length(&map.format_length(ticks)), Some(ticks));
        }
        assert_eq!(map.format_position(Position(0)), "1.1.1.000");
        assert_eq!(map.format_length(3840), "1.0.0.000");
    }

    #[test]
    fn short_and_odd_input_is_read_or_refused() {
        let map = TempoMap::default();
        assert_eq!(map.parse_position("3"), Some(map.position(3, 1)));
        assert_eq!(map.parse_position("3,2"), Some(map.position(3, 2)));
        assert_eq!(map.parse_position(" 1.5 "), Some(map.position(2, 1)));
        assert_eq!(map.parse_length("2"), Some(2 * 3840));
        for bad in [
            "",
            "0",
            "1.0",
            "a",
            "1.2.3.4.5",
            "-1",
            "99999999999999999999",
        ] {
            assert_eq!(map.parse_position(bad), None, "{bad:?}");
        }
        assert_eq!(map.parse_length("18446744073709551615"), None);
    }

    #[test]
    fn a_tempo_jump_changes_the_speed_from_its_tick_on() {
        let mut map = TempoMap::fixed(120.0);
        let bar = map.signature.bar_ticks();
        map.set_event(2 * bar, 60.0, false);
        // Without the track the fixed tempo still applies.
        assert_eq!(map.seconds(Position(4 * bar)), 8.0);
        map.use_track = true;
        // Two bars at 120 (4 s), then two bars at 60 (8 s).
        assert!((map.seconds(Position(2 * bar)) - 4.0).abs() < 1e-9);
        assert!((map.seconds(Position(4 * bar)) - 12.0).abs() < 1e-9);
        assert_eq!(map.bpm_at(2 * bar - 1), 120.0);
        assert_eq!(map.bpm_at(2 * bar), 60.0);
        assert_eq!(map.frames(4 * bar, 48_000), 12 * 48_000);
        assert_eq!(map.ticks(12 * 48_000, 48_000), 4 * bar);
        // A bar after the jump is twice as long in frames as one before it.
        assert_eq!(
            map.duration_frames(2 * bar, bar, 48_000),
            2 * map.duration_frames(0, bar, 48_000)
        );
    }

    #[test]
    fn a_ramp_matches_numeric_integration_and_inverts() {
        let mut map = TempoMap::fixed(60.0);
        map.use_track = true;
        map.set_event(4 * TICKS_PER_QUARTER, 180.0, true);
        // Numeric: sum of 60 / bpm(t) per quarter over small steps.
        let steps = 40_000;
        let len = 4 * TICKS_PER_QUARTER;
        let mut secs = 0.0;
        for i in 0..steps {
            let t = (i as f64 + 0.5) / steps as f64 * len as f64;
            let bpm = 60.0 + 120.0 * t / len as f64;
            secs += 60.0 / bpm / TICKS_PER_QUARTER as f64 * len as f64 / steps as f64;
        }
        assert!((map.seconds(Position(len)) - secs).abs() < 1e-6, "{secs}");
        assert!((map.bpm_at(len / 2) - 120.0).abs() < 1e-9);
        for ticks in [0, 1, 960, 1919, 3840, 7000, 100_000] {
            let f = map.frames(ticks, 48_000);
            let back = map.ticks(f, 48_000);
            assert!(back.abs_diff(ticks) <= 1, "{ticks} -> {f} -> {back}");
        }
    }

    #[test]
    fn events_stay_sorted_and_clamped() {
        let mut map = TempoMap::fixed(120.0);
        map.set_event(7680, 90.0, false);
        map.set_event(3840, 5000.0, true);
        map.set_event(7680, 100.0, false);
        map.set_event(0, 140.0, false);
        map.set_event(100, f64::NAN, false);
        let ticks: Vec<u64> = map.events.iter().map(|e| e.tick).collect();
        assert_eq!(ticks, [3840, 7680]);
        assert_eq!(map.events[0].bpm, BPM_RANGE.1);
        assert_eq!(map.events[1].bpm, 100.0);
        assert_eq!(map.bpm, 140.0);
        assert!(map.remove_event(3840));
        assert!(!map.remove_event(3840));
        // Unsorted or broken events (hostile file) never panic.
        map.events = vec![
            TempoEvent {
                tick: 9000,
                bpm: 0.0,
                ramp: true,
            },
            TempoEvent {
                tick: 10,
                bpm: f64::INFINITY,
                ramp: true,
            },
            TempoEvent {
                tick: 10,
                bpm: 60.0,
                ramp: false,
            },
        ];
        map.use_track = true;
        let _ = (
            map.seconds(Position(u64::MAX / 2)),
            map.ticks(u64::MAX / 2, 48_000),
        );
    }

    #[test]
    fn six_eight_bar_length() {
        let sig = TimeSignature {
            numerator: 6,
            denominator: 8,
        };
        assert_eq!(sig.bar_ticks(), 3 * TICKS_PER_QUARTER);
    }
}
