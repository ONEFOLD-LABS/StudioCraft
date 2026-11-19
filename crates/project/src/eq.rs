//! Kanal-EQ (Spec „Mixer“ → Signalfluss, Punkt 4; Issue #20): vier Bänder pro Kanalzug,
//! einzeln und als Ganzes abschaltbar. Die Filter selbst rechnet die Engine.

/// Filterform eines Bands.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub enum BandKind {
    LowShelf,
    Peak,
    HighShelf,
    /// Hochpass (schneidet Tiefen), Gain ohne Wirkung.
    LowCut,
    /// Tiefpass (schneidet Höhen), Gain ohne Wirkung.
    HighCut,
}

impl BandKind {
    pub const ALL: [BandKind; 5] = [
        BandKind::LowCut,
        BandKind::LowShelf,
        BandKind::Peak,
        BandKind::HighShelf,
        BandKind::HighCut,
    ];

    pub fn label(self) -> &'static str {
        match self {
            BandKind::LowShelf => "Low-Shelf",
            BandKind::Peak => "Glocke",
            BandKind::HighShelf => "High-Shelf",
            BandKind::LowCut => "Low-Cut",
            BandKind::HighCut => "High-Cut",
        }
    }

    /// Ob der Gain wirkt (bei den Filtern nicht).
    pub fn uses_gain(self) -> bool {
        !matches!(self, BandKind::LowCut | BandKind::HighCut)
    }
}

/// Grenzen der Regler.
pub const EQ_FREQ: (f32, f32) = (20.0, 20_000.0);
pub const EQ_GAIN: (f32, f32) = (-24.0, 24.0);
pub const EQ_Q: (f32, f32) = (0.1, 12.0);

/// Ein Band.
#[derive(Debug, Clone, Copy, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct EqBand {
    pub on: bool,
    pub kind: BandKind,
    /// Frequenz in Hz.
    pub freq: f32,
    /// Verstärkung in dB.
    pub gain: f32,
    /// Güte.
    pub q: f32,
}

impl EqBand {
    /// Ob das Band den Klang ändert.
    pub fn active(&self) -> bool {
        self.on && (!self.kind.uses_gain() || self.gain.abs() > 1e-3)
    }

    /// Werte in ihre Grenzen gezwungen, NaN durch Standardwerte ersetzt.
    pub fn sanitized(mut self) -> Self {
        let fix =
            |v: f32, (lo, hi): (f32, f32), d: f32| if v.is_finite() { v.clamp(lo, hi) } else { d };
        self.freq = fix(self.freq, EQ_FREQ, 1000.0);
        self.gain = fix(self.gain, EQ_GAIN, 0.0);
        self.q = fix(self.q, EQ_Q, 0.7);
        self
    }
}

/// Der EQ eines Kanalzugs.
#[derive(Debug, Clone, Copy, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct ChannelEq {
    pub on: bool,
    pub bands: [EqBand; 4],
}

impl Default for ChannelEq {
    /// Neutral: alle Bänder an, aber ohne Gain; Low-Shelf 100 Hz, Glocken 500 Hz und 2 kHz,
    /// High-Shelf 8 kHz.
    fn default() -> Self {
        let band = |kind, freq, q| EqBand {
            on: true,
            kind,
            freq,
            gain: 0.0,
            q,
        };
        Self {
            on: true,
            bands: [
                band(BandKind::LowShelf, 100.0, 0.7),
                band(BandKind::Peak, 500.0, 1.0),
                band(BandKind::Peak, 2000.0, 1.0),
                band(BandKind::HighShelf, 8000.0, 0.7),
            ],
        }
    }
}

impl ChannelEq {
    /// Ob der EQ den Klang ändert.
    pub fn active(&self) -> bool {
        self.on && self.bands.iter().any(EqBand::active)
    }

    /// Ob alles auf dem Standard steht (wird dann nicht gespeichert).
    pub fn is_default(&self) -> bool {
        *self == Self::default()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn neutral_eq_does_nothing_until_a_band_has_gain() {
        let mut eq = ChannelEq::default();
        assert!(!eq.active());
        eq.bands[1].gain = 3.0;
        assert!(eq.active());
        eq.on = false;
        assert!(!eq.active());
        let mut cut = ChannelEq::default();
        cut.bands[0].kind = BandKind::LowCut;
        assert!(cut.active(), "a cut filter works without gain");
    }

    #[test]
    fn out_of_range_values_are_clamped() {
        let b = EqBand {
            on: true,
            kind: BandKind::Peak,
            freq: f32::NAN,
            gain: 99.0,
            q: 0.0,
        }
        .sanitized();
        assert_eq!((b.freq, b.gain, b.q), (1000.0, 24.0, 0.1));
    }
}
