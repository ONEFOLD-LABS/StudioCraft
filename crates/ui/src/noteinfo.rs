//! Info-Zeile des Key-Editors (Spec „Editoren“): Anfang, Länge, Tonhöhe und Velocity der
//! gewählten Noten, direkt editierbar. Bei mehreren Noten zeigen die Felder die früheste,
//! die Beschriftungen sind gelb und jede Änderung wirkt relativ auf alle (Anfang einen Takt
//! später: alle einen Takt später; Tonhöhe +2: alle zwei Halbtöne höher). Jede Änderung ist
//! ein Rückgängig-Schritt.

use crate::i18n::tr;
use egui::{RichText, Ui};
use studiocraft_project::{EventId, Position};

use crate::ProjectWindow;
use crate::infoline::field;
use crate::keyeditor::note_name;

/// Eine Änderung aus der Info-Zeile, als Zielwert für die Bezugsnote.
#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) enum NoteEdit {
    /// Anfang in Projekt-Ticks.
    Start(u64),
    Length(u64),
    Pitch(u8),
    Velocity(u8),
}

/// Tonhöhe aus „C3“, „F#2“, „H-1“ (mittleres C = C3) oder einer Zahl 0–127.
pub(crate) fn parse_pitch(text: &str) -> Option<u8> {
    let t = text.trim();
    if let Ok(n) = t.parse::<u8>() {
        return (n <= 127).then_some(n);
    }
    let names = [
        "C", "C#", "D", "D#", "E", "F", "F#", "G", "G#", "A", "A#", "H",
    ];
    let upper = t.to_uppercase().replace('B', "H");
    let (k, rest) = names
        .iter()
        .enumerate()
        .filter(|(_, n)| upper.starts_with(*n))
        .max_by_key(|(_, n)| n.len())
        .map(|(k, n)| (k, &upper[n.len()..]))?;
    let octave: i32 = rest.trim().parse().ok()?;
    let pitch = (octave + 2) * 12 + k as i32;
    u8::try_from(pitch).ok().filter(|p| *p <= 127)
}

impl ProjectWindow {
    /// Die gewählten Noten des Parts, die es gibt, und der Index der frühesten.
    fn selected_notes(&self, part: EventId) -> Option<(Vec<usize>, usize)> {
        let ev = self.project.event(part)?;
        let sel: Vec<usize> = self
            .key_editor
            .selection
            .iter()
            .copied()
            .filter(|&i| i < ev.notes.len())
            .collect();
        let first = sel
            .iter()
            .copied()
            .min_by_key(|&i| (ev.notes[i].start, ev.notes[i].pitch, i))?;
        Some((sel, first))
    }

    /// Felder der Info-Zeile für die gewählten Noten von `part` (nichts ohne Auswahl).
    pub(crate) fn note_info(&mut self, ui: &mut Ui, part: EventId) {
        let Some((sel, first)) = self.selected_notes(part) else {
            return;
        };
        let Some(ev) = self.project.event(part) else {
            return;
        };
        let n = ev.notes[first];
        let multi = sel.len() > 1;
        if multi {
            let t = crate::theme::tokens(ui.ctx());
            ui.label(
                RichText::new(crate::i18n::trf("{n} Noten", &[("n", &sel.len())])).color(t.solo),
            )
            .on_hover_text(tr(
                "Werte der ersten Note; Änderungen wirken relativ auf alle",
            ));
        }
        let tempo = &self.project.tempo;
        let start = ev.start.0.saturating_add_signed(n.start);
        let mut edits = Vec::new();
        if let Some(t) = field(
            ui,
            tr("Anfang"),
            tempo.format_position(Position(start)),
            80.0,
            multi,
        ) {
            edits.extend(tempo.parse_position(&t).map(|p| NoteEdit::Start(p.0)));
        }
        if let Some(t) = field(ui, tr("Länge"), tempo.format_length(n.length), 70.0, multi) {
            edits.extend(tempo.parse_length(&t).map(NoteEdit::Length));
        }
        if let Some(t) = field(ui, tr("Tonhöhe"), note_name(n.pitch), 44.0, multi) {
            match parse_pitch(&t) {
                Some(p) => edits.push(NoteEdit::Pitch(p)),
                None => {
                    self.message = Some(crate::i18n::trf(
                        "Tonhöhe „{t}“ nicht verstanden",
                        &[("t", &t)],
                    ))
                }
            }
        }
        if let Some(t) = field(ui, tr("Velocity"), n.velocity.to_string(), 36.0, multi) {
            match t.trim().parse::<u8>() {
                Ok(v) => edits.push(NoteEdit::Velocity(v.clamp(1, 127))),
                Err(_) => {
                    self.message = Some(crate::i18n::trf(
                        "Velocity „{t}“ nicht verstanden",
                        &[("t", &t)],
                    ))
                }
            }
        }
        for e in edits {
            self.apply_note_edit(part, e);
        }
    }

    /// Wendet `edit` an: die früheste gewählte Note bekommt den Wert, alle anderen gewählten
    /// dieselbe Änderung. Werte bleiben gültig (Tonhöhe 0–127, Velocity 1–127, Länge ≥ 1).
    pub(crate) fn apply_note_edit(&mut self, part: EventId, edit: NoteEdit) {
        let Some((sel, first)) = self.selected_notes(part) else {
            return;
        };
        let before = self.project.clone();
        let Some(ev) = self.project.event_mut(part) else {
            return;
        };
        let part_start = i64::try_from(ev.start.0).unwrap_or(i64::MAX);
        let r = ev.notes[first];
        for i in sel {
            let n = &mut ev.notes[i];
            match edit {
                NoteEdit::Start(t) => {
                    let target = i64::try_from(t).unwrap_or(i64::MAX) - part_start;
                    n.start = n.start.saturating_add(target - r.start);
                }
                NoteEdit::Length(len) => {
                    let d = i64::try_from(len).unwrap_or(i64::MAX)
                        - i64::try_from(r.length).unwrap_or(i64::MAX);
                    n.length = n.length.saturating_add_signed(d).max(1);
                }
                NoteEdit::Pitch(p) => {
                    let d = i32::from(p) - i32::from(r.pitch);
                    n.pitch = (i32::from(n.pitch) + d).clamp(0, 127) as u8;
                }
                NoteEdit::Velocity(v) => {
                    let d = i32::from(v) - i32::from(r.velocity);
                    n.velocity = (i32::from(n.velocity) + d).clamp(1, 127) as u8;
                }
            }
        }
        self.checkpoint(before);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pitch_names_and_numbers() {
        assert_eq!(parse_pitch("C3"), Some(60));
        assert_eq!(parse_pitch("c#3"), Some(61));
        assert_eq!(parse_pitch("H2"), Some(59));
        assert_eq!(parse_pitch("B2"), Some(59));
        assert_eq!(parse_pitch("C-2"), Some(0));
        assert_eq!(parse_pitch("G8"), Some(127));
        assert_eq!(parse_pitch("G#8"), None);
        assert_eq!(parse_pitch("64"), Some(64));
        assert_eq!(parse_pitch("200"), None);
        assert_eq!(parse_pitch("X3"), None);
        for p in 0..=127u8 {
            assert_eq!(parse_pitch(&note_name(p)), Some(p), "{}", note_name(p));
        }
    }
}
