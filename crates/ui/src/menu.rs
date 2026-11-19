//! Menüleiste, Datei-Befehle mit Dialogen und Rückfrage bei ungesicherten Änderungen.

use std::path::PathBuf;

use egui::{Key, KeyboardShortcut, Modifiers, Ui, ViewportCommand};
use studiocraft_project::file::EXTENSION;

use crate::ProjectWindow;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
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

const CMD: Modifiers = Modifiers::COMMAND;
const SC_NEW: KeyboardShortcut = KeyboardShortcut::new(CMD, Key::N);
const SC_OPEN: KeyboardShortcut = KeyboardShortcut::new(CMD, Key::O);
const SC_SAVE: KeyboardShortcut = KeyboardShortcut::new(CMD, Key::S);
const SC_SAVE_AS: KeyboardShortcut = KeyboardShortcut::new(CMD.plus(Modifiers::SHIFT), Key::S);
const SC_VERSION: KeyboardShortcut = KeyboardShortcut::new(CMD.plus(Modifiers::ALT), Key::S);
const SC_UNDO: KeyboardShortcut = KeyboardShortcut::new(CMD, Key::Z);
const SC_REDO: KeyboardShortcut = KeyboardShortcut::new(CMD.plus(Modifiers::SHIFT), Key::Z);
const SC_TRACK: KeyboardShortcut = KeyboardShortcut::new(CMD, Key::T);

impl ProjectWindow {
    /// Tastenkürzel der Datei-Befehle (Spec „Tastaturkonzept“).
    pub(crate) fn file_shortcuts(&mut self, ui: &Ui) {
        // Die längeren Kombinationen zuerst, sonst schluckt Strg+S die anderen.
        let hit = ui.input_mut(|i| {
            [
                (SC_SAVE_AS, Action::SaveAs),
                (SC_VERSION, Action::SaveVersion),
                (SC_SAVE, Action::Save),
                (SC_NEW, Action::New),
                (SC_OPEN, Action::Open),
            ]
            .into_iter()
            .find(|(sc, _)| i.consume_shortcut(sc))
            .map(|(_, a)| a)
        });
        if ui.input_mut(|i| i.consume_shortcut(&SC_TRACK)) {
            self.add_audio_track();
        }
        if let Some(a) = hit {
            self.request(ui, a);
        }
    }

    pub(crate) fn menu_bar(&mut self, ui: &mut Ui) {
        egui::MenuBar::new().ui(ui, |ui| {
            ui.menu_button("Datei", |ui| {
                let fmt = |sc: &KeyboardShortcut| ui.ctx().format_shortcut(sc);
                let items = [
                    ("Neu", Some(fmt(&SC_NEW)), Action::New),
                    ("Öffnen …", Some(fmt(&SC_OPEN)), Action::Open),
                    ("Speichern", Some(fmt(&SC_SAVE)), Action::Save),
                    ("Speichern unter …", Some(fmt(&SC_SAVE_AS)), Action::SaveAs),
                    (
                        "Neue Version speichern",
                        Some(fmt(&SC_VERSION)),
                        Action::SaveVersion,
                    ),
                    ("Audiodatei importieren …", None, Action::ImportAudio),
                    ("Audio-Mixdown exportieren …", None, Action::ExportMixdown),
                    ("Beenden", None, Action::Quit),
                ];
                for (i, (label, sc, action)) in items.into_iter().enumerate() {
                    if i == 5 || i == 7 {
                        ui.separator();
                    }
                    let mut b = egui::Button::new(label);
                    if let Some(sc) = sc {
                        b = b.shortcut_text(sc);
                    }
                    if ui.add(b).clicked() {
                        self.request(ui, action);
                    }
                }
            });
            ui.menu_button("Bearbeiten", |ui| {
                let undo = egui::Button::new("Rückgängig")
                    .shortcut_text(ui.ctx().format_shortcut(&SC_UNDO));
                if ui.add_enabled(!self.undo.is_empty(), undo).clicked() {
                    self.undo();
                }
                let redo = egui::Button::new("Wiederherstellen")
                    .shortcut_text(ui.ctx().format_shortcut(&SC_REDO));
                if ui.add_enabled(!self.redo.is_empty(), redo).clicked() {
                    self.redo();
                }
                ui.separator();
                let del = egui::Button::new("Löschen").shortcut_text("Entf");
                if ui.add_enabled(!self.selection.is_empty(), del).clicked() {
                    self.delete_selection();
                }
            });
            ui.menu_button("Projekt", |ui| {
                let audio = egui::Button::new("Audio-Spur hinzufügen")
                    .shortcut_text(ui.ctx().format_shortcut(&SC_TRACK));
                if ui.add(audio).clicked() {
                    self.add_audio_track();
                }
                if ui.button("Gruppen-Spur hinzufügen").clicked() {
                    self.add_bus_track(studiocraft_project::TrackKind::Group);
                }
                if ui.button("FX-Kanal hinzufügen").clicked() {
                    self.add_bus_track(studiocraft_project::TrackKind::Fx);
                }
                ui.menu_button("Instrument-Spur hinzufügen", |ui| {
                    if let Some(info) = self.instrument_menu(ui) {
                        self.add_instrument_track(info);
                    }
                });
            });
            ui.menu_button("Ansicht", |ui| {
                ui.checkbox(&mut self.show_inspector, "Inspector (Alt+I)");
                ui.checkbox(&mut self.show_info, "Info-Zeile");
                if ui
                    .selectable_label(
                        self.show_lower && self.lower == crate::LowerZone::Mixer,
                        "Mixer (F3)",
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

    /// „Speichern unter“: legt wie klassischen DAWs einen Projektordner mit dem Namen an.
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

    fn import_dialog(&mut self) {
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
