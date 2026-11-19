//! Bearbeitungs- und Navigationskürzel aus der Standardbelegung (Spec „Tastaturkonzept“,
//! Issue #7): Ausschneiden/Kopieren/Einfügen, Duplizieren, am Cursor teilen, alles auswählen,
//! auf das Projekt zoomen, Spur und Event wechseln, M/S/R der gewählten Spur, F2, Alt+L und
//! Q (Quantisieren).
//!
//! Die Kürzel werden vor [`ProjectWindow::handle_keys`] verbraucht, damit dort keine
//! Doppelbelegung greift (z. B. Alt+X gegen X).

use egui::{Key, Modifiers, Ui};
use studiocraft_project::{Event, EventId, Position};

use crate::ProjectWindow;

/// Kopierte Events mit ihrer Spur, für Strg+V.
#[derive(Debug, Clone, Default)]
pub(crate) struct Clipboard {
    events: Vec<(usize, Event)>,
}

/// Ein Befehl aus der Standardbelegung.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
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
    PrevEvent,
    NextEvent,
    Mute,
    Solo,
    RecordArm,
    ToggleTransport,
    ToggleLeftZone,
    Quantize,
}

const BINDINGS: [(Modifiers, Key, EditCommand); 17] = [
    (Modifiers::COMMAND, Key::X, EditCommand::Cut),
    (Modifiers::COMMAND, Key::C, EditCommand::Copy),
    (Modifiers::COMMAND, Key::V, EditCommand::Paste),
    (Modifiers::COMMAND, Key::D, EditCommand::Duplicate),
    (Modifiers::COMMAND, Key::A, EditCommand::SelectAll),
    (Modifiers::ALT, Key::X, EditCommand::SplitAtCursor),
    (Modifiers::ALT, Key::L, EditCommand::ToggleLeftZone),
    (Modifiers::SHIFT, Key::F, EditCommand::ZoomToProject),
    (Modifiers::NONE, Key::ArrowUp, EditCommand::PrevTrack),
    (Modifiers::NONE, Key::ArrowDown, EditCommand::NextTrack),
    (Modifiers::NONE, Key::ArrowLeft, EditCommand::PrevEvent),
    (Modifiers::NONE, Key::ArrowRight, EditCommand::NextEvent),
    (Modifiers::NONE, Key::M, EditCommand::Mute),
    (Modifiers::NONE, Key::S, EditCommand::Solo),
    (Modifiers::NONE, Key::R, EditCommand::RecordArm),
    (Modifiers::NONE, Key::F2, EditCommand::ToggleTransport),
    (Modifiers::NONE, Key::Q, EditCommand::Quantize),
];

/// Wie `InputState::consume_key`, aber die Modifier müssen genau passen: `R` soll nicht auch
/// bei Alt+R auslösen.
fn consume_exact(i: &mut egui::InputState, m: Modifiers, key: Key) -> bool {
    let mut hit = false;
    i.events.retain(|e| {
        let is = matches!(e, egui::Event::Key { key: k, pressed: true, modifiers, .. }
            if *k == key && modifiers.matches_exact(m));
        hit |= is;
        !is
    });
    hit
}

impl ProjectWindow {
    /// Verbraucht die Kürzel dieses Moduls und führt sie aus.
    pub(crate) fn edit_shortcuts(&mut self, ui: &Ui) {
        if ui.ctx().egui_wants_keyboard_input() {
            return;
        }
        let hits: Vec<EditCommand> = ui.input_mut(|i| {
            BINDINGS
                .iter()
                .filter(|(m, k, _)| consume_exact(i, *m, *k))
                .map(|(_, _, c)| *c)
                .collect()
        });
        for c in hits {
            self.run_edit_command(c);
        }
    }

    pub(crate) fn run_edit_command(&mut self, c: EditCommand) {
        match c {
            EditCommand::Copy => self.copy_selection(),
            EditCommand::Cut => {
                self.copy_selection();
                self.delete_selection();
            }
            EditCommand::Paste => self.paste(),
            EditCommand::Duplicate => self.duplicate(),
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
        }
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
            self.clipboard = Clipboard { events };
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
