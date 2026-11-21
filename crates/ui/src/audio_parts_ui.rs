//! Audio-Parts in der Oberfläche (Issue #184): gewählte Audio-Events zu einem Part
//! zusammenfassen und einen Part wieder auflösen (Menü Bearbeiten).

use crate::ProjectWindow;
use crate::i18n::tr;

impl ProjectWindow {
    /// Lassen sich die gewählten Events zu einem Audio-Part zusammenfassen?
    pub(crate) fn can_group_audio(&self) -> bool {
        let mut track = None;
        !self.selection.is_empty()
            && self.selection.iter().all(|&id| {
                let Some((t, i)) = self.project.find(id) else {
                    return false;
                };
                let tr = &self.project.tracks[t];
                let ev = &tr.events[i];
                tr.kind == studiocraft_project::TrackKind::Audio
                    && ev.clip.is_some()
                    && *track.get_or_insert(t) == t
            })
    }

    /// Gewählte Audio-Events einer Spur zu einem Part zusammenfassen (ein Rückgängig-Schritt).
    pub(crate) fn group_audio_selection(&mut self) {
        let before = self.project.clone();
        match self.project.group_audio(&self.selection.clone()) {
            Some(part) => {
                self.selection = vec![part];
                self.checkpoint(before);
            }
            None => {
                self.message = Some(
                    tr("Nur Audio-Events einer Audiospur lassen sich zu einem Part zusammenfassen")
                        .into(),
                );
            }
        }
    }

    /// Gewählte Audio-Parts wieder in ihre Events auflösen.
    pub(crate) fn dissolve_audio_parts(&mut self) {
        let before = self.project.clone();
        let mut new = Vec::new();
        for id in self.selection.clone() {
            new.extend(self.project.dissolve_part(id));
        }
        if !new.is_empty() {
            self.selection = new;
            self.checkpoint(before);
        }
    }

    /// Einträge im Menü Bearbeiten.
    pub(crate) fn audio_part_menu(&mut self, ui: &mut egui::Ui) {
        if ui
            .add_enabled(
                self.can_group_audio(),
                egui::Button::new(tr("Audio-Part bilden")),
            )
            .on_hover_text(tr(
                "Die gewählten Audio-Events einer Spur zu einem Part zusammenfassen",
            ))
            .clicked()
        {
            self.group_audio_selection();
            ui.close();
        }
        let any_part = self
            .selection
            .iter()
            .any(|&id| self.project.event(id).is_some_and(|e| e.is_audio_part()));
        if ui
            .add_enabled(any_part, egui::Button::new(tr("Audio-Part auflösen")))
            .clicked()
        {
            self.dissolve_audio_parts();
            ui.close();
        }
    }
}
