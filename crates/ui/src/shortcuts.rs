//! Bearbeitungs- und Navigationskürzel aus der Standardbelegung (Spec „Tastaturkonzept“,
//! Issue #2): Ausschneiden/Kopieren/Einfügen, Duplizieren, am Cursor teilen, alles auswählen,
//! auf das Projekt zoomen, Spur und Event wechseln, M/S/R der gewählten Spur, F2, Alt+L und
//! Q (Quantisieren), X (Crossfade) und Nudge (Strg+Pfeil: um einen Rasterschritt verschieben,
//! Strg+Umschalt+Pfeil: Länge ändern).
//!
//! Fokus-Regel: Hat der Key-Editor den Fokus, wirken Ausschneiden/Kopieren/Einfügen und Nudge
//! auf die gewählten Noten, Pfeil hoch/runter transponiert sie um einen Halbton.
//!
//! Die Kürzel werden vor [`ProjectWindow::handle_keys`] verbraucht, damit dort keine
//! Doppelbelegung greift (z. B. Alt+X gegen X).

use crate::i18n::tr;
use studiocraft_project::{Event, EventId, Note, Position};

use crate::ProjectWindow;

/// Kopierte Events mit ihrer Spur, für Strg+V.
#[derive(Debug, Clone, Default)]
pub(crate) struct Clipboard {
    events: Vec<(usize, Event)>,
    /// Kopierte Noten, Anfang relativ zur frühesten.
    notes: Vec<Note>,
}

/// Dialog „Wiederholen“.
#[derive(Debug, Clone)]
pub(crate) struct RepeatDialog {
    pub open: bool,
    pub count: u32,
    pub shared: bool,
}

impl Default for RepeatDialog {
    fn default() -> Self {
        Self {
            open: false,
            count: 1,
            shared: false,
        }
    }
}

impl ProjectWindow {
    /// Wiederholt die Auswahl `count` Mal hintereinander, auf Wunsch als geteilte Kopien.
    pub(crate) fn repeat_selection(&mut self, count: u32, shared: bool) {
        let before = self.project.clone();
        let ids = self.selection.clone();
        let new = self.project.repeat(&ids, count, shared);
        if !new.is_empty() {
            self.selection = new;
        }
        self.checkpoint(before);
    }

    /// Zeichnet den Dialog „Wiederholen“, solange er offen ist.
    pub(crate) fn repeat_dialog(&mut self, ctx: &egui::Context) {
        if !self.repeat_dialog.open {
            return;
        }
        let mut open = true;
        let mut run = false;
        self.dialogs.show(
            ctx,
            crate::dialog::window_id(tr("Wiederholen")),
            &mut open,
            crate::dialog::Kind::Modal,
        );
        egui::Window::new(tr("Wiederholen"))
            .open(&mut open)
            .resizable(false)
            .collapsible(false)
            .show(ctx, |ui| {
                ui.horizontal(|ui| {
                    let l = ui.label(tr("Anzahl"));
                    ui.add(egui::DragValue::new(&mut self.repeat_dialog.count).range(1..=99))
                        .labelled_by(l.id);
                });
                ui.checkbox(&mut self.repeat_dialog.shared, tr("Geteilte Kopien"));
                if ui.button(tr("Wiederholen")).clicked() {
                    run = true;
                }
            });
        if run {
            let (n, shared) = (self.repeat_dialog.count, self.repeat_dialog.shared);
            self.repeat_selection(n, shared);
            open = false;
        }
        self.repeat_dialog.open = open;
    }
}

/// Ein Befehl aus der Standardbelegung.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub(crate) enum EditCommand {
    Cut,
    Copy,
    Paste,
    Duplicate,
    SplitAtCursor,
    SelectAll,
    ZoomToProject,
    PrevTrack,
    NextTrack,
    /// Umschalt+Pfeil hoch/runter: gewählte Noten eine Oktave höher/tiefer (Key-Editor).
    OctaveUp,
    OctaveDown,
    /// F: Projektfenster und Editor folgen der Wiedergabe.
    ToggleFollow,
    PrevEvent,
    NextEvent,
    Mute,
    Solo,
    RecordArm,
    ToggleTransport,
    ToggleLeftZone,
    Quantize,
    Crossfade,
    /// Strg+K: Dialog „Wiederholen“ (Anzahl, geteilte Kopien).
    Repeat,
    NudgeLeft,
    NudgeRight,
    Shorter,
    Longer,
}

impl ProjectWindow {
    pub(crate) fn run_edit_command(&mut self, c: EditCommand) {
        match c {
            EditCommand::Copy if self.key_editor_active() => self.copy_notes(),
            EditCommand::Cut if self.key_editor_active() => {
                self.copy_notes();
                self.delete_selected_notes();
            }
            EditCommand::Paste if self.key_editor_active() => self.paste_notes(),
            EditCommand::PrevTrack if self.key_editor_active() => self.transpose_notes(1),
            EditCommand::NextTrack if self.key_editor_active() => self.transpose_notes(-1),
            EditCommand::OctaveUp if self.key_editor_active() => self.transpose_notes(12),
            EditCommand::OctaveDown if self.key_editor_active() => self.transpose_notes(-12),
            EditCommand::OctaveUp | EditCommand::OctaveDown => {}
            EditCommand::ToggleFollow => self.follow_playback = !self.follow_playback,
            EditCommand::Copy => self.copy_selection(),
            EditCommand::Cut => {
                self.copy_selection();
                self.delete_selection();
            }
            EditCommand::Paste => self.paste(),
            EditCommand::NudgeLeft => self.nudge(-1, false),
            EditCommand::NudgeRight => self.nudge(1, false),
            EditCommand::Shorter => self.nudge(-1, true),
            EditCommand::Longer => self.nudge(1, true),
            EditCommand::Duplicate => self.duplicate(),
            EditCommand::Repeat => {
                if !self.selection.is_empty() {
                    self.repeat_dialog.open = true;
                }
            }
            EditCommand::SplitAtCursor => self.split_at_cursor(),
            EditCommand::SelectAll => {
                self.selection = self
                    .project
                    .tracks
                    .iter()
                    .flat_map(|t| t.events.iter().map(|e| e.id))
                    .collect();
            }
            EditCommand::ZoomToProject => self.zoom_to_project(),
            EditCommand::PrevTrack => self.step_track(-1),
            EditCommand::NextTrack => self.step_track(1),
            EditCommand::PrevEvent => self.step_event(false),
            EditCommand::NextEvent => self.step_event(true),
            EditCommand::Mute | EditCommand::Solo | EditCommand::RecordArm => {
                let Some(track) = self
                    .selected_track
                    .and_then(|i| self.project.tracks.get_mut(i))
                else {
                    return;
                };
                match c {
                    EditCommand::Mute => track.mute = !track.mute,
                    EditCommand::Solo => track.solo = !track.solo,
                    _ if track.kind.holds_events() => track.record = !track.record,
                    _ => {}
                }
                self.dirty = true;
            }
            EditCommand::ToggleTransport => self.show_transport = !self.show_transport,
            EditCommand::ToggleLeftZone => self.show_inspector = !self.show_inspector,
            EditCommand::Quantize => self.quantize_selection(),
            EditCommand::Crossfade => {
                let before = self.project.clone();
                let ids = self.selection.clone();
                if !self.project.crossfade_selection(&ids) && !ids.is_empty() {
                    self.message = Some(
                        tr("Crossfade: die Events müssen sich auf einer Spur überlappen").into(),
                    );
                }
                self.checkpoint(before);
            }
        }
    }

    /// Gewählte Noten des Key-Editors in die Zwischenablage.
    fn copy_notes(&mut self) {
        let Some(ev) = self.key_editor.part.and_then(|p| self.project.event(p)) else {
            return;
        };
        let mut notes: Vec<Note> = self
            .key_editor
            .selection
            .iter()
            .filter_map(|&i| ev.notes.get(i).copied())
            .collect();
        let Some(first) = notes.iter().map(|n| n.start).min() else {
            return;
        };
        for n in &mut notes {
            n.start -= first;
        }
        self.clipboard.notes = notes;
    }

    /// Noten aus der Zwischenablage am Cursor in den offenen Part einfügen und wählen.
    fn paste_notes(&mut self) {
        let Some(part) = self.key_editor.part else {
            return;
        };
        if self.clipboard.notes.is_empty() {
            return;
        }
        let before = self.project.clone();
        let at = i64::try_from(self.cursor).unwrap_or(i64::MAX);
        let notes = self.clipboard.notes.clone();
        let Some(ev) = self.project.event_mut(part) else {
            return;
        };
        let rel = at.saturating_sub(i64::try_from(ev.start.0).unwrap_or(0));
        let first = ev.notes.len();
        ev.notes.extend(notes.into_iter().map(|n| Note {
            start: n.start.saturating_add(rel),
            ..n
        }));
        self.key_editor.selection = (first..first + self.clipboard.notes.len()).collect();
        self.checkpoint(before);
    }

    /// Gewählte Noten um `semitones` transponieren (in 0–127 begrenzt).
    fn transpose_notes(&mut self, semitones: i32) {
        let Some(part) = self.key_editor.part else {
            return;
        };
        let before = self.project.clone();
        let sel = self.key_editor.selection.clone();
        if let Some(ev) = self.project.event_mut(part) {
            for i in sel {
                if let Some(n) = ev.notes.get_mut(i) {
                    n.pitch = (i32::from(n.pitch) + semitones).clamp(0, 127) as u8;
                }
            }
        }
        self.checkpoint(before);
    }

    /// Nudge: die Auswahl um einen Rasterschritt verschieben (`length`: ihre Länge ändern).
    /// Mit Fokus im Key-Editor die gewählten Noten, sonst die gewählten Events.
    fn nudge(&mut self, dir: i64, length: bool) {
        let step = i64::try_from(self.grid_ticks()).unwrap_or(1).max(1) * dir;
        let before = self.project.clone();
        if self.key_editor_active() {
            let sel = self.key_editor.selection.clone();
            if let Some(ev) = self.key_editor.part.and_then(|p| self.project.event_mut(p)) {
                for i in sel {
                    let Some(n) = ev.notes.get_mut(i) else {
                        continue;
                    };
                    if length {
                        let len = i64::try_from(n.length).unwrap_or(i64::MAX);
                        n.length = u64::try_from(len.saturating_add(step)).unwrap_or(0).max(1);
                    } else {
                        n.start = n.start.saturating_add(step);
                    }
                }
            }
        } else {
            for id in self.selection.clone() {
                let Some((track, _)) = self.project.find(id) else {
                    continue;
                };
                let Some(ev) = self.project.event(id).cloned() else {
                    continue;
                };
                let shift = |v: u64| {
                    u64::try_from(i64::try_from(v).unwrap_or(i64::MAX).saturating_add(step))
                        .unwrap_or(0)
                };
                if length {
                    let end = shift(ev.end()).max(ev.start.0 + 1);
                    self.project.trim_end(id, end);
                } else {
                    self.project.move_event(id, track, shift(ev.start.0), false);
                }
            }
        }
        self.checkpoint(before);
    }

    fn copy_selection(&mut self) {
        let mut events: Vec<(usize, Event)> = self
            .selection
            .iter()
            .filter_map(|&id| {
                let (t, i) = self.project.find(id)?;
                Some((t, self.project.tracks[t].events[i].clone()))
            })
            .collect();
        events.sort_by_key(|(t, e)| (e.start.0, *t));
        if !events.is_empty() {
            self.clipboard.events = events;
        }
    }

    /// Fügt die Zwischenablage am Cursor ein. Die oberste kopierte Spur landet auf der
    /// gewählten Spur, die anderen behalten ihren Abstand; passt eine Spur nicht, bleibt das
    /// Event auf seiner ursprünglichen Spur.
    fn paste(&mut self) {
        let Some(first) = self.clipboard.events.iter().map(|(_, e)| e.start.0).min() else {
            return;
        };
        let top = self
            .clipboard
            .events
            .iter()
            .map(|(t, _)| *t)
            .min()
            .unwrap_or(0);
        let target = self.selected_track.unwrap_or(top);
        let at = self.cursor;
        self.place_copies(first, at, top, target);
    }

    /// Strg+D: die Auswahl direkt hinter sich wiederholen.
    fn duplicate(&mut self) {
        self.copy_selection();
        let events = &self.clipboard.events;
        let (Some(first), Some(end)) = (
            events.iter().map(|(_, e)| e.start.0).min(),
            events.iter().map(|(_, e)| e.end()).max(),
        ) else {
            return;
        };
        let top = events.iter().map(|(t, _)| *t).min().unwrap_or(0);
        self.place_copies(first, end, top, top);
    }

    fn place_copies(&mut self, first: u64, at: u64, top: usize, target: usize) {
        let before = self.project.clone();
        let mut placed = Vec::new();
        for (track, ev) in self.clipboard.events.clone() {
            let mut ev = ev;
            ev.start = Position(at.saturating_add(ev.start.0 - first));
            let wanted = (track - top).saturating_add(target);
            let id = self
                .project
                .insert_event(wanted, ev.clone())
                .or_else(|| self.project.insert_event(track, ev));
            placed.extend(id);
        }
        if !placed.is_empty() {
            self.selection = placed;
        }
        self.checkpoint(before);
    }

    /// Alt+X: gewählte Events am Cursor teilen; ohne Auswahl alle Events der gewählten Spur
    /// unter dem Cursor.
    fn split_at_cursor(&mut self) {
        let at = self.cursor;
        let mut ids: Vec<EventId> = self.selection.clone();
        if ids.is_empty()
            && let Some(id) = self
                .selected_track
                .and_then(|t| self.project.event_at(t, at))
        {
            ids.push(id);
        }
        let before = self.project.clone();
        for id in ids {
            if let Some(right) = self.project.split(id, at) {
                self.selection.push(right);
            }
        }
        self.checkpoint(before);
    }

    /// Umschalt+F: das ganze Projekt in die Breite der Event-Anzeige zoomen.
    fn zoom_to_project(&mut self) {
        let end = self
            .project
            .tracks
            .iter()
            .flat_map(|t| t.events.iter().map(Event::end))
            .max()
            .unwrap_or(0)
            .max(self.bar_ticks());
        let width = self.lanes_rect.width();
        if !width.is_finite() || width <= 0.0 {
            return;
        }
        let quarters = end as f64 / crate::TPQ;
        let px = (f64::from(width) * 0.95 / quarters) as f32;
        self.view.px_per_quarter = px.clamp(1.0, 2000.0);
        self.view.scroll_ticks = 0.0;
    }

    fn step_track(&mut self, delta: isize) {
        let n = self.project.tracks.len();
        if n == 0 {
            return;
        }
        let next = match self.selected_track {
            Some(i) => i.saturating_add_signed(delta).min(n - 1),
            None => 0,
        };
        self.selected_track = Some(next);
    }

    /// Pfeil links/rechts: vorheriges/nächstes Event auf der gewählten Spur.
    fn step_event(&mut self, forward: bool) {
        let Some(track) = self.selected_track.and_then(|i| self.project.tracks.get(i)) else {
            return;
        };
        let current = self
            .selection
            .last()
            .and_then(|&id| track.events.iter().position(|e| e.id == id));
        let n = track.events.len();
        let next = match (current, forward) {
            (_, _) if n == 0 => return,
            (Some(i), true) => (i + 1).min(n - 1),
            (Some(i), false) => i.saturating_sub(1),
            (None, true) => 0,
            (None, false) => n - 1,
        };
        let id = track.events[next].id;
        self.selection = vec![id];
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use studiocraft_project::{Project, TrackKind};

    const BAR: u64 = 3840;

    fn window() -> (ProjectWindow, EventId, EventId) {
        let mut p = Project::default();
        p.add_track(TrackKind::Instrument, "A");
        p.add_track(TrackKind::Instrument, "B");
        let a = p.create_event(0, 0, BAR).expect("a");
        let b = p.create_event(1, BAR, BAR).expect("b");
        let mut w = ProjectWindow::new(p, None, String::new());
        w.selected_track = Some(0);
        w.selection.clear();
        (w, a, b)
    }

    fn starts(w: &ProjectWindow, track: usize) -> Vec<u64> {
        w.project.tracks[track]
            .events
            .iter()
            .map(|e| e.start.0)
            .collect()
    }

    #[test]
    fn copy_paste_keeps_track_spacing_at_the_cursor() {
        let (mut w, a, b) = window();
        w.selection = vec![a, b];
        w.run_edit_command(EditCommand::Copy);
        w.cursor = 4 * BAR;
        w.run_edit_command(EditCommand::Paste);
        assert_eq!(starts(&w, 0), vec![0, 4 * BAR]);
        assert_eq!(starts(&w, 1), vec![BAR, 5 * BAR]);
        assert_eq!(w.selection.len(), 2);
        w.undo();
        assert_eq!(starts(&w, 0), vec![0]);
    }

    #[test]
    fn cut_removes_and_paste_brings_back() {
        let (mut w, a, _) = window();
        w.selection = vec![a];
        w.run_edit_command(EditCommand::Cut);
        assert!(w.project.tracks[0].events.is_empty());
        w.selected_track = Some(1);
        w.cursor = 2 * BAR;
        w.run_edit_command(EditCommand::Paste);
        assert_eq!(starts(&w, 1), vec![BAR, 2 * BAR]);
    }

    #[test]
    fn duplicate_places_the_copy_right_after_the_selection() {
        let (mut w, a, b) = window();
        w.selection = vec![a, b];
        w.run_edit_command(EditCommand::Duplicate);
        assert_eq!(starts(&w, 0), vec![0, 2 * BAR]);
        assert_eq!(starts(&w, 1), vec![BAR, 3 * BAR]);
    }

    #[test]
    fn split_at_cursor_and_select_all() {
        let (mut w, _, _) = window();
        w.cursor = BAR / 2;
        w.run_edit_command(EditCommand::SplitAtCursor);
        assert_eq!(starts(&w, 0), vec![0, BAR / 2]);
        w.run_edit_command(EditCommand::SelectAll);
        assert_eq!(w.selection.len(), 3);
    }

    #[test]
    fn arrows_walk_tracks_and_events_and_m_s_r_toggle() {
        let (mut w, a, b) = window();
        w.run_edit_command(EditCommand::NextTrack);
        w.run_edit_command(EditCommand::NextTrack);
        assert_eq!(w.selected_track, Some(1));
        w.run_edit_command(EditCommand::NextEvent);
        assert_eq!(w.selection, vec![b]);
        w.run_edit_command(EditCommand::PrevTrack);
        w.run_edit_command(EditCommand::PrevEvent);
        assert_eq!(w.selection, vec![a]);
        for c in [EditCommand::Mute, EditCommand::Solo, EditCommand::RecordArm] {
            w.run_edit_command(c);
        }
        let t = &w.project.tracks[0];
        assert!(t.mute && t.solo && t.record);
    }

    #[test]
    fn paste_with_empty_clipboard_does_nothing() {
        let (mut w, _, _) = window();
        w.run_edit_command(EditCommand::Paste);
        assert_eq!(w.project.tracks[0].events.len(), 1);
        assert!(w.undo.is_empty());
    }
}
