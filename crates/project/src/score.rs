//! Angaben für den Noten-Editor (Issue #40): die Tonart des Projekts und die
//! Anzeige-Quantisierung eines Parts. Beide ändern nur die Darstellung als Noten, nie die
//! gespeicherten Zeiten oder Tonhöhen.

use crate::time::TICKS_PER_QUARTER;

/// Tonart: Anzahl der Vorzeichen (positiv Kreuze, negativ Be, −7 bis 7) und Dur oder Moll.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, serde::Serialize, serde::Deserialize)]
pub struct KeySignature {
    pub fifths: i8,
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub minor: bool,
}

impl KeySignature {
    pub const C_MAJOR: Self = Self {
        fifths: 0,
        minor: false,
    };

    pub fn is_c_major(&self) -> bool {
        *self == Self::C_MAJOR
    }

    /// Gültige Tonart (Vorzeichen auf −7 bis 7 begrenzt).
    pub fn sane(self) -> Self {
        Self {
            fifths: self.fifths.clamp(-7, 7),
            minor: self.minor,
        }
    }

    /// Name der Tonart in deutscher Schreibweise, z. B. „Es-Dur“ oder „fis-Moll“.
    pub fn name(self) -> String {
        const MAJOR: [&str; 15] = [
            "Ces", "Ges", "Des", "As", "Es", "B", "F", "C", "G", "D", "A", "E", "H", "Fis", "Cis",
        ];
        const MINOR: [&str; 15] = [
            "as", "es", "b", "f", "c", "g", "d", "a", "e", "h", "fis", "cis", "gis", "dis", "ais",
        ];
        let i = usize::try_from(i16::from(self.sane().fifths) + 7).unwrap_or(7);
        if self.minor {
            format!("{}-Moll", MINOR[i])
        } else {
            format!("{}-Dur", MAJOR[i])
        }
    }
}

/// Erlaubte Anzeige-Quantisierungen in Ticks: Viertel, Achtel, Sechzehntel, Zweiunddreißigstel.
pub const SCORE_QUANTIZE: [u64; 4] = [
    TICKS_PER_QUARTER,
    TICKS_PER_QUARTER / 2,
    TICKS_PER_QUARTER / 4,
    TICKS_PER_QUARTER / 8,
];

/// Vorgabe der Anzeige-Quantisierung: Sechzehntel.
pub const DEFAULT_SCORE_QUANTIZE: u64 = TICKS_PER_QUARTER / 4;

/// Die Anzeige-Quantisierung eines Parts: sein Wert, falls erlaubt, sonst die Vorgabe.
pub fn score_quantize(value: Option<u64>) -> u64 {
    value
        .filter(|v| SCORE_QUANTIZE.contains(v))
        .unwrap_or(DEFAULT_SCORE_QUANTIZE)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn names_and_limits() {
        assert_eq!(KeySignature::C_MAJOR.name(), "C-Dur");
        let es = KeySignature {
            fifths: -3,
            minor: false,
        };
        assert_eq!(es.name(), "Es-Dur");
        let fis = KeySignature {
            fifths: 3,
            minor: true,
        };
        assert_eq!(fis.name(), "fis-Moll");
        let wild = KeySignature {
            fifths: 100,
            minor: false,
        };
        assert_eq!(wild.sane().fifths, 7);
        assert_eq!(wild.name(), "Cis-Dur");
        assert_eq!(score_quantize(None), 240);
        assert_eq!(score_quantize(Some(480)), 480);
        assert_eq!(score_quantize(Some(7)), 240);
    }

    #[test]
    fn key_and_quantize_survive_the_file_and_hostile_values_are_fixed() {
        let mut p = crate::Project::default();
        let t = p.add_track(crate::TrackKind::Instrument, "Klavier");
        let id = p.create_event(t, 0, 3840);
        p.key = KeySignature {
            fifths: -2,
            minor: true,
        };
        if let Some(e) = id.and_then(|id| p.event_mut(id)) {
            e.score_quantize = Some(480);
        }
        let back = crate::file::from_json(&crate::file::to_json(&p)).unwrap();
        assert_eq!(back.key, p.key);
        assert_eq!(back.tracks[0].events[0].score_quantize, Some(480));
        // Defaults stay out of the file; older files load with them.
        let plain = crate::file::to_json(&crate::Project::default());
        assert!(!plain.contains("\"key\"") && !plain.contains("score_quantize"));

        let mut bad = back;
        bad.key.fifths = -40;
        bad.tracks[0].events[0].score_quantize = Some(1);
        assert!(bad.sanitize() >= 2);
        assert_eq!(bad.key.fifths, -7);
        assert_eq!(bad.tracks[0].events[0].score_quantize, None);
    }
}
