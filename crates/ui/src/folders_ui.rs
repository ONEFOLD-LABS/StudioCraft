//! Ordnerspuren in der Oberfläche (Issue #10): anlegen (Projekt → Ordner-Spur hinzufügen,
//! mit der gewählten Spur darin) und im Inspector einer Spur ihren Ordner wählen. Auf- und
//! zuklappen über den Pfeil im Spurkopf der Ordnerspur.

use crate::i18n::tr;
use egui::Ui;
use studiocraft_project::TrackKind;

use crate::ProjectWindow;

impl ProjectWindow {
    /// Neue Ordnerspur; ist eine Spur gewählt, kommt sie gleich hinein (in deren bisherigen
    /// Ordner gelegt, damit sie an ihrer Stelle im Baum bleibt).
    pub(crate) fn add_folder_track(&mut self) {
        let before = self.project.clone();
        let n = self
            .project
            .tracks
            .iter()
            .filter(|t| t.kind == TrackKind::Folder)
            .count();
        let folder = self.project.add_track(
            TrackKind::Folder,
            format!("{} {:02}", crate::i18n::tr("Ordner"), n + 1),
        );
        if let Some(sel) = self.selected_track.filter(|&s| s != folder) {
            let outer = self.project.parent_of(sel);
            self.project.set_parent(folder, outer);
            self.project.set_parent(sel, Some(folder));
        }
        self.selected_track = Some(folder);
        self.checkpoint(before);
    }

    /// Inspector: Ordner der Spur `idx` wählen.
    pub(crate) fn folder_ui(&mut self, ui: &mut Ui, idx: usize) {
        let folders: Vec<(usize, String)> = self
            .project
            .tracks
            .iter()
            .enumerate()
            .filter(|(i, t)| {
                t.kind == TrackKind::Folder && *i != idx && !self.project.is_inside(*i, idx)
            })
            .map(|(i, t)| (i, t.name.clone()))
            .collect();
        if folders.is_empty() && self.project.parent_of(idx).is_none() {
            return;
        }
        let current = self.project.parent_of(idx);
        let current_name = current
            .and_then(|c| self.project.tracks.get(c))
            .map_or(tr("Kein Ordner").to_owned(), |t| t.name.clone());
        let mut pick: Option<Option<usize>> = None;
        ui.horizontal(|ui| {
            let l = ui.label(tr("Ordner"));
            egui::ComboBox::from_id_salt(("ordner", idx))
                .selected_text(current_name)
                .show_ui(ui, |ui| {
                    if ui
                        .selectable_label(current.is_none(), tr("Kein Ordner"))
                        .clicked()
                    {
                        pick = Some(None);
                    }
                    for (i, name) in &folders {
                        if ui.selectable_label(current == Some(*i), name).clicked() {
                            pick = Some(Some(*i));
                        }
                    }
                })
                .response
                .labelled_by(l.id);
        });
        if let Some(p) = pick {
            let before = self.project.clone();
            if self.project.set_parent(idx, p) {
                self.checkpoint(before);
            }
        }
    }
}
