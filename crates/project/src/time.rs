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

/// Tempo-Map. Im ersten Schritt nur festes Tempo und feste Taktart;
/// die variable Tempospur kommt später.
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
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

    /// Dauer in Ticks als Anzahl Frames bei `rate`.
    pub fn frames(&self, ticks: u64, rate: u32) -> u64 {
        (ticks as f64 / TICKS_PER_QUARTER as f64 * 60.0 / self.bpm * f64::from(rate)).round() as u64
    }

    /// Anzahl Frames bei `rate` als Dauer in Ticks.
    pub fn ticks(&self, frames: u64, rate: u32) -> u64 {
        (frames as f64 / f64::from(rate) * self.bpm / 60.0 * TICKS_PER_QUARTER as f64).round()
            as u64
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
    fn six_eight_bar_length() {
        let sig = TimeSignature {
            numerator: 6,
            denominator: 8,
        };
        assert_eq!(sig.bar_ticks(), 3 * TICKS_PER_QUARTER);
    }
}
