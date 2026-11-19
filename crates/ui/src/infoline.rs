//! Info-Zeile unter der Werkzeugzeile (docs/ui-parity.md, „Info-Zeile“; Spec „Editoren“,
//! gemeinsames Verhalten): zeigt die Werte des gewählten Events und lässt sie direkt ändern.
//! Eine Eingabe gilt beim Verlassen des Felds oder mit Enter; Escape verwirft sie.

use egui::{Key, TextEdit, Ui};
use studiocraft_project::{EventId, Position};

use crate::ProjectWindow;

/// Eine Änderung aus der Info-Zeile.
#[derive(Debug, Clone, PartialEq)]
enum Edit {
    Name(String),
    Start(u64),
    End(u64),
    Length(u64),
    FadeIn(u64),
    FadeOut(u64),
    Mute(bool),
}

pub(crate) fn show(w: &mut ProjectWindow, ui: &mut Ui) {
    ui.horizontal(|ui| {
        let id = match w.selection.as_slice() {
            [id] => *id,
            [] => {
                ui.weak("Keine Auswahl");
                return;
            }
            many => {
                ui.weak(format!("{} Events ausgewählt", many.len()));
                return;
            }
        };
        let Some(ev) = w.project.event(id) else {
            ui.weak("Keine Auswahl");
            return;
        };
        let tempo = &w.project.tempo;
        let mut edits = Vec::new();
        if let Some(name) = field(ui, "Name", ev.name.clone(), 110.0) {
            edits.push(Edit::Name(name));
        }
        let start = tempo.format_position(ev.start);
        if let Some(t) = field(ui, "Anfang", start, 80.0) {
            edits.extend(tempo.parse_position(&t).map(|p| Edit::Start(p.0)));
        }
        let end = tempo.format_position(Position(ev.end()));
        if let Some(t) = field(ui, "Ende", end, 80.0) {
            edits.extend(tempo.parse_position(&t).map(|p| Edit::End(p.0)));
        }
        if let Some(t) = field(ui, "Länge", tempo.format_length(ev.length), 80.0) {
            edits.extend(tempo.parse_length(&t).map(Edit::Length));
        }
        if let Some(t) = field(ui, "Fade-In", tempo.format_length(ev.fade_in), 80.0) {
            edits.extend(tempo.parse_length(&t).map(Edit::FadeIn));
        }
        if let Some(t) = field(ui, "Fade-Out", tempo.format_length(ev.fade_out), 80.0) {
            edits.extend(tempo.parse_length(&t).map(Edit::FadeOut));
        }
        let mut muted = ev.muted;
        if ui.checkbox(&mut muted, "Stumm").changed() {
            edits.push(Edit::Mute(muted));
        }
        for edit in edits {
            apply(w, id, edit);
        }
    });
}

/// Beschriftetes Textfeld. Gibt den neuen Text zurück, sobald die Eingabe abgeschlossen ist.
fn field(ui: &mut Ui, label: &str, value: String, width: f32) -> Option<String> {
    let id = ui.make_persistent_id(("infozeile", label));
    let mut text = ui
        .data(|d| d.get_temp::<String>(id))
        .unwrap_or_else(|| value.clone());
    let l = ui.weak(label);
    let r = ui
        .add(
            TextEdit::singleline(&mut text)
                .desired_width(width)
                .id(id.with("feld")),
        )
        .labelled_by(l.id);
    if r.has_focus() {
        ui.data_mut(|d| d.insert_temp(id, text));
        return None;
    }
    ui.data_mut(|d| d.remove_temp::<String>(id));
    let cancelled = ui.input(|i| i.key_pressed(Key::Escape));
    (r.lost_focus() && !cancelled && text != value).then_some(text)
}

fn apply(w: &mut ProjectWindow, id: EventId, edit: Edit) {
    let before = w.project.clone();
    let p = &mut w.project;
    match edit {
        Edit::Name(name) => {
            if let Some(ev) = p.event_mut(id) {
                ev.name = name;
            }
        }
        Edit::Start(start) => {
            if let Some((track, _)) = p.find(id) {
                p.move_event(id, track, start, false);
            }
        }
        Edit::End(end) => {
            p.trim_end(id, end);
        }
        Edit::Length(len) => {
            if let Some(start) = p.event(id).map(|e| e.start.0) {
                p.trim_end(id, start.saturating_add(len));
            }
        }
        Edit::FadeIn(fade) => {
            if let Some(out) = p.event(id).map(|e| e.fade_out) {
                // Ein längerer Fade-In kürzt den Fade-Out, nicht umgekehrt.
                let len = p.event(id).map_or(0, |e| e.length);
                p.set_fades(id, fade.min(len), out.min(len.saturating_sub(fade)));
            }
        }
        Edit::FadeOut(fade) => {
            if let Some(fade_in) = p.event(id).map(|e| e.fade_in) {
                p.set_fades(id, fade_in, fade);
            }
        }
        Edit::Mute(m) => {
            if let Some(ev) = p.event_mut(id) {
                ev.muted = m;
            }
        }
    }
    w.checkpoint(before);
}

#[cfg(test)]
mod tests {
    use super::*;

    fn window() -> (ProjectWindow, EventId) {
        let mut project = studiocraft_project::Project::default();
        project.add_track(studiocraft_project::TrackKind::Instrument, "Synth");
        let id = project.create_event(0, 3840, 3840).expect("event");
        let mut w = ProjectWindow::new(project, None, String::new());
        w.selection = vec![id];
        (w, id)
    }

    #[test]
    fn edits_move_trim_fade_and_mute_with_undo() {
        let (mut w, id) = window();
        apply(&mut w, id, Edit::Start(7680));
        assert_eq!(
            w.project.event(id).map(|e| (e.start.0, e.length)),
            Some((7680, 3840))
        );
        apply(&mut w, id, Edit::Length(960));
        assert_eq!(w.project.event(id).map(|e| e.length), Some(960));
        apply(&mut w, id, Edit::End(7680 + 1920));
        assert_eq!(w.project.event(id).map(|e| e.length), Some(1920));
        apply(&mut w, id, Edit::FadeOut(480));
        apply(&mut w, id, Edit::FadeIn(1800));
        let ev = w.project.event(id).cloned().expect("event");
        assert_eq!((ev.fade_in, ev.fade_out), (1800, 120));
        apply(&mut w, id, Edit::Mute(true));
        apply(&mut w, id, Edit::Name("Refrain".into()));
        let ev = w.project.event(id).cloned().expect("event");
        assert!(ev.muted);
        assert_eq!(ev.name, "Refrain");
        for _ in 0..7 {
            w.undo();
        }
        let ev = w.project.event(id).cloned().expect("event");
        assert_eq!((ev.start.0, ev.length, ev.muted), (3840, 3840, false));
    }

    #[test]
    fn end_before_start_keeps_a_minimal_event() {
        let (mut w, id) = window();
        apply(&mut w, id, Edit::End(0));
        assert_eq!(w.project.event(id).map(|e| e.length), Some(1));
    }
}
