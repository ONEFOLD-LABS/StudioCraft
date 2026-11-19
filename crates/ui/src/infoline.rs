//! Info-Zeile unter der Werkzeugzeile (docs/ui-parity.md, „Info-Zeile“; Spec „Editoren“,
//! gemeinsames Verhalten): zeigt die Werte des gewählten Events und lässt sie direkt ändern.
//! Eine Eingabe gilt beim Verlassen des Felds oder mit Enter; Escape verwirft sie.
//!
//! Mehrfachauswahl: Die Felder zeigen das früheste gewählte Event, die Beschriftungen sind
//! gelb. Zeitwerte wirken relativ auf alle (Anfang einen Takt später: alle einen Takt später),
//! Name und Stumm gelten für alle gleich. Jede Änderung ist ein Rückgängig-Schritt.

use egui::{Key, RichText, TextEdit, Ui};
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

/// Die gewählten Events, die es gibt, und das früheste davon (Anzeige und Bezug).
fn selected(w: &ProjectWindow) -> Option<(Vec<EventId>, EventId)> {
    let ids: Vec<EventId> = w
        .selection
        .iter()
        .copied()
        .filter(|&id| w.project.event(id).is_some())
        .collect();
    let first = ids
        .iter()
        .copied()
        .min_by_key(|&id| w.project.event(id).map(|e| (e.start.0, id)))?;
    Some((ids, first))
}

pub(crate) fn show(w: &mut ProjectWindow, ui: &mut Ui) {
    ui.horizontal(|ui| {
        let Some((ids, id)) = selected(w) else {
            ui.weak("Keine Auswahl");
            return;
        };
        let Some(ev) = w.project.event(id) else {
            return;
        };
        let multi = ids.len() > 1;
        if multi {
            let t = crate::theme::tokens(ui.ctx());
            ui.label(RichText::new(format!("{} Events", ids.len())).color(t.solo))
                .on_hover_text("Werte des ersten Events; Zeitwerte ändern alle relativ");
        }
        let tempo = &w.project.tempo;
        let mut edits = Vec::new();
        if let Some(name) = field(ui, "Name", ev.name.clone(), 110.0, multi) {
            edits.push(Edit::Name(name));
        }
        let start = tempo.format_position(ev.start);
        if let Some(t) = field(ui, "Anfang", start, 80.0, multi) {
            edits.extend(tempo.parse_position(&t).map(|p| Edit::Start(p.0)));
        }
        let end = tempo.format_position(Position(ev.end()));
        if let Some(t) = field(ui, "Ende", end, 80.0, multi) {
            edits.extend(tempo.parse_position(&t).map(|p| Edit::End(p.0)));
        }
        if let Some(t) = field(ui, "Länge", tempo.format_length(ev.length), 80.0, multi) {
            edits.extend(tempo.parse_length(&t).map(Edit::Length));
        }
        if let Some(t) = field(ui, "Fade-In", tempo.format_length(ev.fade_in), 80.0, multi) {
            edits.extend(tempo.parse_length(&t).map(Edit::FadeIn));
        }
        if let Some(t) = field(
            ui,
            "Fade-Out",
            tempo.format_length(ev.fade_out),
            80.0,
            multi,
        ) {
            edits.extend(tempo.parse_length(&t).map(Edit::FadeOut));
        }
        let mut muted = ev.muted;
        if ui.checkbox(&mut muted, "Stumm").changed() {
            edits.push(Edit::Mute(muted));
        }
        for edit in edits {
            apply(w, &ids, id, edit);
        }
    });
}

/// Beschriftetes Textfeld. Gibt den neuen Text zurück, sobald die Eingabe abgeschlossen ist.
/// `multi`: Mehrfachauswahl, die Beschriftung wird gelb.
pub(crate) fn field(
    ui: &mut Ui,
    label: &str,
    value: String,
    width: f32,
    multi: bool,
) -> Option<String> {
    let id = ui.make_persistent_id(("infozeile", label));
    let mut text = ui
        .data(|d| d.get_temp::<String>(id))
        .unwrap_or_else(|| value.clone());
    let l = if multi {
        let t = crate::theme::tokens(ui.ctx());
        ui.label(RichText::new(label).color(t.solo))
    } else {
        ui.weak(label)
    };
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

/// Wendet `edit` an: auf das Bezugs-Event `reference` absolut, auf alle anderen in `ids`
/// relativ (gleicher Abstand); Name und Stumm für alle gleich. Ein Rückgängig-Schritt.
fn apply(w: &mut ProjectWindow, ids: &[EventId], reference: EventId, edit: Edit) {
    let Some(r) = w.project.event(reference).cloned() else {
        return;
    };
    let before = w.project.clone();
    let delta = |new: u64, old: u64| i128::from(new) - i128::from(old);
    let shift = |v: u64, d: i128| u64::try_from((i128::from(v) + d).max(0)).unwrap_or(u64::MAX);
    for &id in ids {
        let Some(ev) = w.project.event(id).cloned() else {
            continue;
        };
        let own = match &edit {
            Edit::Name(_) | Edit::Mute(_) => edit.clone(),
            Edit::Start(t) => Edit::Start(shift(ev.start.0, delta(*t, r.start.0))),
            Edit::End(t) => Edit::End(shift(ev.end(), delta(*t, r.end()))),
            Edit::Length(l) => Edit::Length(shift(ev.length, delta(*l, r.length)).max(1)),
            Edit::FadeIn(f) => Edit::FadeIn(shift(ev.fade_in, delta(*f, r.fade_in))),
            Edit::FadeOut(f) => Edit::FadeOut(shift(ev.fade_out, delta(*f, r.fade_out))),
        };
        apply_one(&mut w.project, id, own);
    }
    w.checkpoint(before);
}

/// Wendet einen absoluten Wert auf ein Event an.
fn apply_one(p: &mut studiocraft_project::Project, id: EventId, edit: Edit) {
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
        apply(&mut w, &[id], id, Edit::Start(7680));
        assert_eq!(
            w.project.event(id).map(|e| (e.start.0, e.length)),
            Some((7680, 3840))
        );
        apply(&mut w, &[id], id, Edit::Length(960));
        assert_eq!(w.project.event(id).map(|e| e.length), Some(960));
        apply(&mut w, &[id], id, Edit::End(7680 + 1920));
        assert_eq!(w.project.event(id).map(|e| e.length), Some(1920));
        apply(&mut w, &[id], id, Edit::FadeOut(480));
        apply(&mut w, &[id], id, Edit::FadeIn(1800));
        let ev = w.project.event(id).cloned().expect("event");
        assert_eq!((ev.fade_in, ev.fade_out), (1800, 120));
        apply(&mut w, &[id], id, Edit::Mute(true));
        apply(&mut w, &[id], id, Edit::Name("Refrain".into()));
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
    fn several_events_change_relative_to_the_first() {
        let (mut w, a) = window();
        let b = w.project.create_event(0, 9600, 1920).expect("b");
        w.selection = vec![b, a];
        let (ids, first) = selected(&w).expect("selection");
        assert_eq!(first, a, "the earliest event is the reference");
        let span = |w: &ProjectWindow, id| w.project.event(id).map(|e| (e.start.0, e.length));
        // Start of the first one bar later: both move one bar.
        apply(&mut w, &ids, first, Edit::Start(3840 + 3840));
        assert_eq!(span(&w, a), Some((7680, 3840)));
        assert_eq!(span(&w, b), Some((13440, 1920)));
        // Length a beat shorter: both a beat shorter.
        apply(&mut w, &ids, first, Edit::Length(3840 - 960));
        assert_eq!(span(&w, a), Some((7680, 2880)));
        assert_eq!(span(&w, b), Some((13440, 960)));
        // Much shorter still keeps every event at least one tick long.
        apply(&mut w, &ids, first, Edit::Length(1));
        assert_eq!(span(&w, b), Some((13440, 1)));
        apply(&mut w, &ids, first, Edit::Mute(true));
        apply(&mut w, &ids, first, Edit::Name("Chor".into()));
        for id in [a, b] {
            let ev = w.project.event(id).expect("event");
            assert!(ev.muted);
            assert_eq!(ev.name, "Chor");
        }
        // Each change was one undo step for all events.
        for _ in 0..5 {
            w.undo();
        }
        assert_eq!(span(&w, a), Some((3840, 3840)));
        assert_eq!(span(&w, b), Some((9600, 1920)));
    }

    #[test]
    fn moving_earlier_than_zero_stops_at_the_start() {
        let (mut w, a) = window();
        let b = w.project.create_event(0, 960, 960).expect("b");
        w.selection = vec![a, b];
        let (ids, first) = selected(&w).expect("selection");
        assert_eq!(first, b);
        // b to 0 is 960 earlier; a follows by the same amount.
        apply(&mut w, &ids, first, Edit::Start(0));
        assert_eq!(w.project.event(a).map(|e| e.start.0), Some(2880));
        assert_eq!(w.project.event(b).map(|e| e.start.0), Some(0));
    }

    #[test]
    fn end_before_start_keeps_a_minimal_event() {
        let (mut w, id) = window();
        apply(&mut w, &[id], id, Edit::End(0));
        assert_eq!(w.project.event(id).map(|e| e.length), Some(1));
    }
}
