//! Dialog „Audio-Anschlüsse“ (F4, Issue #23): Ein- und Ausgangsbusse anlegen, benennen und
//! auf Hardware-Kanäle legen. Spuren wählen im Inspector Busse statt roher Kanalpaare.

use crate::i18n::tr;
use egui::Ui;
use studiocraft_project::{IoBus, MAIN_OUT};

use crate::ProjectWindow;

/// Eine Tabelle mit Bussen; gibt eine geänderte Liste und die zu entfernende Id zurück.
fn bus_table(
    ui: &mut Ui,
    id: &str,
    busses: &[IoBus],
    channels: u16,
    output: bool,
) -> (Vec<IoBus>, Option<u32>) {
    let mut list = busses.to_vec();
    let mut remove = None;
    egui::Grid::new(id)
        .num_columns(4)
        .striped(true)
        .show(ui, |ui| {
            ui.strong(tr("Name"));
            ui.strong(tr("Art"));
            ui.strong(tr("Kanäle"));
            ui.label("");
            ui.end_row();
            for b in &mut list {
                let r = ui.add(
                    egui::TextEdit::singleline(&mut b.name)
                        .desired_width(140.0)
                        .char_limit(64),
                );
                let name_label =
                    crate::i18n::trf("Name von {ports}", &[("ports", &b.ports_label())]);
                r.widget_info(|| {
                    egui::WidgetInfo::labeled(egui::Role::TextInput, true, name_label.as_str())
                });
                egui::ComboBox::from_id_salt((id, b.id, "art"))
                    .selected_text(if b.stereo { tr("Stereo") } else { tr("Mono") })
                    .width(70.0)
                    .show_ui(ui, |ui| {
                        ui.selectable_value(&mut b.stereo, true, tr("Stereo"));
                        ui.selectable_value(&mut b.stereo, false, tr("Mono"));
                    })
                    .response
                    .widget_info(|| {
                        egui::WidgetInfo::labeled(
                            egui::Role::ComboBox,
                            true,
                            crate::i18n::trf("Art von {name}", &[("name", &b.name)]),
                        )
                    });
                let width = if b.stereo { 2 } else { 1 };
                let max_first = channels.max(width).saturating_sub(width);
                egui::ComboBox::from_id_salt((id, b.id, "kanal"))
                    .selected_text(b.ports_label())
                    .width(60.0)
                    .show_ui(ui, |ui| {
                        let mut first = 0u16;
                        while first <= max_first {
                            let label = IoBus { first, ..b.clone() }.ports_label();
                            ui.selectable_value(&mut b.first, first, label);
                            first += width;
                        }
                    })
                    .response
                    .widget_info(|| {
                        egui::WidgetInfo::labeled(
                            egui::Role::ComboBox,
                            true,
                            crate::i18n::trf("Kanäle von {name}", &[("name", &b.name)]),
                        )
                    });
                let removable = !(output && b.id == MAIN_OUT);
                if ui
                    .add_enabled(removable, egui::Button::new(tr("Entfernen")).small())
                    .on_hover_text(if removable {
                        tr("Bus entfernen")
                    } else {
                        tr("Der Hauptmix bleibt")
                    })
                    .clicked()
                {
                    remove = Some(b.id);
                }
                ui.end_row();
            }
        });
    (list, remove)
}

impl ProjectWindow {
    /// Gerätekanäle des Ausgangs (von der App gemeldet).
    pub fn set_output_channels(&mut self, channels: u16) {
        self.output_channels = channels.max(1);
    }

    pub(crate) fn io_dialog(&mut self, ctx: &egui::Context) {
        if !self.io_dialog {
            return;
        }
        let mut open = true;
        let io = self.project.io.clone();
        let (in_ch, out_ch) = (self.input_channels(), self.output_channels);
        let mut new_io = io.clone();
        let mut removal: Option<(bool, u32)> = None;
        self.dialogs.escape_closes(
            ctx,
            crate::dialog::window_id(tr("Audio-Anschlüsse")),
            &mut open,
        );
        egui::Window::new(tr("Audio-Anschlüsse"))
            .open(&mut open)
            .resizable(false)
            .collapsible(false)
            .show(ctx, |ui| {
                ui.strong(tr("Eingänge"));
                let (list, rm) = bus_table(ui, "io-eingang", &io.inputs, in_ch, false);
                new_io.inputs = list;
                if let Some(id) = rm {
                    removal = Some((false, id));
                }
                ui.horizontal(|ui| {
                    if ui.button(tr("+ Mono-Eingang")).clicked() {
                        new_io.add(false, false);
                    }
                    if ui.button(tr("+ Stereo-Eingang")).clicked() {
                        new_io.add(false, true);
                    }
                });
                ui.separator();
                ui.strong(tr("Ausgänge"));
                let (list, rm) = bus_table(ui, "io-ausgang", &io.outputs, out_ch, true);
                new_io.outputs = list;
                if let Some(id) = rm {
                    removal = Some((true, id));
                }
                ui.horizontal(|ui| {
                    if ui.button(tr("+ Mono-Ausgang")).clicked() {
                        new_io.add(true, false);
                    }
                    if ui.button(tr("+ Stereo-Ausgang")).clicked() {
                        new_io.add(true, true);
                    }
                });
                ui.label(
                    egui::RichText::new(crate::i18n::trf(
                        "Gerät: {in_ch} Eingangs-, {out_ch} Ausgangskanäle",
                        &[("in_ch", &in_ch), ("out_ch", &out_ch)],
                    ))
                    .weak(),
                );
            });
        if new_io != io || removal.is_some() {
            // Tippen im Namen: ein Undo-Schritt je Feld, wenn der Fokus geht.
            let typing = ctx.memory(|m| m.focused().is_some());
            let before = self
                .io_before
                .get_or_insert_with(|| self.project.clone())
                .clone();
            self.project.io = new_io;
            if let Some((output, id)) = removal {
                self.project.remove_io_bus(output, id);
            }
            if !typing {
                self.io_before = None;
                self.checkpoint(before);
            }
        } else if !ctx.memory(|m| m.focused().is_some())
            && let Some(before) = self.io_before.take()
        {
            self.checkpoint(before);
        }
        if !open {
            self.io_dialog = false;
        }
    }
}
