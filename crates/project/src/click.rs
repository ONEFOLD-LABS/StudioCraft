//! Metronom und Vorzähler als Projekteinstellungen (Spec „Vorzähler und Metronom“, Issue #14).

/// Klang des Metronoms.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ClickSound {
    /// Heller Klick (1500/1000 Hz).
    #[default]
    Bright,
    /// Dunkler Klick (800/600 Hz).
    Dark,
}

impl ClickSound {
    pub const ALL: [ClickSound; 2] = [Self::Bright, Self::Dark];

    pub fn label(self) -> &'static str {
        match self {
            Self::Bright => "Hell",
            Self::Dark => "Dunkel",
        }
    }

    /// Tonhöhen für Takt-Eins und übrige Zählzeiten in Hz.
    pub fn pitches(self) -> (f32, f32) {
        match self {
            Self::Bright => (1500.0, 1000.0),
            Self::Dark => (800.0, 600.0),
        }
    }
}

/// Metronom- und Vorzähler-Einstellungen eines Projekts.
#[derive(Debug, Clone, Copy, PartialEq, serde::Serialize, serde::Deserialize)]
#[serde(default)]
pub struct ClickSettings {
    /// Vorzähler vor der Aufnahme in Takten (0 = aus).
    pub precount_bars: u32,
    /// Lautstärke des Klicks in dB.
    pub level_db: f32,
    /// Takt-Eins betonen (höherer Ton).
    pub accent: bool,
    pub sound: ClickSound,
}

impl Default for ClickSettings {
    fn default() -> Self {
        Self {
            precount_bars: 0,
            level_db: -6.0,
            accent: true,
            sound: ClickSound::Bright,
        }
    }
}

/// Höchstens so viele Takte Vorzähler.
pub const MAX_PRECOUNT_BARS: u32 = 8;
/// Bereich der Klick-Lautstärke in dB.
pub const CLICK_DB: (f32, f32) = (-40.0, 6.0);

impl ClickSettings {
    pub fn is_default(&self) -> bool {
        *self == Self::default()
    }

    /// Gültige Werte (fremde Dateien).
    pub fn sanitized(self) -> Self {
        Self {
            precount_bars: self.precount_bars.min(MAX_PRECOUNT_BARS),
            level_db: if self.level_db.is_finite() {
                self.level_db.clamp(CLICK_DB.0, CLICK_DB.1)
            } else {
                -6.0
            },
            ..self
        }
    }
}
