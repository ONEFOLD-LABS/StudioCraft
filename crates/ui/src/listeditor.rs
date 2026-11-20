//! List-Editor (Spec „Editoren“, Issue #39): alle Noten und Controller des aktiven Parts als
//! Tabelle mit Position, Länge, zwei Datenwerten und Kanal. Werte lassen sich direkt eingeben,
//! die Tabelle nach Event-Typ filtern. Der aktive Part und die Auswahl sind dieselben wie im
//! Key-Editor; eine Zeile anklicken wählt die Note bzw. den Controller-Punkt dort mit.
//!
//! Rückgängig: Die Zone gehört zum Panel-Undo (`panel_undo.rs`): eine Eingabe oder ein Zug an
//! einem Wert ist ein Schritt.

use egui::{DragValue, Key, RichText, TextEdit, Ui};
use studiocraft_project::{CtrlKind, CtrlPoint, EventId, Note, Position};

use crate::ProjectWindow;
use crate::i18n::tr;

/// Höhe einer Tabellenzeile.
const ROW_H: f32 = 22.0;
/// Spaltenbreiten: Typ, Position, Länge, Daten 1, Daten 2, Kanal.
const COLS: [f32; 6] = [120.0, 100.0, 90.0, 150.0, 80.0, 50.0];

/// Welche Event-Typen die Tabelle zeigt.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct ListFilter {
    pub notes: bool,
    pub cc: bool,
    pub pitch_bend: bool,
    pub aftertouch: bool,
}

impl Default for ListFilter {
    fn default() -> Self {
        Self {
            notes: true,
            cc: true,
            pitch_bend: true,
            aftertouch: true,
        }
    }
}

impl ListFilter {
    fn shows(&self, kind: CtrlKind) -> bool {
        match kind {
            CtrlKind::Cc(_) => self.cc,
            CtrlKind::PitchBend => self.pitch_bend,
            CtrlKind::Aftertouch => self.aftertouch,
        }
    }
}

/// Eine Zeile: eine Note oder ein Controller-Punkt (Index im Part).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub(crate) enum Row {
    Note(usize),
    Ctrl(usize),
}

/// Eine Änderung aus der Tabelle.
#[derive(Debug, Clone, Copy, PartialEq)]
enum Edit {
    /// Neuer Anfang relativ zum Part.
    Start(i64),
    Length(u64),
    Pitch(u8),
    Velocity(u8),
    CcNumber(u8),
    Value(u16),
}

/// Die Zeilen von `ev` in zeitlicher Reihenfolge (bei gleicher Zeit Noten zuerst).
pub(crate) fn rows(ev: &studiocraft_project::Event, filter: ListFilter) -> Vec<Row> {
    let mut rows: Vec<(i64, u8, Row)> = Vec::new();
    if filter.notes {
        rows.extend(
            ev.notes
                .iter()
                .enumerate()
                .map(|(i, n)| (n.start, n.pitch, Row::Note(i))),
        );
    }
    rows.extend(
        ev.controllers
            .iter()
            .enumerate()
            .filter(|(_, c)| filter.shows(c.kind))
            .map(|(i, c)| (c.start, u8::MAX, Row::Ctrl(i))),
    );
    rows.sort_by_key(|&(start, order, row)| {
        let i = match row {
            Row::Note(i) | Row::Ctrl(i) => i,
        };
        (start, order, i)
    });
    rows.into_iter().map(|(_, _, r)| r).collect()
}

impl ProjectWindow {
    /// Part des List-Editors: der aktive Part des Key-Editors, sonst der erste gewählte
    /// MIDI-Part (er wird dann auch im Key-Editor aktiv).
    fn list_part(&mut self) -> Option<EventId> {
        if let Some(p) = self
            .key_editor
            .part
            .filter(|p| self.project.event(*p).is_some())
        {
            return Some(p);
        }
        let part = self.selection.iter().copied().find(|&id| {
            self.project.find(id).is_some_and(|(t, i)| {
                let track = &self.project.tracks[t];
                track.kind != studiocraft_project::TrackKind::Audio
                    && track.events[i].clip.is_none()
            })
        })?;
        self.key_editor.part = Some(part);
        self.key_editor.open = vec![part];
        self.key_editor.selection.clear();
        self.key_editor.ctrl_sel.clear();
        Some(part)
    }

    /// Wendet eine Änderung auf eine Zeile des Parts an (Werte begrenzt).
    fn list_apply(&mut self, part: EventId, row: Row, edit: Edit) {
        let Some(ev) = self.project.event_mut(part) else {
            return;
        };
        match row {
            Row::Note(i) => {
                let Some(n) = ev.notes.get_mut(i) else { return };
                match edit {
                    Edit::Start(s) => n.start = s,
                    Edit::Length(l) => n.length = l.max(1),
                    Edit::Pitch(p) => n.pitch = p.min(127),
                    Edit::Velocity(v) => n.velocity = v.clamp(1, 127),
                    Edit::CcNumber(_) | Edit::Value(_) => {}
                }
            }
            Row::Ctrl(i) => {
                let Some(c) = ev.controllers.get_mut(i) else {
                    return;
                };
                match edit {
                    Edit::Start(s) => c.start = s,
                    Edit::CcNumber(n) => {
                        if matches!(c.kind, CtrlKind::Cc(_)) {
                            c.kind = CtrlKind::Cc(n.min(127));
                            c.value = c.value.min(c.kind.max());
                        }
                    }
                    Edit::Value(v) => c.value = v.min(c.kind.max()),
                    Edit::Length(_) | Edit::Pitch(_) | Edit::Velocity(_) => {}
                }
            }
        }
    }

    /// Löscht die gewählten Noten und Controller-Punkte des Parts.
    fn list_delete_selection(&mut self, part: EventId) {
        let notes = std::mem::take(&mut self.key_editor.selection);
        let ctrls = std::mem::take(&mut self.key_editor.ctrl_sel);
        let Some(ev) = self.project.event_mut(part) else {
            return;
        };
        let mut i = 0;
        ev.notes.retain(|_| {
            i += 1;
            !notes.contains(&(i - 1))
        });
        let mut i = 0;
        ev.controllers.retain(|_| {
            i += 1;
            !ctrls.contains(&(i - 1))
        });
    }

    /// Fügt eine Note am Cursor ein (im Part, sonst am Part-Anfang): C3, Anschlag 100, eine
    /// Zählzeit lang. Sie wird die Auswahl.
    fn list_add_note(&mut self, part: EventId) {
        let cursor = self.cursor;
        let beat = self.project.tempo.signature_at(cursor).beat_ticks();
        let Some(ev) = self.project.event_mut(part) else {
            return;
        };
        let start = if ev.contains(cursor) {
            i64::try_from(cursor - ev.start.0).unwrap_or(0)
        } else {
            0
        };
        ev.notes.push(Note {
            pitch: 60,
            velocity: 100,
            start,
            length: beat.max(1),
        });
        self.key_editor.selection = vec![ev.notes.len() - 1];
        self.key_editor.ctrl_sel.clear();
    }

    /// Klick auf den Typ einer Zeile: wählt sie (mit Strg dazu oder ab).
    fn list_select(&mut self, row: Row, add: bool) {
        let ke = &mut self.key_editor;
        if !add {
            ke.selection.clear();
            ke.ctrl_sel.clear();
        }
        let (list, i) = match row {
            Row::Note(i) => (&mut ke.selection, i),
            Row::Ctrl(i) => (&mut ke.ctrl_sel, i),
        };
        if let Some(pos) = list.iter().position(|&x| x == i) {
            if add {
                list.remove(pos);
            }
        } else {
            list.push(i);
        }
    }

    fn list_selected(&self, row: Row) -> bool {
        match row {
            Row::Note(i) => self.key_editor.selection.contains(&i),
            Row::Ctrl(i) => self.key_editor.ctrl_sel.contains(&i),
        }
    }
}

/// Textfeld einer Zelle; gibt den neuen Text zurück, sobald die Eingabe abgeschlossen ist
/// (Enter oder Fokus weg; Escape verwirft).
fn cell_text(ui: &mut Ui, key: (Row, u8), value: String, label: egui::Id) -> Option<String> {
    let id = ui.make_persistent_id(("liste", key));
    let mut text = ui
        .data(|d| d.get_temp::<String>(id))
        .unwrap_or_else(|| value.clone());
    let r = ui
        .add_sized(
            [COLS[usize::from(key.1)], ROW_H - 4.0],
            TextEdit::singleline(&mut text).id(id.with("feld")),
        )
        .labelled_by(label);
    if r.has_focus() {
        ui.data_mut(|d| d.insert_temp(id, text));
        return None;
    }
    ui.data_mut(|d| d.remove_temp::<String>(id));
    let cancelled = ui.input(|i| i.key_pressed(Key::Escape));
    (r.lost_focus() && !cancelled && text != value).then_some(text)
}

/// Zahlenfeld einer Zelle; gibt den neuen Wert zurück, wenn er sich ändert.
fn cell_number(
    ui: &mut Ui,
    col: usize,
    value: u16,
    range: std::ops::RangeInclusive<u16>,
    label: egui::Id,
    format: Option<&dyn Fn(u16) -> String>,
) -> Option<u16> {
    let mut v = value;
    let mut d = DragValue::new(&mut v).range(range).speed(0.25);
    if let Some(f) = format {
        d = d.custom_formatter(move |x, _| f(x as u16));
    }
    let r = ui.add_sized([COLS[col], ROW_H - 4.0], d).labelled_by(label);
    (r.changed() && v != value).then_some(v)
}

fn pitch_name(p: u16) -> String {
    crate::keyeditor::note_name(p.min(127) as u8)
}

/// Tonhöhe mit Drum-Namen, wenn die Map eine Zeile dafür hat: „Snare (D1)“.
fn drum_name(map: &studiocraft_project::DrumMap, p: u16) -> String {
    let pitch = p.min(127) as u8;
    match map.row(pitch) {
        Some(r) => format!("{} ({})", crate::i18n::tr_dyn(&r.name), pitch_name(p)),
        None => pitch_name(p),
    }
}

/// Zeichnet den List-Editor in die untere Zone.
pub(crate) fn show(w: &mut ProjectWindow, ui: &mut Ui) {
    let Some(part) = w.list_part() else {
        ui.centered_and_justified(|ui| {
            ui.weak(tr(
                "Einen MIDI-Part wählen oder öffnen, um seine Events hier zu sehen",
            ))
        });
        return;
    };
    let Some(ev) = w.project.event(part).cloned() else {
        return;
    };
    let channel = w
        .project
        .find(part)
        .map_or(1, |(t, _)| w.project.tracks[t].midi_channel);
    // Spuren mit Drum-Map zeigen die Namen ihrer Zeilen (Issue #161).
    let drums = w
        .project
        .find(part)
        .and_then(|(t, _)| w.project.tracks[t].drum_map.clone());
    let pitch_label = |p: u16| match &drums {
        Some(m) => drum_name(m, p),
        None => pitch_name(p),
    };

    ui.horizontal(|ui| {
        ui.weak(tr("Zeigen:"));
        let f = &mut w.list_filter;
        ui.checkbox(&mut f.notes, tr("Noten"));
        ui.checkbox(&mut f.cc, tr("Controller"));
        ui.checkbox(&mut f.pitch_bend, tr("Pitchbend"));
        ui.checkbox(&mut f.aftertouch, tr("Aftertouch"));
        ui.separator();
        if ui.button(tr("Note einfügen")).clicked() {
            w.list_add_note(part);
        }
        let any = !w.key_editor.selection.is_empty() || !w.key_editor.ctrl_sel.is_empty();
        if ui
            .add_enabled(any, egui::Button::new(tr("Auswahl löschen")))
            .clicked()
        {
            w.list_delete_selection(part);
        }
    });
    ui.separator();

    // Kopfzeile; ihre Beschriftungen benennen die Felder darunter.
    let headers = [
        tr("Typ"),
        tr("Position"),
        tr("Länge"),
        tr("Daten 1"),
        tr("Daten 2"),
        tr("Kanal"),
    ];
    let mut ids = [ui.scope_id(); 6];
    ui.horizontal(|ui| {
        for (k, h) in headers.iter().enumerate() {
            let r = ui.add_sized(
                [COLS[k], ROW_H],
                egui::Label::new(RichText::new(*h).strong()),
            );
            ids[k] = r.id;
        }
    });

    let rows = rows(&ev, w.list_filter);
    if rows.is_empty() {
        ui.weak(tr("Keine Events"));
        return;
    }
    let tempo = w.project.tempo.clone();
    let abs = |rel: i64| ev.start.0.saturating_add_signed(rel);
    let mut edits: Vec<(Row, Edit)> = Vec::new();
    let mut clicked: Option<(Row, bool)> = None;
    egui::ScrollArea::vertical()
        .auto_shrink([false, false])
        .show_rows(ui, ROW_H, rows.len(), |ui, range| {
            for &row in &rows[range] {
                ui.horizontal(|ui| {
                    let (kind, start) = match row {
                        Row::Note(i) => (None, ev.notes[i].start),
                        Row::Ctrl(i) => (Some(ev.controllers[i].kind), ev.controllers[i].start),
                    };
                    let name = kind.map_or_else(
                        || tr("Note").to_owned(),
                        |k| crate::i18n::tr_dyn(&k.label()),
                    );
                    let sel = w.list_selected(row);
                    let r = ui
                        .add_sized([COLS[0], ROW_H - 4.0], egui::Button::selectable(sel, name))
                        .labelled_by(ids[0]);
                    if r.clicked() {
                        clicked = Some((row, ui.input(|i| i.modifiers.command)));
                    }
                    let pos = tempo.format_position(Position(abs(start)));
                    if let Some(t) = cell_text(ui, (row, 1), pos, ids[1])
                        && let Some(p) = tempo.parse_position(&t)
                    {
                        let rel = i64::try_from(p.0).unwrap_or(i64::MAX)
                            - i64::try_from(ev.start.0).unwrap_or(i64::MAX);
                        edits.push((row, Edit::Start(rel)));
                    }
                    match row {
                        Row::Note(i) => {
                            let n: Note = ev.notes[i];
                            let len = tempo.format_length(n.length);
                            if let Some(t) = cell_text(ui, (row, 2), len, ids[2])
                                && let Some(l) = tempo.parse_length(&t)
                            {
                                edits.push((row, Edit::Length(l)));
                            }
                            let p = cell_number(
                                ui,
                                3,
                                u16::from(n.pitch),
                                0..=127,
                                ids[3],
                                Some(&pitch_label),
                            );
                            if let Some(p) = p {
                                edits.push((row, Edit::Pitch(p as u8)));
                            }
                            let v =
                                cell_number(ui, 4, u16::from(n.velocity), 1..=127, ids[4], None);
                            if let Some(v) = v {
                                edits.push((row, Edit::Velocity(v as u8)));
                            }
                        }
                        Row::Ctrl(i) => {
                            let c: CtrlPoint = ev.controllers[i];
                            ui.add_sized([COLS[2], ROW_H - 4.0], egui::Label::new("–"));
                            match c.kind {
                                CtrlKind::Cc(num) => {
                                    let n =
                                        cell_number(ui, 3, u16::from(num), 0..=127, ids[3], None);
                                    if let Some(n) = n {
                                        edits.push((row, Edit::CcNumber(n as u8)));
                                    }
                                }
                                _ => {
                                    ui.add_sized([COLS[3], ROW_H - 4.0], egui::Label::new("–"));
                                }
                            }
                            let v = cell_number(ui, 4, c.value, 0..=c.kind.max(), ids[4], None);
                            if let Some(v) = v {
                                edits.push((row, Edit::Value(v)));
                            }
                        }
                    }
                    ui.add_sized(
                        [COLS[5], ROW_H - 4.0],
                        egui::Label::new(channel.to_string()),
                    );
                });
            }
        });
    if let Some((row, add)) = clicked {
        w.list_select(row, add);
    }
    if !edits.is_empty() {
        for (row, edit) in edits {
            w.list_apply(part, row, edit);
        }
        w.dirty = true;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use studiocraft_project::{Project, TrackKind};

    /// Fenster mit einem MIDI-Part (zwei Noten, zwei Controller-Punkte), gewählt.
    fn window() -> (ProjectWindow, EventId) {
        let mut p = Project::default();
        let t = p.add_track(TrackKind::Midi, "M");
        let id = p.create_event(t, 3840, 3840).unwrap();
        let ev = p.event_mut(id).unwrap();
        ev.notes = vec![
            Note {
                pitch: 64,
                velocity: 90,
                start: 960,
                length: 480,
            },
            Note {
                pitch: 60,
                velocity: 100,
                start: 0,
                length: 960,
            },
        ];
        ev.controllers = vec![
            CtrlPoint {
                kind: CtrlKind::Cc(1),
                start: 960,
                value: 10,
            },
            CtrlPoint {
                kind: CtrlKind::PitchBend,
                start: 480,
                value: 8192,
            },
        ];
        let mut w = ProjectWindow::new(p, None, "kein Audio (Test)".into());
        w.selection = vec![id];
        (w, id)
    }

    #[test]
    fn rows_are_sorted_by_time_and_filtered_by_type() {
        let (w, id) = window();
        let ev = w.project.event(id).unwrap();
        let all = rows(ev, ListFilter::default());
        assert_eq!(
            all,
            vec![Row::Note(1), Row::Ctrl(1), Row::Note(0), Row::Ctrl(0)]
        );
        let only_cc = ListFilter {
            notes: false,
            pitch_bend: false,
            ..ListFilter::default()
        };
        assert_eq!(rows(ev, only_cc), vec![Row::Ctrl(0)]);
    }

    #[test]
    fn edits_change_values_within_their_range() {
        let (mut w, id) = window();
        w.list_apply(id, Row::Note(0), Edit::Pitch(200));
        w.list_apply(id, Row::Note(0), Edit::Velocity(0));
        w.list_apply(id, Row::Note(0), Edit::Length(0));
        w.list_apply(id, Row::Note(0), Edit::Start(-240));
        w.list_apply(id, Row::Ctrl(0), Edit::CcNumber(7));
        w.list_apply(id, Row::Ctrl(1), Edit::Value(60_000));
        // Nicht passende Änderungen und fremde Indizes bleiben ohne Wirkung.
        w.list_apply(id, Row::Ctrl(1), Edit::CcNumber(7));
        w.list_apply(id, Row::Note(9), Edit::Pitch(1));
        let ev = w.project.event(id).unwrap();
        assert_eq!(
            ev.notes[0],
            Note {
                pitch: 127,
                velocity: 1,
                start: -240,
                length: 1
            }
        );
        assert_eq!(ev.controllers[0].kind, CtrlKind::Cc(7));
        assert_eq!(ev.controllers[1].kind, CtrlKind::PitchBend);
        assert_eq!(ev.controllers[1].value, 16_383);
    }

    #[test]
    fn the_selected_part_opens_and_rows_select_notes_in_the_key_editor() {
        let (mut w, id) = window();
        assert_eq!(w.list_part(), Some(id));
        assert_eq!(w.key_editor.part, Some(id));
        w.list_select(Row::Note(1), false);
        w.list_select(Row::Ctrl(0), true);
        assert_eq!(w.key_editor.selection, vec![1]);
        assert_eq!(w.key_editor.ctrl_sel, vec![0]);
        // Strg-Klick auf eine gewählte Zeile wählt sie ab.
        w.list_select(Row::Note(1), true);
        assert!(w.key_editor.selection.is_empty());
        w.list_select(Row::Note(0), false);
        assert!(w.key_editor.ctrl_sel.is_empty());
        w.list_delete_selection(id);
        let ev = w.project.event(id).unwrap();
        assert_eq!(ev.notes.len(), 1);
        assert_eq!(ev.notes[0].pitch, 60);
        assert_eq!(ev.controllers.len(), 2);
    }

    #[test]
    fn a_new_note_lands_at_the_cursor_inside_the_part() {
        let (mut w, id) = window();
        w.cursor = 3840 + 1920;
        w.list_add_note(id);
        w.cursor = 0;
        w.list_add_note(id);
        let ev = w.project.event(id).unwrap();
        let starts: Vec<i64> = ev.notes[2..].iter().map(|n| n.start).collect();
        assert_eq!(starts, vec![1920, 0]);
        assert_eq!(w.key_editor.selection, vec![3]);
    }

    #[test]
    fn drum_tracks_show_drum_names_in_the_pitch_column() {
        let map = studiocraft_project::DrumMap::general_midi();
        assert_eq!(drum_name(&map, 38), format!("Snare ({})", pitch_name(38)));
        // Tonhöhen ohne Zeile zeigen den Tonnamen.
        assert_eq!(drum_name(&map, 100), pitch_name(100));
    }

    #[test]
    fn audio_events_are_not_listed() {
        let mut p = Project::default();
        let t = p.add_track(TrackKind::Audio, "A");
        let id = p.create_event(t, 0, 3840).unwrap();
        let mut w = ProjectWindow::new(p, None, "kein Audio (Test)".into());
        w.selection = vec![id];
        assert_eq!(w.list_part(), None);
    }
}
