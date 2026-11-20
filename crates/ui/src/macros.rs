//! Makros: mehrere Befehle der zentralen Befehlsliste nacheinander (Spec „Tastaturkonzept“,
//! Issue #1). Bearbeitet werden sie im Dialog „Tastaturbefehle“, ausgelöst über ihre Taste oder
//! das Menü Bearbeiten → Makros.

use crate::i18n::tr;
use crate::keymap::{COMMANDS, Cmd, MACROS, MAX_NAME, MAX_STEPS, info};

/// Kurzer Name eines Befehls für die Schrittliste.
fn step_name(cmd: Cmd) -> &'static str {
    info(cmd).map_or("?", |i| i.name)
}

impl crate::ProjectWindow {
    /// Abschnitt „Makros“ im Dialog „Tastaturbefehle“.
    pub(crate) fn macros_ui(&mut self, ui: &mut egui::Ui) {
        let mut changed = false;
        egui::CollapsingHeader::new(tr("Makros"))
            .id_salt("makros")
            .show(ui, |ui| {
                for slot in 1..=MACROS as u8 {
                    changed |= self.macro_row(ui, slot);
                }
            });
        if changed {
            self.keymap_changed();
        }
    }

    /// Eine Zeile: Name, Schritte, Befehl hinzufügen, letzten Schritt entfernen.
    fn macro_row(&mut self, ui: &mut egui::Ui, slot: u8) -> bool {
        let Some(m) = self.keymap.macro_slot_mut(slot) else {
            return false;
        };
        let mut changed = false;
        ui.horizontal(|ui| {
            ui.label(format!("{slot}"));
            let name = ui
                .add(
                    egui::TextEdit::singleline(&mut m.name)
                        .hint_text(tr("Name"))
                        .char_limit(MAX_NAME)
                        .desired_width(140.0),
                )
                .on_hover_text(crate::i18n::trf(
                    "Name von Makro {slot}",
                    &[("slot", &slot)],
                ));
            let label = crate::i18n::trf("Name von Makro {slot}", &[("slot", &slot)]);
            name.widget_info(|| {
                egui::WidgetInfo::labeled(egui::Role::TextInput, true, label.as_str())
            });
            changed |= name.changed();
            let mut add = None;
            ui.add_enabled_ui(m.steps.len() < MAX_STEPS, |ui| {
                let r = ui.menu_button(tr("+ Befehl"), |ui| {
                    let mut category = "";
                    for c in COMMANDS.iter().filter(|c| !matches!(c.cmd, Cmd::Macro(_))) {
                        if c.category == category {
                            continue;
                        }
                        category = c.category;
                        ui.menu_button(category, |ui| {
                            for c in COMMANDS.iter().filter(|e| e.category == c.category) {
                                if ui.button(c.name).clicked() {
                                    add = Some(c.cmd);
                                    ui.close();
                                }
                            }
                        });
                    }
                });
                let label = crate::i18n::trf("Befehl an Makro {slot} anhängen", &[("slot", &slot)]);
                r.response.on_hover_text(label.as_str()).widget_info(|| {
                    egui::WidgetInfo::labeled(egui::Role::Button, true, label.as_str())
                });
            });
            if let Some(c) = add {
                m.steps.push(c);
                changed = true;
            }
            if ui
                .add_enabled(
                    !m.steps.is_empty(),
                    egui::Button::new(tr("Entfernen")).small(),
                )
                .on_hover_text(crate::i18n::trf(
                    "Letzten Befehl aus Makro {slot} entfernen",
                    &[("slot", &slot)],
                ))
                .clicked()
            {
                m.steps.pop();
                changed = true;
            }
            let steps: Vec<&str> = m.steps.iter().map(|c| step_name(*c)).collect();
            let text = if steps.is_empty() {
                crate::i18n::tr("leer").to_owned()
            } else {
                steps.join(", ")
            };
            ui.label(egui::RichText::new(text).weak());
        });
        changed
    }

    /// Untermenü Bearbeiten → Makros: belegte Makros ausführen, Dialog öffnen.
    pub(crate) fn macros_menu(&mut self, ui: &mut egui::Ui) {
        ui.menu_button(tr("Makros"), |ui| {
            let mut any = false;
            for slot in 1..=MACROS as u8 {
                let filled = self
                    .keymap
                    .macro_slot(slot)
                    .is_some_and(|m| !m.steps.is_empty());
                if !filled {
                    continue;
                }
                any = true;
                let b = egui::Button::new(self.keymap.macro_label(slot))
                    .shortcut_text(self.key_label(ui.ctx(), Cmd::Macro(slot)));
                if ui.add(b).clicked() {
                    ui.close();
                    self.run_macro(ui, slot);
                }
            }
            if !any {
                ui.add_enabled(false, egui::Button::new(tr("Keine Makros")));
            }
            ui.separator();
            if ui.button(tr("Makros bearbeiten …")).clicked() {
                self.key_dialog.open = true;
                ui.close();
            }
        });
    }
}
