//! Stem-Export (Issue #56): mehrere Kanäle zwischen den Locatoren, je Kanal eine Datei.
//!
//! Jeder gewählte Kanal (Spur, Gruppe, FX-Kanal oder Hauptmix) wird offline über denselben
//! Bereich gerendert (siehe `render.rs`), so dass alle Dateien sampelgenau gleich beginnen und
//! gleich lang sind. Mit Kanalbearbeitung: Inserts, EQ, Fader und Panorama des Kanals; ohne:
//! nur was hineinläuft (Instrumente spielen immer).

use crate::i18n::tr;
use std::path::{Path, PathBuf};

use studiocraft_engine::export;

use crate::ProjectWindow;
use crate::render::{RenderSpec, RenderTarget};

/// Dateiformat der Stems (wie beim Mixdown, je 24 Bit).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub(crate) enum StemFormat {
    #[default]
    Wav,
    Flac,
    Aiff,
}

impl StemFormat {
    pub const ALL: [StemFormat; 3] = [Self::Wav, Self::Flac, Self::Aiff];
    pub fn extension(self) -> &'static str {
        match self {
            Self::Wav => "wav",
            Self::Flac => "flac",
            Self::Aiff => "aif",
        }
    }
    pub fn label(self) -> &'static str {
        match self {
            Self::Wav => "WAV (24 Bit)",
            Self::Flac => "FLAC (24 Bit)",
            Self::Aiff => "AIFF (24 Bit)",
        }
    }
}

/// Zustand des Dialogs „Stems exportieren“.
#[derive(Debug, Clone)]
pub(crate) struct StemDialog {
    pub open: bool,
    /// Gewählte Kanäle: Spur-Ids, `None` = Hauptmix.
    pub chosen: Vec<Option<u32>>,
    pub with_processing: bool,
    /// Dateiname mit `{projekt}`, `{kanal}` und `{nr}`.
    pub pattern: String,
    pub format: StemFormat,
    pub folder: Option<PathBuf>,
    pub message: Option<String>,
}

impl Default for StemDialog {
    fn default() -> Self {
        Self {
            open: false,
            chosen: Vec::new(),
            with_processing: true,
            pattern: "{projekt} - {kanal}".into(),
            format: StemFormat::Wav,
            folder: None,
            message: None,
        }
    }
}

/// Dateiname aus dem Muster, ohne Zeichen, die Dateisysteme ablehnen.
fn file_name(pattern: &str, project: &str, channel: &str, nr: usize, ext: &str) -> String {
    let name = pattern
        .replace("{projekt}", project)
        .replace("{kanal}", channel)
        .replace("{nr}", &format!("{nr:02}"));
    let clean = crate::session::sanitize(name.trim());
    let clean = if clean.is_empty() {
        format!("Stem {nr:02}")
    } else {
        clean
    };
    format!("{clean}.{ext}")
}

impl ProjectWindow {
    /// Kanäle, die exportiert werden können: (Spur-Id oder `None` = Hauptmix, Name).
    pub(crate) fn stem_channels(&self) -> Vec<(Option<u32>, String)> {
        let mut out: Vec<(Option<u32>, String)> = self
            .project
            .tracks
            .iter()
            .filter(|t| crate::mixer::has_channel(t.kind))
            .map(|t| (Some(t.id), t.name.clone()))
            .collect();
        let main = self
            .project
            .io
            .outputs
            .first()
            .map_or(tr("Summe").to_owned(), |b| b.name.clone());
        out.push((None, main));
        out
    }

    pub(crate) fn open_stem_dialog(&mut self) {
        let all: Vec<Option<u32>> = self
            .project
            .tracks
            .iter()
            .filter(|t| crate::mixer::has_channel(t.kind))
            .map(|t| Some(t.id))
            .collect();
        let folder = self
            .project_path()
            .and_then(Path::parent)
            .map(|p| p.join("Stems"));
        let d = &mut self.stems;
        d.open = true;
        d.message = None;
        if d.chosen.is_empty() {
            d.chosen = all;
        }
        if d.folder.is_none() {
            d.folder = folder;
        }
    }

    /// Exportiert die gewählten Kanäle nach `folder`. Gibt die geschriebenen Dateien zurück.
    pub(crate) fn export_stems(&mut self, folder: &Path) -> Result<Vec<PathBuf>, String> {
        let chosen = self.stems.chosen.clone();
        if chosen.is_empty() {
            return Err(crate::i18n::tr("Keine Kanäle gewählt").into());
        }
        std::fs::create_dir_all(folder).map_err(|e| format!("{}: {e}", folder.display()))?;
        let range = self.export_range();
        let project = self
            .project_path()
            .and_then(Path::file_stem)
            .map_or(tr("Projekt").to_owned(), |n| {
                n.to_string_lossy().into_owned()
            });
        let names = self.stem_channels();
        let (pattern, format, processing) = (
            self.stems.pattern.clone(),
            self.stems.format,
            self.stems.with_processing,
        );
        let mut written = Vec::new();
        for (nr, choice) in chosen.iter().enumerate() {
            let target = match choice {
                None => RenderTarget::Main,
                Some(id) => match self.project.track_index(*id) {
                    Some(i) => RenderTarget::Track(i),
                    None => continue,
                },
            };
            let audio = self.render_offline(RenderSpec {
                target,
                range,
                with_channel: processing,
                inserts: processing,
            })?;
            let channel = names
                .iter()
                .find(|(c, _)| c == choice)
                .map_or(tr("Kanal").to_owned(), |(_, n)| n.clone());
            let path = folder.join(file_name(
                &pattern,
                &project,
                &channel,
                nr + 1,
                format.extension(),
            ));
            export::write_audio(&audio, self.sample_rate, &path)?;
            written.push(path);
        }
        Ok(written)
    }

    /// Das Fenster „Stems exportieren“.
    pub(crate) fn stem_dialog(&mut self, ctx: &egui::Context) {
        if !self.stems.open {
            return;
        }
        let channels = self.stem_channels();
        let mut open = true;
        let mut run = false;
        let d = &mut self.stems;
        egui::Window::new(tr("Stems exportieren"))
            .open(&mut open)
            .resizable(false)
            .collapsible(false)
            .show(ctx, |ui| {
                ui.label(tr(
                    "Kanäle (zwischen den Locatoren, sonst das ganze Projekt):",
                ));
                egui::ScrollArea::vertical()
                    .max_height(220.0)
                    .show(ui, |ui| {
                        for (id, name) in &channels {
                            let mut on = d.chosen.contains(id);
                            if ui.checkbox(&mut on, name.as_str()).changed() {
                                if on {
                                    d.chosen.push(*id);
                                } else {
                                    d.chosen.retain(|c| c != id);
                                }
                            }
                        }
                    });
                ui.horizontal(|ui| {
                    if ui.small_button(tr("Alle")).clicked() {
                        d.chosen = channels.iter().map(|(id, _)| *id).collect();
                    }
                    if ui.small_button(tr("Keine")).clicked() {
                        d.chosen.clear();
                    }
                });
                ui.separator();
                ui.checkbox(&mut d.with_processing, tr("Mit Kanalbearbeitung"))
                    .on_hover_text(tr("Inserts, EQ, Fader und Panorama des Kanals"));
                ui.horizontal(|ui| {
                    let l = ui.label(tr("Dateiname"));
                    ui.add(egui::TextEdit::singleline(&mut d.pattern).desired_width(200.0))
                        .labelled_by(l.id)
                        .on_hover_text(tr("{projekt}, {kanal} und {nr} werden ersetzt"));
                });
                ui.horizontal(|ui| {
                    let l = ui.label(tr("Format"));
                    egui::ComboBox::from_id_salt("stem-format")
                        .selected_text(d.format.label())
                        .show_ui(ui, |ui| {
                            for f in StemFormat::ALL {
                                ui.selectable_value(&mut d.format, f, f.label());
                            }
                        })
                        .response
                        .labelled_by(l.id);
                });
                ui.horizontal(|ui| {
                    let folder = d
                        .folder
                        .as_ref()
                        .map_or(tr("kein Ordner").to_owned(), |p| p.display().to_string());
                    ui.label(folder);
                    #[cfg(not(target_arch = "wasm32"))]
                    if ui.button(tr("Ordner …")).clicked()
                        && let Some(p) = rfd::FileDialog::new().pick_folder()
                    {
                        d.folder = Some(p);
                    }
                });
                if let Some(m) = &d.message {
                    ui.label(m);
                }
                let ready = !d.chosen.is_empty() && d.folder.is_some();
                if ui
                    .add_enabled(ready, egui::Button::new(tr("Exportieren")))
                    .clicked()
                {
                    run = true;
                }
            });
        if run && let Some(folder) = self.stems.folder.clone() {
            self.stems.message = Some(match self.export_stems(&folder) {
                Ok(files) => crate::i18n::trf("{n} Dateien geschrieben", &[("n", &files.len())]),
                Err(e) => e,
            });
        }
        if !open {
            self.stems.open = false;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use studiocraft_engine::AudioClip;
    use studiocraft_project::{Project, TrackKind};

    #[test]
    fn file_names_follow_the_pattern_and_stay_valid() {
        assert_eq!(
            file_name("{projekt} - {kanal}", "Song", "Bass", 3, "wav"),
            "Song - Bass.wav"
        );
        assert_eq!(
            file_name("{nr} {kanal}", "S", "Vox/Lead", 2, "flac"),
            "02 Vox_Lead.flac"
        );
        assert_eq!(file_name("   ", "S", "K", 7, "aif"), "Stem 07.aif");
    }

    #[test]
    fn stems_of_tracks_groups_and_the_main_mix_line_up() {
        let mut w = ProjectWindow::new(Project::default(), None, String::new());
        let clip = |v: f32| AudioClip {
            sample_rate: 48_000,
            samples: vec![v; 2 * 24_000].into(),
        };
        w.import_clip("A", None, clip(0.5), None).unwrap();
        w.import_clip("B", None, clip(0.25), None).unwrap();
        let g = w.project.add_track(TrackKind::Group, "G");
        let gid = w.project.tracks[g].id;
        w.project.tracks[0].output = Some(gid);
        w.project.tracks[g].volume_db = -6.0;
        let a = w.project.tracks[0].id;
        w.stems.chosen = vec![Some(a), Some(gid), None];
        let dir = std::env::temp_dir().join(format!("sc-stems-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let files = w.export_stems(&dir).unwrap();
        assert_eq!(files.len(), 3);
        let load = |p: &Path| AudioClip::load(p).unwrap().samples;
        let (sa, sg, sm) = (load(&files[0]), load(&files[1]), load(&files[2]));
        assert_eq!(sa.len(), sg.len(), "same length");
        assert_eq!(sa.len(), sm.len(), "same length");
        let at = 2 * 12_000;
        let g6 = studiocraft_project::db_to_gain(-6.0);
        assert!((sa[at] - 0.5).abs() < 1e-3, "A: {}", sa[at]);
        assert!(
            (sg[at] - 0.5 * g6).abs() < 1e-3,
            "G with its fader: {}",
            sg[at]
        );
        assert!(
            (sm[at] - (0.5 * g6 + 0.25)).abs() < 1e-3,
            "main: {}",
            sm[at]
        );
        assert!(
            files[1]
                .file_name()
                .unwrap()
                .to_string_lossy()
                .ends_with("- G.wav")
        );
        // Without processing the group's fader is left out.
        w.stems.with_processing = false;
        w.stems.chosen = vec![Some(gid)];
        let files = w.export_stems(&dir).unwrap();
        let sg = load(&files[0]);
        assert!((sg[at] - 0.5).abs() < 1e-3, "G raw: {}", sg[at]);
        let _ = std::fs::remove_dir_all(&dir);
    }
}
