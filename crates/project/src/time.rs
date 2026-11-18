//! Musikalische Zeit: Positionen in Ticks, Taktart und Tempo-Map.

/// Auflösung: Ticks pro Viertelnote.
pub const TICKS_PER_QUARTER: u64 = 960;

/// Tempo-unabhängige Position im Projekt, gemessen in Ticks ab Projektanfang.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Default)]
pub struct Position(pub u64);

/// Taktart, z. B. 4/4 oder 6/8.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
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

/// Tempo-Map. Im ersten Schritt nur festes Tempo und feste Taktart;
/// die variable Tempospur kommt später.
#[derive(Debug, Clone, PartialEq)]
pub struct TempoMap {
    pub bpm: f64,
    pub signature: TimeSignature,
}

impl Default for TempoMap {
    fn default() -> Self {
        Self {
            bpm: 120.0,
            signature: TimeSignature::COMMON,
        }
    }
}

impl TempoMap {
    /// Position in Sekunden ab Projektanfang.
    pub fn seconds(&self, pos: Position) -> f64 {
        pos.0 as f64 / TICKS_PER_QUARTER as f64 * 60.0 / self.bpm
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
    fn six_eight_bar_length() {
        let sig = TimeSignature {
            numerator: 6,
            denominator: 8,
        };
        assert_eq!(sig.bar_ticks(), 3 * TICKS_PER_QUARTER);
    }
}
