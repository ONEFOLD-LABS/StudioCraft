//! Menüleiste, Datei-Befehle mit Dialogen und Rückfrage bei ungesicherten Änderungen.

use crate::file_dialog::FileDialog;
use crate::i18n::tr;
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
    ImportMidi,
    ImportInterchange,
    ExportMixdown,
    ExportMidi,
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
            ui.menu_button(tr("Datei"), |ui| {
                let items = [
                    (tr("Neu"), Action::New),
                    (tr("Öffnen …"), Action::Open),
                    (tr("Speichern"), Action::Save),
                    (tr("Speichern unter …"), Action::SaveAs),
                    (tr("Neue Version speichern"), Action::SaveVersion),
                    (tr("Audiodatei importieren …"), Action::ImportAudio),
                    (tr("MIDI-Datei importieren …"), Action::ImportMidi),
                    (tr("AAF/OMF importieren …"), Action::ImportInterchange),
                    (tr("Audio-Mixdown exportieren …"), Action::ExportMixdown),
                    (tr("MIDI-Datei exportieren …"), Action::ExportMidi),
                    (tr("Beenden"), Action::Quit),
                ];
                for (label, action) in items {
                    if action == Action::ImportAudio {
                        ui.separator();
                    }
                    if action == Action::Quit {
                        ui.separator();
                        self.autosave_menu(ui);
                        ui.separator();
                    }
                    let keys = self.key_label(ui.ctx(), Cmd::File(action));
                    let b = egui::Button::new(label).shortcut_text(keys);
                    if ui.add(b).clicked() {
                        self.request(ui, action);
                    }
                    if action == Action::ExportMixdown
                        && ui.button(tr("Stems exportieren …")).clicked()
                    {
                        self.open_stem_dialog();
                        ui.close();
                    }
                    if action == Action::ExportMixdown
                        && ui.button(tr("Noten exportieren …")).clicked()
                    {
                        self.open_score_export();
                        ui.close();
                    }
                }
            });
            ui.menu_button(tr("Bearbeiten"), |ui| {
                let undo = egui::Button::new(tr("Rückgängig"))
                    .shortcut_text(self.key_label(ui.ctx(), Cmd::Undo));
                if ui.add_enabled(!self.undo.is_empty(), undo).clicked() {
                    self.undo();
                }
                let redo = egui::Button::new(tr("Wiederherstellen"))
                    .shortcut_text(self.key_label(ui.ctx(), Cmd::Redo));
                if ui.add_enabled(!self.redo.is_empty(), redo).clicked() {
                    self.redo();
                }
                ui.separator();
                use crate::shortcuts::EditCommand as E;
                let has_sel = !self.selection.is_empty();
                let items = [
                    (tr("Ausschneiden"), E::Cut, has_sel),
                    (tr("Kopieren"), E::Copy, has_sel),
                    (tr("Einfügen"), E::Paste, true),
                    (tr("Duplizieren"), E::Duplicate, has_sel),
                    (tr("Wiederholen …"), E::Repeat, has_sel),
                    (tr("Am Cursor teilen"), E::SplitAtCursor, true),
                    (tr("Alles auswählen"), E::SelectAll, true),
                    (tr("Quantisieren"), E::Quantize, true),
                    (tr("Crossfade"), E::Crossfade, has_sel),
                ];
                for (label, cmd, enabled) in items {
                    let keys = self.key_label(ui.ctx(), Cmd::Edit(cmd));
                    let b = egui::Button::new(label).shortcut_text(keys);
                    if ui.add_enabled(enabled, b).clicked() {
                        self.run_edit_command(cmd);
                        ui.close();
                    }
                }
                self.audio_part_menu(ui);
                self.render_menu(ui);
                self.freeze_menu(ui);
                self.event_color_menu(ui);
                if ui.button(tr("Quantisierungs-Panel …")).clicked() {
                    self.show_quantize = true;
                    ui.close();
                }
                let del = egui::Button::new(tr("Löschen"))
                    .shortcut_text(self.key_label(ui.ctx(), Cmd::Delete));
                if ui.add_enabled(has_sel, del).clicked() {
                    self.delete_selection();
                }
                ui.separator();
                self.macros_menu(ui);
                if ui.button(tr("Tastaturbefehle …")).clicked() {
                    self.key_dialog.open = true;
                    ui.close();
                }
            });
            ui.menu_button(tr("Projekt"), |ui| {
                let audio = egui::Button::new(tr("Audio-Spur hinzufügen"))
                    .shortcut_text(self.key_label(ui.ctx(), Cmd::AddAudioTrack));
                if ui.add(audio).clicked() {
                    self.add_audio_track();
                }
                if ui.button(tr("Gruppen-Spur hinzufügen")).clicked() {
                    self.add_bus_track(studiocraft_project::TrackKind::Group);
                }
                if ui.button(tr("FX-Kanal hinzufügen")).clicked() {
                    self.add_bus_track(studiocraft_project::TrackKind::Fx);
                }
                if ui
                    .button(tr("Ordner-Spur hinzufügen"))
                    .on_hover_text(tr("Die gewählte Spur kommt gleich in den neuen Ordner"))
                    .clicked()
                {
                    self.add_folder_track();
                }
                if ui
                    .button(tr("Plugins neu prüfen"))
                    .on_hover_text(tr("Sperrliste leeren und alle Plugin-Dateien neu prüfen"))
                    .clicked()
                {
                    self.rescan_plugins();
                    ui.close();
                }
                ui.menu_button(tr("Inserts vor dem Fader"), |ui| {
                    for n in 0..=8u8 {
                        let text = match n {
                            0 => tr("alle nach dem Fader").to_owned(),
                            8 => tr("alle vor dem Fader").to_owned(),
                            _ => crate::i18n::trf(
                                "Slots 1–{n} vor, {m}–8 nach dem Fader",
                                &[("n", &n), ("m", &(n + 1))],
                            ),
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
                if ui.button(tr("Marker-Spur hinzufügen")).clicked() {
                    let before = self.project.clone();
                    self.add_marker_track();
                    self.checkpoint(before);
                }
                if ui.button(tr("Tempospur hinzufügen")).clicked() {
                    ui.close();
                    self.add_tempo_track();
                }
                if ui.button(tr("Lineal-Spur hinzufügen")).clicked() {
                    ui.close();
                    self.add_ruler_track();
                }
                let insert = egui::Button::new(tr("Marker einfügen"))
                    .shortcut_text(self.key_label(ui.ctx(), Cmd::InsertMarker));
                if ui.add(insert).clicked() {
                    self.insert_marker();
                }
                let cycle = egui::Button::new(tr("Cycle-Marker aus Locatoren"))
                    .shortcut_text(self.key_label(ui.ctx(), Cmd::CycleMarkerFromLocators));
                if ui.add(cycle).clicked() {
                    self.cycle_marker_from_locators();
                }
                ui.menu_button(tr("Instrument-Spur hinzufügen"), |ui| {
                    if let Some(info) = self.instrument_menu(ui) {
                        self.add_instrument_track(info);
                    }
                });
            });
            ui.menu_button(tr("Transport"), |ui| self.transport_menu(ui));
            ui.menu_button(tr("Ansicht"), |ui| {
                ui.checkbox(&mut self.show_inspector, tr("Linke Zone (Alt+L)"));
                ui.add_enabled(
                    self.show_inspector,
                    egui::Checkbox::new(&mut self.show_inspector_column, tr("Inspector (Alt+I)")),
                );
                ui.checkbox(&mut self.show_info, tr("Info-Zeile"));
                ui.checkbox(&mut self.show_transport, tr("Transportleiste (F2)"));
                ui.checkbox(&mut self.show_status, tr("Statuszeile"));
                ui.checkbox(&mut self.show_right, tr("Rechte Zone (Alt+R)"));
                if self.assistant_available {
                    ui.checkbox(&mut self.show_assistant, tr("KI-Assistent"));
                }
                ui.menu_button(tr("Layouts"), |ui| self.layouts_menu(ui));
                if ui
                    .selectable_label(self.windows.mixer, tr("Mixer-Fenster (F3)"))
                    .clicked()
                {
                    self.toggle_mixer_window();
                    ui.close();
                }
                if ui
                    .button(tr("Editor im eigenen Fenster (Strg+E)"))
                    .clicked()
                {
                    self.open_editor_window();
                    ui.close();
                }
                ui.separator();
                if ui
                    .selectable_label(
                        self.show_lower && self.lower == crate::LowerZone::Mixer,
                        tr("Mixer (untere Zone)"),
                    )
                    .clicked()
                {
                    self.show_lower = true;
                    self.lower = crate::LowerZone::Mixer;
                }
                if ui
                    .selectable_label(
                        self.show_lower && self.lower == crate::LowerZone::Editor,
                        tr("Key-Editor"),
                    )
                    .clicked()
                {
                    self.show_lower = true;
                    self.lower = crate::LowerZone::Editor;
                }
                if ui
                    .selectable_label(
                        self.show_lower && self.lower == crate::LowerZone::List,
                        tr("List-Editor"),
                    )
                    .clicked()
                {
                    self.show_lower = true;
                    self.lower = crate::LowerZone::List;
                }
                if ui
                    .selectable_label(
                        self.show_lower && self.lower == crate::LowerZone::Score,
                        tr("Noten-Editor"),
                    )
                    .clicked()
                {
                    self.show_lower = true;
                    self.lower = crate::LowerZone::Score;
                }
                if ui.button(tr("Noten-Editor im eigenen Fenster")).clicked() {
                    self.open_score_window();
                    ui.close();
                }
                ui.checkbox(&mut self.show_lower, tr("Untere Zone"));
                ui.separator();
                ui.menu_button(tr("Erscheinungsbild"), |ui| {
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
                ui.menu_button(tr("Sprache"), |ui| {
                    for lang in crate::i18n::Language::ALL {
                        if ui
                            .selectable_label(crate::i18n::language() == lang, lang.label())
                            .clicked()
                        {
                            crate::i18n::set_language(lang);
                            self.save_settings();
                            ui.close();
                        }
                    }
                });
            });
            ui.menu_button(tr("Hilfe"), |ui| {
                if ui.button(tr("Über StudioCraft …")).clicked() {
                    self.about = crate::about::About::Open;
                    ui.close();
                }
                crate::sponsor::menu_item(ui);
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
        // Im Browser: Datei-Auswahl und Downloads statt Dateisystem.
        #[cfg(target_arch = "wasm32")]
        if self.run_in_browser(ui.ctx(), action) {
            return;
        }
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
            Action::ImportMidi => {
                self.import_midi_dialog();
                Ok(())
            }
            Action::ImportInterchange => {
                // Im Browser wählt `browser.rs` die Datei.
                #[cfg(not(target_arch = "wasm32"))]
                self.import_interchange_dialog();
                Ok(())
            }
            Action::ExportMixdown => self.export_dialog(),
            Action::ExportMidi => self.export_midi_dialog(),
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
            .map_or(format!("{}.{EXTENSION}", tr("Unbenannt")), |n| {
                n.to_string_lossy().into_owned()
            });
        let Some(chosen) = dialog()
            .add_filter(tr("StudioCraft-Projekt"), &[EXTENSION])
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
            .add_filter(
                tr("Audiodateien"),
                studiocraft_engine::AudioClip::extensions(),
            )
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
            .add_filter(tr("WAV (24 Bit)"), &["wav"])
            .add_filter(tr("FLAC (24 Bit, verlustfrei komprimiert)"), &["flac"])
            .add_filter(tr("AIFF (24 Bit)"), &["aif", "aiff"])
            .set_file_name(format!("{name}.wav"))
            .save_file()
        {
            Some(p) => self.export_mixdown(&p).map(|_| ()),
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
            ui.heading(tr("Ungesicherte Änderungen"));
            ui.label(tr(
                "Sollen die Änderungen am Projekt vorher gespeichert werden?",
            ));
            ui.add_space(8.0);
            ui.horizontal(|ui| {
                if ui.button(tr("Speichern")).clicked() {
                    choice = Some(true);
                }
                if ui.button(tr("Nicht speichern")).clicked() {
                    choice = Some(false);
                }
                if ui.button(tr("Abbrechen")).clicked() || ui.input(|i| i.key_pressed(Key::Escape))
                {
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

pub(crate) fn dialog() -> FileDialog {
    let start = std::env::var_os("HOME")
        .map(PathBuf::from)
        .map(|h| h.join("Musik"))
        .filter(|p| p.is_dir());
    match start {
        Some(dir) => FileDialog::new().set_directory(dir),
        None => FileDialog::new(),
    }
}

fn pick_project() -> Option<PathBuf> {
    dialog()
        .add_filter(tr("StudioCraft-Projekt"), &[EXTENSION])
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
        if item(self, ui, tr("Start/Stopp"), Cmd::PlayStop) {
            self.toggle_play();
            ui.close();
        }
        if ui.button(tr("Stopp")).clicked() {
            self.stop();
            ui.close();
        }
        if item(self, ui, tr("Zum Anfang"), Cmd::ToStart) {
            self.locate(0);
            ui.close();
        }
        if item(self, ui, tr("Cycle an/aus"), Cmd::ToggleCycle) {
            self.toggle_cycle();
            ui.close();
        }
        if item(
            self,
            ui,
            tr("Nachträgliche Aufnahme"),
            Cmd::RetrospectiveRecord,
        ) {
            self.retrospective_record();
            ui.close();
        }
        self.record_mode_menu(ui);
        if item(self, ui, tr("Metronom an/aus"), Cmd::Metronome) {
            self.toggle_metronome();
            ui.close();
        }
        self.click_menu(ui);
        ui.separator();
        let mut changed = ui
            .checkbox(
                &mut self.return_on_stop,
                tr("Beim Stoppen zur Startposition"),
            )
            .changed();
        ui.menu_button(tr("Zweimal Stopp springt zu"), |ui| {
            use crate::settings::StopTarget;
            changed |= ui
                .radio_value(&mut self.stop_twice, StopTarget::Start, tr("Anfang"))
                .changed();
            changed |= ui
                .radio_value(
                    &mut self.stop_twice,
                    StopTarget::LeftLocator,
                    tr("Linker Locator"),
                )
                .changed();
        });
        if changed {
            self.save_settings();
        }
        ui.separator();
        let mut constrain = self.latency.constrain;
        if ui
            .checkbox(&mut constrain, tr("Latenzausgleich begrenzen"))
            .on_hover_text(tr(
                "Bei Aufnahme und Mithören: Plugins mit hoher Latenz auf diesen Spuren umgehen",
            ))
            .changed()
        {
            self.latency.constrain = constrain;
            self.dirty = true;
        }
        let total = self.latency.sent.total;
        if total > 0 {
            ui.label(
                egui::RichText::new(crate::i18n::trf(
                    "Ausgleich: {zeit}",
                    &[("zeit", &self.latency_text(total))],
                ))
                .weak(),
            );
        }
        ui.separator();
        let io = egui::Button::new(tr("Audio-Anschlüsse …"))
            .shortcut_text(self.key_label(ui.ctx(), Cmd::AudioConnections));
        if ui.add(io).clicked() {
            self.io_dialog = true;
            ui.close();
        }
        if ui.button(tr("Audio- und MIDI-Geräte …")).clicked() {
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
