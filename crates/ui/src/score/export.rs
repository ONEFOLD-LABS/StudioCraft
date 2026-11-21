//! Dialog „Noten exportieren“ (Issue #183): die gewählten MIDI-Parts (sonst der aktive Part)
//! als MusicXML oder als druckfertiges PDF mit Papierformat, Rändern, Notengröße und Titel.

use std::path::{Path, PathBuf};

use studiocraft_project::EventId;

use super::musicxml::{self, PartScore};
use super::notation;
use super::pdf::{self, Paper};
use crate::ProjectWindow;
use crate::i18n::{tr, trf};

/// Zustand des Dialogs.
#[derive(Default)]
pub(crate) struct ExportDialog {
    pub open: bool,
    pub pdf: bool,
    pub layout: pdf::Layout,
    pub message: Option<String>,
}

impl ProjectWindow {
    /// Datei → „Noten exportieren …“ und der Knopf im Noten-Editor.
    pub(crate) fn open_score_export(&mut self) {
        let title = self
            .project_path()
            .and_then(Path::file_stem)
            .map(|s| s.to_string_lossy().into_owned())
            .unwrap_or_default();
        let d = &mut self.score_view.export;
        d.open = true;
        d.message = None;
        if d.layout.title.is_empty() {
            d.layout.title = title;
        }
    }

    /// Die zu exportierenden Parts: gewählte MIDI-Parts in Spur- und Zeitfolge, sonst der
    /// aktive Part des Editors.
    pub(crate) fn score_export_parts(&mut self) -> Vec<EventId> {
        let mut chosen: Vec<(usize, u64, EventId)> = self
            .selection
            .iter()
            .filter_map(|&id| {
                let (t, i) = self.project.find(id)?;
                let track = &self.project.tracks[t];
                let ev = &track.events[i];
                (track.kind != studiocraft_project::TrackKind::Audio && ev.clip.is_none())
                    .then_some((t, ev.start.0, id))
            })
            .collect();
        chosen.sort_unstable();
        if chosen.is_empty() {
            return self.list_part().into_iter().collect();
        }
        chosen.into_iter().map(|c| c.2).collect()
    }

    fn part_scores(&mut self) -> Vec<PartScore> {
        let key = self.project.key.sane();
        self.score_export_parts()
            .into_iter()
            .filter_map(|id| {
                let (t, _) = self.project.find(id)?;
                let ev = self.project.event(id)?;
                let track = &self.project.tracks[t].name;
                let name = if ev.name.is_empty() || ev.name == *track {
                    track.clone()
                } else {
                    format!("{track} – {}", ev.name)
                };
                Some(PartScore {
                    name,
                    score: notation::layout(&notation::Input {
                        notes: &ev.notes,
                        part_start: ev.start.0,
                        part_len: ev.length,
                        tempo: &self.project.tempo,
                        key,
                        quantize: studiocraft_project::score_quantize(ev.score_quantize),
                    }),
                })
            })
            .collect()
    }

    /// Schreibt die Noten nach `path` (MusicXML oder PDF).
    pub(crate) fn export_score_to(&mut self, path: &Path, as_pdf: bool) -> Result<(), String> {
        let parts = self.part_scores();
        if parts.is_empty() {
            return Err(tr("Erst einen MIDI-Part auswählen").into());
        }
        let layout = self.score_view.export.layout.clone();
        let bytes = if as_pdf {
            pdf::write(&parts, &layout)?
        } else {
            musicxml::write(&layout.title, &parts).into_bytes()
        };
        std::fs::write(path, bytes).map_err(|e| format!("{}: {e}", path.display()))
    }

    /// Das Fenster „Noten exportieren“.
    pub(crate) fn score_export_dialog(&mut self, ctx: &egui::Context) {
        if !self.score_view.export.open {
            return;
        }
        let count = self.score_export_parts().len();
        let mut open = true;
        let mut run = false;
        let d = &mut self.score_view.export;
        self.dialogs.show(
            ctx,
            crate::dialog::window_id(tr("Noten exportieren")),
            &mut open,
            crate::dialog::Kind::Modal,
        );
        egui::Window::new(tr("Noten exportieren"))
            .open(&mut open)
            .resizable(false)
            .collapsible(false)
            .show(ctx, |ui| {
                ui.label(trf("{n} MIDI-Part(s)", &[("n", &count)]));
                ui.horizontal(|ui| {
                    ui.radio_value(&mut d.pdf, false, "MusicXML");
                    ui.radio_value(&mut d.pdf, true, tr("PDF (Druck)"));
                });
                ui.horizontal(|ui| {
                    ui.label(tr("Titel:"));
                    ui.add(egui::TextEdit::singleline(&mut d.layout.title).desired_width(240.0))
                        .widget_info(|| {
                            egui::WidgetInfo::labeled(egui::Role::TextInput, true, tr("Titel"))
                        });
                });
                if d.pdf {
                    ui.horizontal(|ui| {
                        ui.label(tr("Papier:"));
                        for p in Paper::ALL {
                            ui.radio_value(&mut d.layout.paper, p, p.label());
                        }
                    });
                    ui.horizontal(|ui| {
                        ui.label(tr("Ränder:"));
                        ui.add(
                            egui::DragValue::new(&mut d.layout.margin_mm)
                                .range(5.0..=50.0)
                                .suffix(" mm"),
                        )
                        .widget_info(|| {
                            egui::WidgetInfo::labeled(egui::Role::SpinButton, true, tr("Ränder"))
                        });
                        ui.label(tr("Notengröße:"));
                        ui.add(
                            egui::DragValue::new(&mut d.layout.space_mm)
                                .range(1.0..=3.0)
                                .speed(0.05)
                                .suffix(" mm"),
                        )
                        .on_hover_text(tr("Abstand zweier Notenlinien"))
                        .widget_info(|| {
                            egui::WidgetInfo::labeled(
                                egui::Role::SpinButton,
                                true,
                                tr("Notengröße"),
                            )
                        });
                    });
                }
                if let Some(m) = &d.message {
                    ui.label(m);
                }
                ui.separator();
                run = ui
                    .add_enabled(count > 0, egui::Button::new(tr("Exportieren …")))
                    .clicked();
            });
        self.score_view.export.open = open;
        if !run {
            return;
        }
        let as_pdf = self.score_view.export.pdf;
        let (ext, name) = if as_pdf {
            ("pdf", "PDF")
        } else {
            ("musicxml", "MusicXML")
        };
        let mut file = self.score_view.export.layout.title.trim().to_owned();
        if file.is_empty() {
            file = tr("Noten").to_owned();
        }
        let mut dialog = crate::file_dialog::FileDialog::new()
            .add_filter(name, &[ext])
            .set_file_name(format!("{file}.{ext}"));
        if let Some(dir) = self.project_path().and_then(Path::parent) {
            dialog = dialog.set_directory(dir);
        }
        let Some(path): Option<PathBuf> = dialog.save_file() else {
            return;
        };
        self.score_view.export.message = Some(match self.export_score_to(&path, as_pdf) {
            Ok(()) => trf("Gespeichert: {file}", &[("file", &path.display())]),
            Err(e) => e,
        });
    }
}
