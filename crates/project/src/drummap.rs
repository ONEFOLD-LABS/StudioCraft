//! Drum-Maps (Spec „Editoren“ → Drum-Editor, Issue #38).
//!
//! Eine Drum-Map ordnet Tonhöhen Instrumentnamen zu und legt je Zeile fest, wie gezeichnet
//! (eigene Quantisierung, Notenlänge) und ob sie gehört wird (Mute/Solo je Zeile). Jede Spur
//! hat ihre eigene Kopie ([`crate::Track::drum_map`]); mit einer Drum-Map öffnet ein MIDI-Part
//! im Drum-Editor statt im Key-Editor. Mute/Solo wirken auf die Wiedergabe der Spur.

use crate::TICKS_PER_QUARTER;

/// Höchstens so viele Zeilen hat eine Drum-Map (jede Tonhöhe einmal).
pub const MAX_DRUM_ROWS: usize = 128;

/// Eine Instrument-Zeile.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct DrumRow {
    /// MIDI-Tonhöhe 0–127.
    pub pitch: u8,
    pub name: String,
    /// Eigenes Raster dieser Zeile in Ticks; 0: das Raster der Werkzeugzeile.
    #[serde(default, skip_serializing_if = "is_zero")]
    pub quantize: u64,
    /// Länge gezeichneter Noten in Ticks (der Drum-Editor zeigt keine Längen).
    #[serde(default = "default_length")]
    pub length: u64,
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub mute: bool,
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub solo: bool,
    /// Ausgangsnote: Die Zeile zeigt `pitch`, gespielt wird diese Tonhöhe (`None`: dieselbe).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub output: Option<u8>,
}

fn is_zero(v: &u64) -> bool {
    *v == 0
}

/// Standardlänge gezeichneter Noten: eine Zweiunddreißigstel.
pub const DEFAULT_DRUM_LENGTH: u64 = TICKS_PER_QUARTER / 8;

fn default_length() -> u64 {
    DEFAULT_DRUM_LENGTH
}

impl DrumRow {
    pub fn new(pitch: u8, name: impl Into<String>) -> Self {
        Self {
            pitch,
            name: name.into(),
            quantize: 0,
            length: DEFAULT_DRUM_LENGTH,
            mute: false,
            solo: false,
            output: None,
        }
    }
}

/// Dateiendung gespeicherter Drum-Maps.
pub const DRUM_MAP_EXTENSION: &str = "drummap";
/// Größte Drum-Map-Datei, die gelesen wird.
pub const MAX_DRUM_MAP_FILE: usize = 1 << 20;

/// Eine Drum-Map: Name und Zeilen in Anzeigereihenfolge (oben zuerst).
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct DrumMap {
    pub name: String,
    pub rows: Vec<DrumRow>,
}

/// Belegung nach dem General-MIDI-Schlagzeugstandard (Tonhöhe 35–81), allgemeine Namen.
const GM_KIT: [(u8, &str); 47] = [
    (35, "Bassdrum 2"),
    (36, "Bassdrum"),
    (37, "Sidestick"),
    (38, "Snare"),
    (39, "Klatschen"),
    (40, "Snare 2"),
    (41, "Tom tief 2"),
    (42, "Hi-Hat zu"),
    (43, "Tom tief"),
    (44, "Hi-Hat Pedal"),
    (45, "Tom mittel 2"),
    (46, "Hi-Hat offen"),
    (47, "Tom mittel"),
    (48, "Tom hoch 2"),
    (49, "Crash"),
    (50, "Tom hoch"),
    (51, "Ride"),
    (52, "China"),
    (53, "Ride-Glocke"),
    (54, "Tamburin"),
    (55, "Splash"),
    (56, "Kuhglocke"),
    (57, "Crash 2"),
    (58, "Vibraslap"),
    (59, "Ride 2"),
    (60, "Bongo hoch"),
    (61, "Bongo tief"),
    (62, "Conga hoch gedämpft"),
    (63, "Conga hoch offen"),
    (64, "Conga tief"),
    (65, "Timbale hoch"),
    (66, "Timbale tief"),
    (67, "Agogo hoch"),
    (68, "Agogo tief"),
    (69, "Cabasa"),
    (70, "Maracas"),
    (71, "Pfeife kurz"),
    (72, "Pfeife lang"),
    (73, "Guiro kurz"),
    (74, "Guiro lang"),
    (75, "Claves"),
    (76, "Holzblock hoch"),
    (77, "Holzblock tief"),
    (78, "Cuica gedämpft"),
    (79, "Cuica offen"),
    (80, "Triangel gedämpft"),
    (81, "Triangel offen"),
];

impl DrumMap {
    /// Standard-Map nach General MIDI, höchste Tonhöhe oben.
    pub fn general_midi() -> Self {
        Self {
            name: "GM".into(),
            rows: GM_KIT
                .iter()
                .rev()
                .map(|&(pitch, name)| DrumRow::new(pitch, name))
                .collect(),
        }
    }

    /// Zeile einer Tonhöhe.
    pub fn row(&self, pitch: u8) -> Option<&DrumRow> {
        self.rows.iter().find(|r| r.pitch == pitch)
    }

    /// Name einer Tonhöhe; Tonhöhen ohne Zeile heißen nach ihrer Nummer.
    pub fn name_of(&self, pitch: u8) -> String {
        self.row(pitch)
            .map_or_else(|| pitch.to_string(), |r| r.name.clone())
    }

    /// Ob Noten dieser Tonhöhe klingen: nicht stumm und, wenn eine Zeile solo ist, selbst solo.
    /// Tonhöhen ohne Zeile klingen nur, solange keine Zeile solo ist.
    pub fn audible(&self, pitch: u8) -> bool {
        let any_solo = self.rows.iter().any(|r| r.solo);
        match self.row(pitch) {
            Some(r) => !r.mute && (!any_solo || r.solo),
            None => !any_solo,
        }
    }

    /// Tonhöhe, die für Noten der Zeile `pitch` gespielt wird (Ausgangsnote).
    pub fn output_of(&self, pitch: u8) -> u8 {
        self.row(pitch).and_then(|r| r.output).unwrap_or(pitch)
    }

    /// Ob irgendeine Zeile eine andere Ausgangsnote hat.
    pub fn remaps(&self) -> bool {
        self.rows
            .iter()
            .any(|r| r.output.is_some_and(|o| o != r.pitch))
    }

    /// Fügt eine Zeile für die nächste freie Tonhöhe ab `near` (nach oben, dann nach unten)
    /// am Anfang ein. Gibt ihren Index zurück, oder `None`, wenn alle 128 belegt sind.
    pub fn add_row(&mut self, near: u8) -> Option<usize> {
        let used = |p: u8| self.rows.iter().any(|r| r.pitch == p);
        let pitch = (near..=127).chain((0..near).rev()).find(|&p| !used(p))?;
        self.rows.insert(0, DrumRow::new(pitch, pitch.to_string()));
        Some(0)
    }

    /// Verschiebt Zeile `i` um eine Stelle nach oben (`up`) oder unten.
    pub fn move_row(&mut self, i: usize, up: bool) -> bool {
        let j = if up { i.checked_sub(1) } else { Some(i + 1) };
        match j.filter(|&j| j < self.rows.len() && i < self.rows.len()) {
            Some(j) => {
                self.rows.swap(i, j);
                true
            }
            None => false,
        }
    }

    /// Setzt die Tonhöhe von Zeile `i`, wenn keine andere Zeile sie hat.
    pub fn set_pitch(&mut self, i: usize, pitch: u8) -> bool {
        if pitch > 127
            || self
                .rows
                .iter()
                .enumerate()
                .any(|(k, r)| k != i && r.pitch == pitch)
        {
            return false;
        }
        match self.rows.get_mut(i) {
            Some(r) => {
                r.pitch = pitch;
                true
            }
            None => false,
        }
    }

    /// Die Map als Text für eine Datei (JSON).
    pub fn to_file_text(&self) -> Result<String, String> {
        serde_json::to_string_pretty(self).map_err(|e| e.to_string())
    }

    /// Liest eine Map aus einer Datei (bereinigt; fremde Dateien gelten als feindlich).
    pub fn from_file_text(text: &str) -> Result<Self, String> {
        if text.len() > MAX_DRUM_MAP_FILE {
            return Err("Datei zu groß".into());
        }
        let mut m: DrumMap = serde_json::from_str(text).map_err(|e| e.to_string())?;
        m.sanitize();
        Ok(m)
    }

    /// Ob Mute/Solo irgendetwas stummschaltet (dann muss die Wiedergabe filtern).
    pub fn filters(&self) -> bool {
        self.rows.iter().any(|r| r.mute || r.solo)
    }

    /// Bringt eine geladene Map in gültige Grenzen: Tonhöhen 0–127 je einmal, höchstens
    /// [`MAX_DRUM_ROWS`] Zeilen, Länge mindestens 1 Tick, Raster höchstens vier Takte.
    /// Gibt zurück, ob sich etwas geändert hat.
    pub fn sanitize(&mut self) -> bool {
        let before = self.clone();
        let mut seen = [false; 128];
        self.rows.retain(|r| {
            let ok = r.pitch < 128 && !seen[usize::from(r.pitch)];
            if ok {
                seen[usize::from(r.pitch)] = true;
            }
            ok
        });
        self.rows.truncate(MAX_DRUM_ROWS);
        for r in &mut self.rows {
            r.length = r.length.clamp(1, TICKS_PER_QUARTER * 16);
            r.quantize = r.quantize.min(TICKS_PER_QUARTER * 16);
            r.output = r.output.filter(|&o| o < 128);
            if r.name.chars().count() > 64 {
                r.name = r.name.chars().take(64).collect();
            }
        }
        if self.name.chars().count() > 64 {
            self.name = self.name.chars().take(64).collect();
        }
        *self != before
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn general_midi_kit_has_one_row_per_pitch_top_down() {
        let m = DrumMap::general_midi();
        assert_eq!(m.rows.len(), 47);
        assert_eq!(m.rows.first().map(|r| r.pitch), Some(81));
        assert_eq!(m.rows.last().map(|r| r.pitch), Some(35));
        assert_eq!(m.name_of(38), "Snare");
        assert_eq!(m.name_of(20), "20");
        let mut copy = m.clone();
        assert!(!copy.sanitize(), "the default map is valid");
    }

    #[test]
    fn mute_and_solo_decide_what_sounds() {
        let mut m = DrumMap::general_midi();
        assert!(m.audible(36) && m.audible(10) && !m.filters());
        m.rows.iter_mut().find(|r| r.pitch == 42).unwrap().mute = true;
        assert!(!m.audible(42) && m.audible(36) && m.filters());
        m.rows.iter_mut().find(|r| r.pitch == 38).unwrap().solo = true;
        assert!(m.audible(38));
        assert!(!m.audible(36), "another row is solo");
        assert!(
            !m.audible(10),
            "pitches without a row are silent while soloing"
        );
        // Mute wins over solo on the same row.
        m.rows.iter_mut().find(|r| r.pitch == 38).unwrap().mute = true;
        assert!(!m.audible(38));
    }

    #[test]
    fn rows_are_added_moved_repitched_and_remapped() {
        let mut m = DrumMap {
            name: "Kit".into(),
            rows: vec![DrumRow::new(36, "Kick"), DrumRow::new(38, "Snare")],
        };
        assert_eq!(m.add_row(36), Some(0));
        assert_eq!(m.rows[0].pitch, 37, "next free pitch upwards");
        assert!(m.move_row(0, false));
        assert_eq!(m.rows[1].pitch, 37);
        assert!(!m.move_row(0, true), "already on top");
        assert!(!m.move_row(9, false));
        assert!(!m.set_pitch(1, 38), "38 is taken");
        assert!(m.set_pitch(1, 40));
        assert!(!m.remaps());
        m.rows[2].output = Some(40);
        assert!(m.remaps());
        assert_eq!(m.output_of(38), 40);
        assert_eq!(m.output_of(36), 36);
        assert_eq!(m.output_of(99), 99);
        // Alle Tonhöhen belegt: keine Zeile mehr.
        let mut full = DrumMap {
            name: "voll".into(),
            rows: (0..128).map(|p| DrumRow::new(p, "x")).collect(),
        };
        assert_eq!(full.add_row(60), None);
    }

    #[test]
    fn maps_round_trip_through_files_and_bad_files_are_refused() {
        let mut m = DrumMap::general_midi();
        m.rows[0].output = Some(30);
        m.rows[1].mute = true;
        let text = m.to_file_text().unwrap();
        assert_eq!(DrumMap::from_file_text(&text).unwrap(), m);
        assert!(DrumMap::from_file_text("kein json").is_err());
        assert!(DrumMap::from_file_text(&" ".repeat(MAX_DRUM_MAP_FILE + 1)).is_err());
        // Doppelte Tonhöhen und falsche Ausgänge werden bereinigt.
        let bad =
            r#"{"name":"x","rows":[{"pitch":36,"name":"a","output":200},{"pitch":36,"name":"b"}]}"#;
        let m = DrumMap::from_file_text(bad).unwrap();
        assert_eq!(m.rows.len(), 1);
        assert_eq!(m.rows[0].output, None);
    }

    #[test]
    fn sanitize_drops_doubles_and_clamps_values() {
        let mut m = DrumMap {
            name: "x".into(),
            rows: vec![
                DrumRow::new(36, "a"),
                DrumRow::new(36, "b"),
                DrumRow::new(200, "c"),
                DrumRow {
                    length: 0,
                    quantize: u64::MAX,
                    ..DrumRow::new(38, "d".repeat(100))
                },
            ],
        };
        assert!(m.sanitize());
        assert_eq!(m.rows.len(), 2);
        assert_eq!(m.rows[0].name, "a");
        assert_eq!(m.rows[1].length, 1);
        assert_eq!(m.rows[1].quantize, TICKS_PER_QUARTER * 16);
        assert_eq!(m.rows[1].name.chars().count(), 64);
    }
}
