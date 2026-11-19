//! Quantisierung in der Oberfläche (Spec „Raster, Snap und Bearbeitung“ → Quantisierung):
//! Quantisierungswert in der Werkzeugzeile, Taste Q und ein Panel für Swing und
//! Stärke. Q wirkt im Key-Editor auf die gewählten Noten (ohne Auswahl auf alle), sonst auf
//! die gewählten Events.

use egui::{Context, Ui};
use studiocraft_project::QuantizeValue;

use crate::icons::{Icon, icon_button};
use crate::{LowerZone, ProjectWindow};

/// Auswahl des Quantisierungswerts und Knopf für das Panel.
pub(crate) fn toolbar(w: &mut ProjectWindow, ui: &mut Ui) {
    value_combo(ui, "quantisierung", &mut w.quantize.value, 80.0);
    let t = crate::theme::tokens(ui.ctx());
    if icon_button(
        ui,
        Icon::QuantizePanel,
        w.show_quantize,
        Some(t.accent),
        "Quantisierungs-Panel",
    )
    .clicked()
    {
        w.show_quantize = !w.show_quantize;
    }
}

fn value_combo(ui: &mut Ui, id: &str, value: &mut QuantizeValue, width: f32) {
    egui::ComboBox::from_id_salt(id)
        .selected_text(value.short_label())
        .width(width)
        .show_ui(ui, |ui| {
            for v in QuantizeValue::all() {
                ui.selectable_value(value, v, v.label());
            }
        })
        .response
        .on_hover_text("Quantisierung");
}

/// Das Quantisierungs-Panel als eigenes Fenster.
pub(crate) fn panel(w: &mut ProjectWindow, ctx: &Context) {
    if !w.show_quantize {
        return;
    }
    let mut open = true;
    egui::Window::new("Quantisierung")
        .open(&mut open)
        .resizable(false)
        .collapsible(false)
        // Unter der Werkzeugzeile, damit Raster und Quantisierung sichtbar bleiben.
        .pivot(egui::Align2::CENTER_TOP)
        .default_pos(egui::pos2(ctx.content_rect().center().x, 110.0))
        .show(ctx, |ui| {
            egui::Grid::new("quantisierung-panel")
                .num_columns(2)
                .show(ui, |ui| {
                    let l = ui.label("Raster");
                    ui.horizontal(|ui| {
                        value_combo(ui, "quantisierung-panel-wert", &mut w.quantize.value, 120.0);
                    })
                    .response
                    .labelled_by(l.id);
                    ui.end_row();
                    let l = ui.label("Swing");
                    ui.add(egui::Slider::new(&mut w.quantize.swing, 0..=100).suffix(" %"))
                        .labelled_by(l.id);
                    ui.end_row();
                    let l = ui.label("Stärke");
                    ui.add(egui::Slider::new(&mut w.quantize.strength, 0..=100).suffix(" %"))
                        .labelled_by(l.id);
                    ui.end_row();
                });
            ui.separator();
            if ui.button("Quantisieren (Q)").clicked() {
                w.quantize_selection();
            }
        });
    if !open {
        w.show_quantize = false;
    }
}

impl ProjectWindow {
    /// Der Key-Editor hat den Fokus und ist sichtbar.
    pub(crate) fn key_editor_active(&self) -> bool {
        let in_lower = self.show_lower && self.lower == LowerZone::Editor;
        let in_window = self.windows.editor == Some(crate::windows::EditorWindow::Key);
        self.key_editor.focused && (in_lower || in_window)
    }

    /// Taste Q: gewählte Noten im Key-Editor (ohne Auswahl alle Noten des Parts),
    /// sonst die gewählten Events. Ein Rückgängig-Schritt.
    pub(crate) fn quantize_selection(&mut self) {
        let before = self.project.clone();
        let q = self.quantize;
        if self.key_editor_active()
            && let Some(part) = self.key_editor.part
        {
            let sel = self.key_editor.selection.clone();
            let notes = (!sel.is_empty()).then_some(sel.as_slice());
            self.project.quantize_notes(part, notes, &q);
        } else {
            for id in self.selection.clone() {
                self.project.quantize_event(id, &q);
            }
        }
        if self.project != before {
            self.dirty = true;
        }
        self.checkpoint(before);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use studiocraft_project::{Note, Position, Project, TrackKind};

    fn window() -> (ProjectWindow, studiocraft_project::EventId) {
        let mut project = Project::default();
        let t = project.add_track(TrackKind::Instrument, "Synth");
        let id = project.create_event(t, 0, 3840).expect("part");
        let ev = project.event_mut(id).expect("part");
        for start in [30, 250] {
            ev.notes.push(Note {
                pitch: 60,
                velocity: 100,
                start,
                length: 100,
            });
        }
        let a = project.add_track(TrackKind::Audio, "Audio");
        project.create_event(a, 1000, 960).expect("event");
        (ProjectWindow::new(project, None, String::new()), id)
    }

    fn starts(w: &ProjectWindow, id: studiocraft_project::EventId) -> Vec<i64> {
        let ev = w.project.event(id).expect("part");
        ev.notes.iter().map(|n| n.start).collect()
    }

    #[test]
    fn q_in_the_key_editor_quantizes_the_selected_notes() {
        let (mut w, id) = window();
        w.key_editor.part = Some(id);
        w.key_editor.focused = true;
        w.show_lower = true;
        w.lower = LowerZone::Editor;
        w.key_editor.selection = vec![1];
        w.quantize_selection();
        assert_eq!(starts(&w, id), [30, 240]);
        w.key_editor.selection.clear();
        w.quantize_selection();
        assert_eq!(starts(&w, id), [0, 240]);
        w.undo();
        assert_eq!(starts(&w, id), [30, 240]);
    }

    #[test]
    fn q_in_the_project_window_quantizes_the_selected_events() {
        let (mut w, id) = window();
        let audio = w.project.tracks[1].events[0].id;
        w.selection = vec![id, audio];
        w.quantize.value = QuantizeValue {
            note: studiocraft_project::NoteValue::Quarter,
            feel: studiocraft_project::Feel::Straight,
        };
        w.quantize_selection();
        assert_eq!(starts(&w, id), [0, 0]);
        assert_eq!(w.project.event(audio).expect("event").start, Position(960));
        w.undo();
        assert_eq!(starts(&w, id), [30, 250]);
        assert_eq!(w.project.event(audio).expect("event").start, Position(1000));
    }

    #[test]
    fn use_quantize_grid_follows_the_value() {
        let (mut w, _) = window();
        w.grid = crate::tools::Grid::UseQuantize;
        assert_eq!(w.grid_ticks(), 240);
        w.quantize.value.feel = studiocraft_project::Feel::Triplet;
        assert_eq!(w.grid_ticks(), 160);
    }
}
