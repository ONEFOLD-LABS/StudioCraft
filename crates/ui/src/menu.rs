//! Menüleiste, Datei-Befehle mit Dialogen und Rückfrage bei ungesicherten Änderungen.

use std::path::PathBuf;

use egui::{Key, Ui, ViewportCommand};
use studiocraft_project::file::EXTENSION;

use crate::ProjectWindow;
use crate::keymap::Cmd;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub(crate) enum Action {
    New,
    Open,
    Save,
    SaveAs,
    SaveVersion,
    ImportAudio,
    ExportMixdown,
    Quit,
}

impl Action {
    /// Befehle, vor denen ungesicherte Änderungen verloren gehen könnten.
    fn discards_project(self) -> bool {
        matches!(self, Action::New | Action::Open | Action::Quit)
    }
}

impl ProjectWindow {
    pub(crate) fn menu_bar(&mut self, ui: &mut Ui) {
        egui::MenuBar::new().ui(ui, |ui| {
            ui.menu_button("Datei", |ui| {
                let items = [
                    ("Neu", Action::New),
                    ("Öffnen …", Action::Open),
                    ("Speichern", Action::Save),
                    ("Speichern unter …", Action::SaveAs),
                    ("Neue Version speichern", Action::SaveVersion),
                    ("Audiodatei importieren …", Action::ImportAudio),
                    ("Audio-Mixdown exportieren …", Action::ExportMixdown),
                    ("Beenden", Action::Quit),
                ];
                for (i, (label, action)) in items.into_iter().enumerate() {
                    if i == 5 {
                        ui.separator();
                    }
                    if i == 7 {
                        ui.separator();
                        self.autosave_menu(ui);
                        ui.separator();
                    }
                    let keys = self.key_label(ui.ctx(), Cmd::File(action));
                    let b = egui::Button::new(label).shortcut_text(keys);
                    if ui.add(b).clicked() {
                        self.request(ui, action);
                    }
                    if action == Action::ExportMixdown && ui.button("Stems exportieren …").clicked()
                    {
                        self.open_stem_dialog();
                        ui.close();
                    }
                }
            });
            ui.menu_button("Bearbeiten", |ui| {
                let undo = egui::Button::new("Rückgängig")
                    .shortcut_text(self.key_label(ui.ctx(), Cmd::Undo));
                if ui.add_enabled(!self.undo.is_empty(), undo).clicked() {
                    self.undo();
                }
                let redo = egui::Button::new("Wiederherstellen")
                    .shortcut_text(self.key_label(ui.ctx(), Cmd::Redo));
                if ui.add_enabled(!self.redo.is_empty(), redo).clicked() {
                    self.redo();
                }
                ui.separator();
                use crate::shortcuts::EditCommand as E;
                let has_sel = !self.selection.is_empty();
                let items = [
                    ("Ausschneiden", E::Cut, has_sel),
                    ("Kopieren", E::Copy, has_sel),
                    ("Einfügen", E::Paste, true),
                    ("Duplizieren", E::Duplicate, has_sel),
                    ("Wiederholen …", E::Repeat, has_sel),
                    ("Am Cursor teilen", E::SplitAtCursor, true),
                    ("Alles auswählen", E::SelectAll, true),
                    ("Quantisieren", E::Quantize, true),
                    ("Crossfade", E::Crossfade, has_sel),
                ];
                for (label, cmd, enabled) in items {
                    let keys = self.key_label(ui.ctx(), Cmd::Edit(cmd));
                    let b = egui::Button::new(label).shortcut_text(keys);
                    if ui.add_enabled(enabled, b).clicked() {
                        self.run_edit_command(cmd);
                        ui.close();
                    }
                }
                self.render_menu(ui);
                self.freeze_menu(ui);
                self.event_color_menu(ui);
                if ui.button("Quantisierungs-Panel …").clicked() {
                    self.show_quantize = true;
                    ui.close();
                }
                let del = egui::Button::new("Löschen")
                    .shortcut_text(self.key_label(ui.ctx(), Cmd::Delete));
                if ui.add_enabled(has_sel, del).clicked() {
                    self.delete_selection();
                }
                ui.separator();
                self.macros_menu(ui);
                if ui.button("Tastaturbefehle …").clicked() {
                    self.key_dialog.open = true;
                    ui.close();
                }
            });
            ui.menu_button("Projekt", |ui| {
                let audio = egui::Button::new("Audio-Spur hinzufügen")
                    .shortcut_text(self.key_label(ui.ctx(), Cmd::AddAudioTrack));
                if ui.add(audio).clicked() {
                    self.add_audio_track();
                }
                if ui.button("Gruppen-Spur hinzufügen").clicked() {
                    self.add_bus_track(studiocraft_project::TrackKind::Group);
                }
                if ui.button("FX-Kanal hinzufügen").clicked() {
                    self.add_bus_track(studiocraft_project::TrackKind::Fx);
                }
                if ui
                    .button("Ordner-Spur hinzufügen")
                    .on_hover_text("Die gewählte Spur kommt gleich in den neuen Ordner")
                    .clicked()
                {
                    self.add_folder_track();
                }
                ui.menu_button("Inserts vor dem Fader", |ui| {
                    for n in 0..=8u8 {
                        let text = format!("Slots 1–{n} vor, {}–8 nach dem Fader", n + 1);
                        let text = match n {
                            0 => "alle nach dem Fader".to_owned(),
                            8 => "alle vor dem Fader".to_owned(),
                            _ => text,
                        };
                        if ui
                            .radio(self.project.pre_fader_inserts == n, text)
                            .clicked()
                        {
                            let before = self.project.clone();
                            self.project.pre_fader_inserts = n;
                            self.checkpoint(before);
                            ui.close();
                        }
                    }
                });
                if ui.button("Marker-Spur hinzufügen").clicked() {
                    let before = self.project.clone();
                    self.add_marker_track();
                    self.checkpoint(before);
                }
                if ui.button("Tempospur hinzufügen").clicked() {
                    ui.close();
                    self.add_tempo_track();
                }
                let insert = egui::Button::new("Marker einfügen")
                    .shortcut_text(self.key_label(ui.ctx(), Cmd::InsertMarker));
                if ui.add(insert).clicked() {
                    self.insert_marker();
                }
                let cycle = egui::Button::new("Cycle-Marker aus Locatoren")
                    .shortcut_text(self.key_label(ui.ctx(), Cmd::CycleMarkerFromLocators));
                if ui.add(cycle).clicked() {
                    self.cycle_marker_from_locators();
                }
                ui.menu_button("Instrument-Spur hinzufügen", |ui| {
                    if let Some(info) = self.instrument_menu(ui) {
                        self.add_instrument_track(info);
                    }
                });
            });
            ui.menu_button("Transport", |ui| self.transport_menu(ui));
            ui.menu_button("Ansicht", |ui| {
                ui.checkbox(&mut self.show_inspector, "Linke Zone (Alt+L)");
                ui.add_enabled(
                    self.show_inspector,
                    egui::Checkbox::new(&mut self.show_inspector_column, "Inspector (Alt+I)"),
                );
                ui.checkbox(&mut self.show_info, "Info-Zeile");
                ui.checkbox(&mut self.show_transport, "Transportleiste (F2)");
                ui.checkbox(&mut self.show_status, "Statuszeile");
                ui.checkbox(&mut self.show_right, "Rechte Zone (Alt+R)");
                ui.menu_button("Layouts", |ui| self.layouts_menu(ui));
                if ui
                    .selectable_label(self.windows.mixer, "Mixer-Fenster (F3)")
                    .clicked()
                {
                    self.toggle_mixer_window();
                    ui.close();
                }
                if ui.button("Editor im eigenen Fenster (Strg+E)").clicked() {
                    self.open_editor_window();
                    ui.close();
                }
                ui.separator();
                if ui
                    .selectable_label(
                        self.show_lower && self.lower == crate::LowerZone::Mixer,
                        "Mixer (untere Zone)",
                    )
                    .clicked()
                {
                    self.show_lower = true;
                    self.lower = crate::LowerZone::Mixer;
                }
                if ui
                    .selectable_label(
                        self.show_lower && self.lower == crate::LowerZone::Editor,
                        "Key-Editor",
                    )
                    .clicked()
                {
                    self.show_lower = true;
                    self.lower = crate::LowerZone::Editor;
                }
                ui.checkbox(&mut self.show_lower, "Untere Zone");
                ui.separator();
                ui.menu_button("Erscheinungsbild", |ui| {
                    for pref in [
                        egui::ThemePreference::Dark,
                        egui::ThemePreference::Light,
                        egui::ThemePreference::System,
                    ] {
                        if ui
                            .selectable_label(self.theme == pref, crate::theme::label(pref))
                            .clicked()
                        {
                            self.set_theme(ui.ctx(), pref);
                            ui.close();
                        }
                    }
                });
            });
        });
    }

    /// Führt einen Befehl aus; fragt vorher nach, wenn ungesicherte Änderungen verloren gingen.
    pub(crate) fn request(&mut self, ui: &Ui, action: Action) {
        if action.discards_project() && self.is_modified() {
            self.pending = Some(action);
        } else {
            self.run(ui, action);
        }
    }

    fn run(&mut self, ui: &Ui, action: Action) {
        let result = match action {
            Action::New => {
                self.new_project();
                Ok(())
            }
            Action::Open => match pick_project() {
                Some(p) => self.open(&p),
                None => Ok(()),
            },
            Action::Save => match self.save() {
                Ok(true) => Ok(()),
                Ok(false) => self.save_as_dialog().map(|_| ()),
                Err(e) => Err(e),
            },
            Action::SaveAs => self.save_as_dialog().map(|_| ()),
            Action::SaveVersion => match self.save_new_version() {
                Ok(true) => Ok(()),
                Ok(false) => self.save_as_dialog().map(|_| ()),
                Err(e) => Err(e),
            },
            Action::ImportAudio => {
                self.import_dialog();
                Ok(())
            }
            Action::ExportMixdown => self.export_dialog(),
            Action::Quit => {
                self.allow_close = true;
                ui.ctx().send_viewport_cmd(ViewportCommand::Close);
                Ok(())
            }
        };
        if let Err(e) = result {
            self.message = Some(e);
        }
    }

    /// „Speichern unter“: legt einen Projektordner mit dem Namen an.
    /// `Ok(false)`, wenn der Dialog abgebrochen wurde.
    fn save_as_dialog(&mut self) -> Result<bool, String> {
        let default = self
            .project_path()
            .and_then(|p| p.file_name())
            .map_or(format!("Unbenannt.{EXTENSION}"), |n| {
                n.to_string_lossy().into_owned()
            });
        let Some(chosen) = dialog()
            .add_filter("StudioCraft-Projekt", &[EXTENSION])
            .set_file_name(default)
            .save_file()
        else {
            return Ok(false);
        };
        self.save_as(&project_folder_path(chosen))?;
        Ok(true)
    }

    pub(crate) fn import_dialog(&mut self) {
        let Some(files) = dialog()
            .add_filter("Audiodateien", studiocraft_engine::AudioClip::extensions())
            .pick_files()
        else {
            return;
        };
        let before = self.project.clone();
        let mut errors = Vec::new();
        for f in files {
            if let Err(e) = self.import_audio(&f, None) {
                errors.push(e);
            }
        }
        self.message = (!errors.is_empty()).then(|| errors.join("; "));
        self.checkpoint(before);
    }

    fn export_dialog(&mut self) -> Result<(), String> {
        let name = self
            .project_path()
            .and_then(|p| p.file_stem())
            .map_or("Mixdown".into(), |n| n.to_string_lossy().into_owned());
        match dialog()
            .add_filter("WAV (24 Bit)", &["wav"])
            .add_filter("FLAC (24 Bit, verlustfrei komprimiert)", &["flac"])
            .add_filter("AIFF (24 Bit)", &["aif", "aiff"])
            .set_file_name(format!("{name}.wav"))
            .save_file()
        {
            Some(p) => self.export_mixdown(&p),
            None => Ok(()),
        }
    }

    /// Rückfrage „Änderungen speichern?“ und Schließen des Fensters abfangen.
    pub(crate) fn confirm_dialog(&mut self, ui: &mut Ui) {
        let close = ui.ctx().input(|i| i.viewport().close_requested());
        if close && self.is_modified() && !self.allow_close {
            ui.ctx().send_viewport_cmd(ViewportCommand::CancelClose);
            self.pending = Some(Action::Quit);
        }
        let Some(action) = self.pending else { return };
        let mut choice = None;
        egui::Modal::new(egui::Id::unique("ungesichert")).show(ui.ctx(), |ui| {
            ui.heading("Ungesicherte Änderungen");
            ui.label("Sollen die Änderungen am Projekt vorher gespeichert werden?");
            ui.add_space(8.0);
            ui.horizontal(|ui| {
                if ui.button("Speichern").clicked() {
                    choice = Some(true);
                }
                if ui.button("Nicht speichern").clicked() {
                    choice = Some(false);
                }
                if ui.button("Abbrechen").clicked() || ui.input(|i| i.key_pressed(Key::Escape)) {
                    self.pending = None;
                }
            });
        });
        match choice {
            Some(true) => {
                self.pending = None;
                let saved = match self.save() {
                    Ok(true) => Ok(true),
                    Ok(false) => self.save_as_dialog(),
                    Err(e) => Err(e),
                };
                match saved {
                    Ok(true) => self.run(ui, action),
                    Ok(false) => {}
                    Err(e) => self.message = Some(e),
                }
            }
            Some(false) => {
                self.pending = None;
                self.run(ui, action);
            }
            None => {}
        }
    }
}

fn dialog() -> rfd::FileDialog {
    let start = std::env::var_os("HOME")
        .map(PathBuf::from)
        .map(|h| h.join("Musik"))
        .filter(|p| p.is_dir());
    match start {
        Some(dir) => rfd::FileDialog::new().set_directory(dir),
        None => rfd::FileDialog::new(),
    }
}

fn pick_project() -> Option<PathBuf> {
    dialog()
        .add_filter("StudioCraft-Projekt", &[EXTENSION])
        .pick_file()
}

/// `…/Song.studiocraft` → `…/Song/Song.studiocraft`, außer der Ordner heißt schon so.
pub(crate) fn project_folder_path(chosen: PathBuf) -> PathBuf {
    let stem = chosen.file_stem().unwrap_or_default().to_owned();
    let parent = chosen.parent().map(PathBuf::from).unwrap_or_default();
    let file = PathBuf::from(&stem).with_extension(EXTENSION);
    if parent.file_name() == Some(stem.as_os_str()) {
        parent.join(file)
    } else {
        parent.join(&stem).join(file)
    }
}

impl ProjectWindow {
    /// Menü „Transport“: die wichtigsten Befehle und das Stopp-Verhalten (Spec).
    fn transport_menu(&mut self, ui: &mut Ui) {
        let item = |w: &Self, ui: &mut Ui, label: &str, cmd: Cmd| {
            ui.add(egui::Button::new(label).shortcut_text(w.key_label(ui.ctx(), cmd)))
                .clicked()
        };
        if item(self, ui, "Start/Stopp", Cmd::PlayStop) {
            self.toggle_play();
            ui.close();
        }
        if ui.button("Stopp").clicked() {
            self.stop();
            ui.close();
        }
        if item(self, ui, "Zum Anfang", Cmd::ToStart) {
            self.locate(0);
            ui.close();
        }
        if item(self, ui, "Cycle an/aus", Cmd::ToggleCycle) {
            self.toggle_cycle();
            ui.close();
        }
        if item(self, ui, "Nachträgliche Aufnahme", Cmd::RetrospectiveRecord) {
            self.retrospective_record();
            ui.close();
        }
        self.record_mode_menu(ui);
        if item(self, ui, "Metronom an/aus", Cmd::Metronome) {
            self.toggle_metronome();
            ui.close();
        }
        self.click_menu(ui);
        ui.separator();
        let mut changed = ui
            .checkbox(&mut self.return_on_stop, "Beim Stoppen zur Startposition")
            .changed();
        ui.menu_button("Zweimal Stopp springt zu", |ui| {
            use crate::settings::StopTarget;
            changed |= ui
                .radio_value(&mut self.stop_twice, StopTarget::Start, "Anfang")
                .changed();
            changed |= ui
                .radio_value(
                    &mut self.stop_twice,
                    StopTarget::LeftLocator,
                    "Linker Locator",
                )
                .changed();
        });
        if changed {
            self.save_settings();
        }
        ui.separator();
        let mut constrain = self.latency.constrain;
        if ui
            .checkbox(&mut constrain, "Latenzausgleich begrenzen")
            .on_hover_text(
                "Bei Aufnahme und Mithören: Plugins mit hoher Latenz auf diesen Spuren umgehen",
            )
            .changed()
        {
            self.latency.constrain = constrain;
            self.dirty = true;
        }
        let total = self.latency.sent.total;
        if total > 0 {
            ui.label(
                egui::RichText::new(format!("Ausgleich: {}", self.latency_text(total))).weak(),
            );
        }
        ui.separator();
        let io = egui::Button::new("Audio-Anschlüsse …")
            .shortcut_text(self.key_label(ui.ctx(), Cmd::AudioConnections));
        if ui.add(io).clicked() {
            self.io_dialog = true;
            ui.close();
        }
        if ui.button("Audio- und MIDI-Geräte …").clicked() {
            self.open_device_dialog();
            ui.close();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn save_as_creates_project_folder() {
        assert_eq!(
            project_folder_path(PathBuf::from("/m/Song.studiocraft")),
            PathBuf::from("/m/Song/Song.studiocraft")
        );
        assert_eq!(
            project_folder_path(PathBuf::from("/m/Song/Song.studiocraft")),
            PathBuf::from("/m/Song/Song.studiocraft")
        );
        assert_eq!(
            project_folder_path(PathBuf::from("/m/Song")),
            PathBuf::from("/m/Song/Song.studiocraft")
        );
    }
}
