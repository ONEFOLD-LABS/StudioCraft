//! Benannte Fensterlayouts (Spec „Fenster-Architektur“): welche Zonen offen sind und wie
//! groß sie sind, unter einem Namen gespeichert und per Strg+Alt+1 bis 9 abrufbar. Die
//! Layouts liegen im Projekt und gehören nicht zum Undo-Verlauf.

use crate::i18n::tr;
use egui::Ui;
use studiocraft_project::NamedLayout;
use studiocraft_project::layout::{MAX_LAYOUTS, clip_name};

use crate::ProjectWindow;

impl ProjectWindow {
    /// Speichert das aktuelle Layout unter `name`. Gibt es den Namen schon, wird dieses
    /// Layout ersetzt; sonst kommt es in den nächsten freien Platz (höchstens 9).
    pub(crate) fn save_layout(&mut self, name: &str) {
        let name = clip_name(name.trim());
        if name.is_empty() {
            self.message = Some(tr("Bitte einen Namen für das Layout eingeben").into());
            return;
        }
        let layout = self.project.layout.clone();
        if let Some(l) = self.project.layouts.iter_mut().find(|l| l.name == name) {
            l.layout = layout;
        } else if self.project.layouts.len() < MAX_LAYOUTS {
            self.project.layouts.push(NamedLayout { name, layout });
        } else {
            self.message = Some(crate::i18n::trf(
                "Höchstens {max} Layouts; erst eins entfernen",
                &[("max", &MAX_LAYOUTS)],
            ));
        }
    }

    /// Ruft das Layout in Platz `slot` (1–9) ab.
    pub(crate) fn recall_layout(&mut self, slot: u8) {
        let layout = usize::from(slot)
            .checked_sub(1)
            .and_then(|i| self.project.layouts.get(i))
            .map(|l| l.layout.clone());
        match layout {
            Some(l) => self.apply_layout(l),
            None => {
                self.message = Some(crate::i18n::trf(
                    "Layout {slot} ist nicht belegt",
                    &[("slot", &slot)],
                ))
            }
        }
    }

    /// Untermenü „Layouts“ im Menü Ansicht: speichern, abrufen, entfernen.
    pub(crate) fn layouts_menu(&mut self, ui: &mut Ui) {
        ui.horizontal(|ui| {
            let l = ui.label(tr("Name"));
            ui.add(egui::TextEdit::singleline(&mut self.layout_name).desired_width(140.0))
                .labelled_by(l.id);
            if ui.button(tr("Aktuelles speichern")).clicked() {
                let name = std::mem::take(&mut self.layout_name);
                self.save_layout(&name);
            }
        });
        if self.project.layouts.is_empty() {
            ui.weak(tr("Noch keine Layouts gespeichert"));
            return;
        }
        ui.separator();
        let mut recall = None;
        let mut remove = None;
        for (i, l) in self.project.layouts.iter().enumerate() {
            ui.horizontal(|ui| {
                let slot = i + 1;
                if ui
                    .button(format!("{slot}  {}", l.name))
                    .on_hover_text(format!("Strg+Alt+{slot}"))
                    .clicked()
                {
                    recall = Some(slot);
                }
                if ui
                    .small_button(tr("x"))
                    .on_hover_text(crate::i18n::trf(
                        "Layout {slot} entfernen",
                        &[("slot", &slot)],
                    ))
                    .clicked()
                {
                    remove = Some(i);
                }
            });
        }
        if let Some(slot) = recall.and_then(|s| u8::try_from(s).ok()) {
            self.recall_layout(slot);
            ui.close();
        }
        if let Some(i) = remove {
            self.project.layouts.remove(i);
        }
    }
}
