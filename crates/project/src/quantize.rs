//! Quantisierung (Spec „Raster, Snap und Bearbeitung“ → Quantisierung).
//!
//! Ein globaler Quantisierungswert von 1/1 bis 1/64, gerade, triolisch oder punktiert. Swing
//! verschiebt jede zweite Rasterposition nach rechts, bei 100 % um einen halben Rasterabstand.
//! Die Stärke bestimmt, wie weit eine Position zum Ziel rückt (100 % = ganz).
//! Das Raster zählt ab Projektanfang, Noten werden also an ihrer Projektposition gemessen.

use crate::{EventId, Project, TICKS_PER_QUARTER};

/// Notenwert des Quantisierungsrasters.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum NoteValue {
    Whole,
    Half,
    Quarter,
    Eighth,
    Sixteenth,
    ThirtySecond,
    SixtyFourth,
}

impl NoteValue {
    pub const ALL: [NoteValue; 7] = [
        NoteValue::Whole,
        NoteValue::Half,
        NoteValue::Quarter,
        NoteValue::Eighth,
        NoteValue::Sixteenth,
        NoteValue::ThirtySecond,
        NoteValue::SixtyFourth,
    ];

    /// Nenner: 1, 2, 4 … 64.
    pub fn denominator(self) -> u64 {
        match self {
            NoteValue::Whole => 1,
            NoteValue::Half => 2,
            NoteValue::Quarter => 4,
            NoteValue::Eighth => 8,
            NoteValue::Sixteenth => 16,
            NoteValue::ThirtySecond => 32,
            NoteValue::SixtyFourth => 64,
        }
    }
}

/// Gerade, triolisch oder punktiert.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Default)]
pub enum Feel {
    #[default]
    Straight,
    Triplet,
    Dotted,
}

impl Feel {
    pub const ALL: [Feel; 3] = [Feel::Straight, Feel::Triplet, Feel::Dotted];
}

/// Quantisierungswert, z. B. 1/16 oder 1/8 triolisch.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct QuantizeValue {
    pub note: NoteValue,
    pub feel: Feel,
}

impl Default for QuantizeValue {
    fn default() -> Self {
        Self {
            note: NoteValue::Sixteenth,
            feel: Feel::Straight,
        }
    }
}

impl QuantizeValue {
    /// Alle Werte in Menü-Reihenfolge: je Notenwert gerade, triolisch, punktiert.
    pub fn all() -> impl Iterator<Item = QuantizeValue> {
        NoteValue::ALL
            .into_iter()
            .flat_map(|note| Feel::ALL.into_iter().map(move |feel| Self { note, feel }))
    }

    /// Rasterabstand in Ticks. Alle Werte gehen bei 960 Ticks pro Viertel glatt auf.
    pub fn ticks(self) -> u64 {
        let straight = TICKS_PER_QUARTER * 4 / self.note.denominator();
        match self.feel {
            Feel::Straight => straight,
            Feel::Triplet => straight * 2 / 3,
            Feel::Dotted => straight * 3 / 2,
        }
    }

    /// Kurzform für die Werkzeugzeile: „1/16“, „1/8 T“, „1/4 P“.
    pub fn short_label(self) -> String {
        let n = self.note.denominator();
        match self.feel {
            Feel::Straight => format!("1/{n}"),
            Feel::Triplet => format!("1/{n} T"),
            Feel::Dotted => format!("1/{n} P"),
        }
    }

    /// Langform für Menüs: „1/8 triolisch“.
    pub fn label(self) -> String {
        let n = self.note.denominator();
        match self.feel {
            Feel::Straight => format!("1/{n}"),
            Feel::Triplet => format!("1/{n} triolisch"),
            Feel::Dotted => format!("1/{n} punktiert"),
        }
    }
}

/// Quantisierungswert mit Swing und Stärke (beide 0–100 %).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct Quantize {
    pub value: QuantizeValue,
    pub swing: u8,
    pub strength: u8,
}

impl Default for Quantize {
    fn default() -> Self {
        Self {
            value: QuantizeValue::default(),
            swing: 0,
            strength: 100,
        }
    }
}

impl Quantize {
    /// Rasterposition `k` mit Swing (ungerade Positionen rücken nach rechts).
    fn grid_point(&self, k: i64, step: i64) -> i64 {
        let base = k.saturating_mul(step);
        if k.rem_euclid(2) == 1 {
            let swing = i64::from(self.swing.min(100));
            base.saturating_add(step * swing / 200)
        } else {
            base
        }
    }

    /// Nächste Rasterposition zu `t` (bei Gleichstand die frühere).
    pub fn target(&self, t: i64) -> i64 {
        let step = i64::try_from(self.value.ticks()).unwrap_or(i64::MAX).max(1);
        let k = t.div_euclid(step);
        let a = self.grid_point(k, step);
        let b = self.grid_point(k.saturating_add(1), step);
        // Mit Swing kann der Punkt `k` rechts von `t` liegen; dann ist `k - 1` gerade und
        // höchstens einen ganzen Schritt entfernt, also nie näher als `k`.
        if t.abs_diff(b) < t.abs_diff(a) { b } else { a }
    }

    /// `t` um die Stärke zum Ziel gerückt, auf ganze Ticks gerundet.
    pub fn apply(&self, t: i64) -> i64 {
        let target = self.target(t);
        let strength = i128::from(self.strength.min(100));
        let delta = i128::from(target) - i128::from(t);
        let moved = (delta * strength * 2 + delta.signum() * 100) / 200;
        i64::try_from(i128::from(t) + moved).unwrap_or(t)
    }
}

impl Project {
    /// Quantisiert den Anfang der Noten `indices` (alle bei `None`) im MIDI-Part `id`.
    /// Längen bleiben. Gibt zurück, ob sich etwas geändert hat.
    pub fn quantize_notes(&mut self, id: EventId, indices: Option<&[usize]>, q: &Quantize) -> bool {
        let Some(ev) = self.event_mut(id) else {
            return false;
        };
        let Ok(part) = i64::try_from(ev.start.0) else {
            return false;
        };
        let mut changed = false;
        for (i, note) in ev.notes.iter_mut().enumerate() {
            if indices.is_some_and(|sel| !sel.contains(&i)) {
                continue;
            }
            let abs = part.saturating_add(note.start);
            let start = q.apply(abs).saturating_sub(part);
            changed |= start != note.start;
            note.start = start;
        }
        changed
    }

    /// Quantisiert ein Event: MIDI-Parts quantisieren ihre Noten, alle anderen Events rücken
    /// mit ihrem Anfang aufs Raster. Gibt zurück, ob sich etwas geändert hat.
    pub fn quantize_event(&mut self, id: EventId, q: &Quantize) -> bool {
        let Some((t, i)) = self.find(id) else {
            return false;
        };
        let ev = &self.tracks[t].events[i];
        if !ev.notes.is_empty() {
            return self.quantize_notes(id, None, q);
        }
        let Ok(start) = i64::try_from(ev.start.0) else {
            return false;
        };
        let new = u64::try_from(q.apply(start).max(0)).unwrap_or(0);
        if new == ev.start.0 {
            return false;
        }
        self.move_event(id, t, new, false).is_some()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{Note, Position, TrackKind};

    const SIXTEENTH: i64 = 240;

    fn q(note: NoteValue, feel: Feel, swing: u8, strength: u8) -> Quantize {
        Quantize {
            value: QuantizeValue { note, feel },
            swing,
            strength,
        }
    }

    #[test]
    fn every_value_has_whole_ticks() {
        for v in QuantizeValue::all() {
            let straight = TICKS_PER_QUARTER * 4 / v.note.denominator();
            let t = v.ticks();
            assert!(t > 0, "{}", v.label());
            match v.feel {
                Feel::Straight => assert_eq!(t, straight),
                Feel::Triplet => assert_eq!(t * 3, straight * 2, "{}", v.label()),
                Feel::Dotted => assert_eq!(t * 2, straight * 3, "{}", v.label()),
            }
        }
        assert_eq!(QuantizeValue::all().count(), 21);
        let eighth_triplet = QuantizeValue {
            note: NoteValue::Eighth,
            feel: Feel::Triplet,
        };
        assert_eq!(eighth_triplet.ticks(), 320);
        assert_eq!(eighth_triplet.short_label(), "1/8 T");
    }

    #[test]
    fn full_strength_snaps_to_the_nearest_line() {
        let q = q(NoteValue::Sixteenth, Feel::Straight, 0, 100);
        assert_eq!(q.apply(0), 0);
        assert_eq!(q.apply(119), 0);
        assert_eq!(q.apply(120), 0, "tie goes to the earlier line");
        assert_eq!(q.apply(121), SIXTEENTH);
        assert_eq!(q.apply(250), SIXTEENTH);
        assert_eq!(q.apply(-100), 0);
        assert_eq!(q.apply(-130), -SIXTEENTH);
    }

    #[test]
    fn strength_moves_part_of_the_way() {
        let half = q(NoteValue::Sixteenth, Feel::Straight, 0, 50);
        assert_eq!(half.apply(100), 50);
        assert_eq!(half.apply(140), 190);
        let none = q(NoteValue::Sixteenth, Feel::Straight, 0, 0);
        assert_eq!(none.apply(100), 100);
    }

    #[test]
    fn swing_shifts_every_second_line() {
        let q = q(NoteValue::Eighth, Feel::Straight, 100, 100);
        // 1/8 = 480 ticks; the off-beat moves half a step to 720.
        assert_eq!(q.target(0), 0);
        assert_eq!(q.target(500), 720);
        assert_eq!(q.target(900), 960);
        assert_eq!(q.target(1500), 1680);
        let fifty = Quantize { swing: 50, ..q };
        assert_eq!(fifty.target(480), 600);
    }

    #[test]
    fn extreme_positions_do_not_overflow() {
        let q = q(NoteValue::Whole, Feel::Dotted, 100, 100);
        let _ = q.apply(i64::MAX);
        let _ = q.apply(i64::MIN);
        let odd = Quantize {
            swing: 255,
            strength: 255,
            ..q
        };
        let _ = odd.apply(i64::MAX - 1);
    }

    fn project_with_part() -> (Project, EventId) {
        let mut p = Project::default();
        let t = p.add_track(TrackKind::Instrument, "Synth");
        let id = p.create_event(t, 960, 3840).expect("part");
        let ev = p.event_mut(id).expect("part");
        for start in [10, 250, 470] {
            ev.notes.push(Note {
                pitch: 60,
                velocity: 100,
                start,
                length: 100,
            });
        }
        (p, id)
    }

    #[test]
    fn notes_quantize_at_their_project_position() {
        let (mut p, id) = project_with_part();
        // Part at 960 sits on the grid, so relative starts snap to multiples of 240.
        let q = Quantize::default();
        assert!(p.quantize_notes(id, Some(&[1, 2]), &q));
        let starts: Vec<i64> = p
            .event(id)
            .expect("part")
            .notes
            .iter()
            .map(|n| n.start)
            .collect();
        assert_eq!(starts, [10, 240, 480]);
        assert!(p.quantize_event(id, &q));
        let starts: Vec<i64> = p
            .event(id)
            .expect("part")
            .notes
            .iter()
            .map(|n| n.start)
            .collect();
        assert_eq!(starts, [0, 240, 480]);
        assert!(!p.quantize_event(id, &q), "nothing left to change");
    }

    #[test]
    fn events_without_notes_move_their_start() {
        let mut p = Project::default();
        let t = p.add_track(TrackKind::Audio, "Audio");
        let id = p.create_event(t, 1000, 960).expect("event");
        assert!(p.quantize_event(id, &Quantize::default()));
        assert_eq!(p.event(id).expect("event").start, Position(960));
    }
}
