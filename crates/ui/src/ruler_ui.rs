//! Lineal-Spuren (Spec „Spurtypen“ → Lineal, Issue #44): zusätzliche Zeitlineale mit eigenem
//! Anzeigeformat, beliebig viele. Gezeichnet werden sie in `timeline.rs`; hier stehen das
//! Anlegen und das Format im Inspector.

use egui::Ui;
use studiocraft_project::{RulerFormat, TrackKind};

use crate::ProjectWindow;
use crate::i18n::tr;

impl ProjectWindow {
    /// Legt eine Lineal-Spur an (Format: Sekunden), wählt sie (ein Rückgängig-Schritt).
    pub(crate) fn add_ruler_track(&mut self) -> usize {
        let before = self.project.clone();
        let t = self.project.add_track(TrackKind::Ruler, tr("Lineal"));
        self.project.tracks[t].ruler = RulerFormat::Seconds;
        self.selected_track = Some(t);
        self.checkpoint(before);
        t
    }

    /// Format der Lineal-Spur im Inspector.
    pub(crate) fn ruler_ui(&mut self, ui: &mut Ui, track: usize) {
        let Some(current) = self.project.tracks.get(track).map(|t| t.ruler) else {
            return;
        };
        let mut chosen = current;
        ui.horizontal(|ui| {
            let l = ui.label(tr("Format"));
            egui::ComboBox::from_id_salt(("lineal-format", track))
                .selected_text(tr(current.label()))
                .show_ui(ui, |ui| {
                    for f in RulerFormat::ALL {
                        ui.selectable_value(&mut chosen, f, tr(f.label()));
                    }
                })
                .response
                .labelled_by(l.id);
        });
        if chosen != current {
            let before = self.project.clone();
            self.project.tracks[track].ruler = chosen;
            self.checkpoint(before);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use studiocraft_project::Project;

    #[test]
    fn rulers_are_added_with_seconds_and_undone_in_one_step() {
        let mut w = ProjectWindow::new(Project::default(), None, "kein Audio (Test)".into());
        let t = w.add_ruler_track();
        assert_eq!(w.project.tracks[t].kind, TrackKind::Ruler);
        assert_eq!(w.project.tracks[t].ruler, RulerFormat::Seconds);
        assert_eq!(w.selected_track, Some(t));
        let second = w.add_ruler_track();
        assert_ne!(t, second, "beliebig viele Lineale");
        w.undo();
        w.undo();
        assert!(w.project.tracks.is_empty());
    }
}
