//! Tempospur in der Oberfläche (Issue #50): anlegen, ein- und ausschalten, Punkte im Inspector.

use egui::Ui;
use studiocraft_project::{BPM_RANGE, Position, TrackKind};

use crate::ProjectWindow;

impl ProjectWindow {
    /// Index der Tempospur, falls es eine gibt.
    pub(crate) fn tempo_track(&self) -> Option<usize> {
        self.project
            .tracks
            .iter()
            .position(|t| t.kind == TrackKind::Tempo)
    }

    /// Tempospur anlegen (eine je Projekt), einschalten und wählen; ein Undo-Schritt.
    pub(crate) fn add_tempo_track(&mut self) -> usize {
        let before = self.project.clone();
        let t = match self.tempo_track() {
            Some(t) => t,
            None => self.project.add_track(TrackKind::Tempo, "Tempo"),
        };
        let mut map = self.project.tempo.clone();
        map.use_track = true;
        self.project.set_tempo_map(map);
        self.selected_track = Some(t);
        self.checkpoint(before);
        t
    }

    /// Zwischen festem Tempo und Tempospur umschalten (die Punkte bleiben erhalten).
    pub(crate) fn toggle_tempo_track(&mut self) {
        if !self.project.tempo.use_track && self.tempo_track().is_none() {
            self.add_tempo_track();
            return;
        }
        let before = self.project.clone();
        let mut map = self.project.tempo.clone();
        map.use_track = !map.use_track;
        self.project.set_tempo_map(map);
        self.checkpoint(before);
    }

    /// Inspector der Tempospur: an/aus, Punkte mit Tempo und Rampe, Punkt am Cursor.
    pub(crate) fn tempo_track_ui(&mut self, ui: &mut Ui) {
        let mut map = self.project.tempo.clone();
        let mut changed = false;
        changed |= ui
            .checkbox(&mut map.use_track, "Tempospur aktiv")
            .on_hover_text("Aus: festes Tempo aus der Transportleiste")
            .changed();
        let mut remove = None;
        egui::Grid::new("tempo-punkte")
            .num_columns(4)
            .striped(true)
            .show(ui, |ui| {
                ui.label("1.1.1.000");
                let r = ui.add(
                    egui::DragValue::new(&mut map.bpm)
                        .range(BPM_RANGE.0..=BPM_RANGE.1)
                        .speed(0.2)
                        .fixed_decimals(1),
                );
                r.widget_info(|| {
                    egui::WidgetInfo::labeled(egui::Role::SpinButton, true, "Anfangstempo")
                });
                changed |= r.changed();
                ui.label("");
                ui.label("");
                ui.end_row();
                let tempo = self.project.tempo.clone();
                for (i, e) in map.events.iter_mut().enumerate() {
                    ui.label(tempo.format_position(Position(e.tick)));
                    let label = format!("Tempo bei {}", tempo.format_position(Position(e.tick)));
                    let r = ui.add(
                        egui::DragValue::new(&mut e.bpm)
                            .range(BPM_RANGE.0..=BPM_RANGE.1)
                            .speed(0.2)
                            .fixed_decimals(1),
                    );
                    r.widget_info(|| {
                        egui::WidgetInfo::labeled(egui::Role::SpinButton, true, label.as_str())
                    });
                    changed |= r.changed();
                    changed |= ui
                        .checkbox(&mut e.ramp, "Rampe")
                        .on_hover_text("Tempo gleitet vom vorigen Punkt hierher")
                        .changed();
                    if ui.small_button("Löschen").clicked() {
                        remove = Some(i);
                    }
                    ui.end_row();
                }
            });
        if let Some(i) = remove {
            map.events.remove(i);
            changed = true;
        }
        if ui.button("Punkt am Cursor").clicked() && self.cursor > 0 {
            let bpm = map.bpm_at(self.cursor);
            map.set_event(self.cursor, bpm, false);
            map.use_track = true;
            changed = true;
        }
        if changed {
            // Vom Stand vor der Eingabe aus umrechnen (kein Rundungsfehler beim Ziehen) …
            let base = self
                .tempo_before
                .get_or_insert_with(|| self.project.clone())
                .clone();
            let mut project = base;
            project.set_tempo_map(map);
            self.project = project;
        }
        // … und ein Undo-Schritt, wenn nichts mehr gezogen oder getippt wird.
        let editing = ui.ctx().dragged_id().is_some() || ui.memory(|m| m.focused().is_some());
        if !editing && let Some(before) = self.tempo_track_before.take() {
            self.checkpoint(before);
        }
    }
}
