//! Projekt speichern, öffnen, neu anlegen und als Mixdown exportieren.
//!
//! Aufbau auf der Platte (Spec „Grundsatzentscheidungen“, Projektformat):
//!
//! ```text
//! Mein Song/
//!   Mein Song.studiocraft   offenes JSON, versioniert
//!   Audio/                  alle Audiodateien des Projekts
//! ```
//!
//! Beim Speichern landen Dateien von außerhalb als Kopie in `Audio/`;
//! in der Projektdatei stehen nur Pfade relativ zum Projektordner.

use std::{
    collections::HashMap,
    path::{Path, PathBuf},
    sync::Arc,
};

use studiocraft_engine::{AudioClip, Song, export};
use studiocraft_project::{
    Position, Project,
    file::{self, AUDIO_DIR, EXTENSION},
};

use crate::engine_errors::TrErr;
use crate::{LoadedClip, ProjectWindow, waveform::Peaks};

impl ProjectWindow {
    pub fn project_path(&self) -> Option<&Path> {
        self.project_path.as_deref()
    }

    /// Ob es ungesicherte Änderungen gibt.
    pub fn is_modified(&self) -> bool {
        self.plugins.changed || self.saved.as_ref() != Some(&self.project)
    }

    /// Titel für das Fenster: Projektname, `*` bei ungesicherten Änderungen.
    pub fn title(&self) -> String {
        let name = self
            .project_path
            .as_ref()
            .and_then(|p| p.file_stem())
            .map_or(crate::i18n::tr("Unbenannt").into(), |s| {
                s.to_string_lossy().into_owned()
            });
        let star = if self.is_modified() { " *" } else { "" };
        format!("{name}{star} – StudioCraft")
    }

    /// Leeres Projekt.
    pub fn new_project(&mut self) {
        self.replace_project(Project::default(), HashMap::new(), None);
        self.saved = Some(self.project.clone());
    }

    pub(crate) fn replace_project(
        &mut self,
        project: Project,
        clips: HashMap<u32, LoadedClip>,
        path: Option<PathBuf>,
    ) {
        self.project = project;
        self.apply_layout(self.project.layout.clone());
        self.clips = clips;
        self.project_path = path;
        self.selection.clear();
        self.selected_track = (!self.project.tracks.is_empty()).then_some(0);
        self.clear_history();
        self.drag = None;
        self.dirty = true;
        self.message = None;
        self.locate(0);
    }

    /// Speichert unter `path` (Datei `<Name>.studiocraft`, Ordner = Projektordner).
    pub fn save_as(&mut self, path: &Path) -> Result<(), String> {
        self.capture_plugin_states();
        let path = with_extension(path);
        let dir = path
            .parent()
            .filter(|d| !d.as_os_str().is_empty())
            .unwrap_or(Path::new("."));
        let audio = dir.join(AUDIO_DIR);
        std::fs::create_dir_all(&audio).map_err(|e| format!("{}: {e}", audio.display()))?;

        // Audio in den Projektordner holen und Pfade relativ machen.
        let mut stored = self.project.clone();
        // Clips, deren Samples sich nicht schreiben ließen: Das Projekt wird trotzdem
        // gespeichert, sie bleiben im Speicher und das Projekt als geändert markiert.
        let mut unsaved: Vec<String> = Vec::new();
        for (id, clip) in stored.pool.iter_mut().enumerate() {
            let source = clip.path.as_ref().map(PathBuf::from);
            let inside = source
                .as_ref()
                .filter(|p| p.is_absolute())
                .and_then(|p| p.strip_prefix(dir).ok().map(Path::to_path_buf));
            let rel = match (inside, source) {
                (Some(rel), _) => rel,
                (None, Some(src)) if src.is_relative() && dir.join(&src).exists() => src,
                (None, Some(src)) if src.exists() => {
                    let target = unique_name(&audio, &file_name_of(&src, &clip.name));
                    std::fs::copy(&src, &target).map_err(|e| format!("{}: {e}", src.display()))?;
                    PathBuf::from(AUDIO_DIR).join(target.file_name().unwrap_or_default())
                }
                (_, source) => match (self.clips.get(&(id as u32)), source) {
                    // Keine Datei (z. B. synthetisch erzeugt): Samples als WAV ablegen.
                    (Some(loaded), _) => {
                        let target = unique_name(&audio, &format!("{}.wav", sanitize(&clip.name)));
                        if let Err(e) =
                            export::write_wav_f32(&loaded.samples, clip.sample_rate, &target)
                        {
                            let _ = std::fs::remove_file(&target);
                            let e = crate::engine_errors::engine_error_text(&e);
                            unsaved.push(format!("{} ({e})", clip.name));
                            continue;
                        }
                        PathBuf::from(AUDIO_DIR).join(target.file_name().unwrap_or_default())
                    }
                    // Datei fehlt und nichts geladen (beim Öffnen gemeldet): den Verweis
                    // behalten, damit das Projekt sich trotzdem speichern lässt und die Datei
                    // wieder klingt, sobald sie da ist.
                    (None, Some(src)) => src,
                    (None, None) => {
                        return Err(crate::i18n::trf(
                            "{name}: keine Audiodaten",
                            &[("name", &clip.name)],
                        ));
                    }
                },
            };
            clip.path = Some(rel.to_string_lossy().replace('\\', "/"));
        }

        let json = file::to_json(&stored);
        let tmp = path.with_extension(format!("{EXTENSION}.tmp"));
        std::fs::write(&tmp, json).map_err(|e| format!("{}: {e}", tmp.display()))?;
        std::fs::rename(&tmp, &path).map_err(|e| format!("{}: {e}", path.display()))?;

        // Im Speicher absolute Pfade behalten, damit „Speichern unter“ woanders hin weiter kopieren kann.
        for clip in stored.pool.iter_mut() {
            if let Some(p) = &clip.path {
                clip.path = Some(dir.join(p).to_string_lossy().into_owned());
            }
        }
        self.project = stored;
        self.project_path = Some(path);
        self.saved = Some(self.project.clone());
        self.plugins.changed = false;
        self.message = None;
        if !unsaved.is_empty() {
            // Nie alles verloren, nie still: gespeichert ist der Rest, der Clip bleibt im
            // Speicher, und „geändert“ erinnert daran.
            self.saved = None;
            self.message = Some(crate::i18n::trf(
                "Gespeichert, aber ohne diese Clips (sie bleiben nur im Speicher): {clips}",
                &[("clips", &unsaved.join(", "))],
            ));
        }
        Ok(())
    }

    /// Speichert unter dem bisherigen Pfad. `Ok(false)`: noch kein Pfad bekannt.
    pub fn save(&mut self) -> Result<bool, String> {
        match self.project_path.clone() {
            Some(p) => self.save_as(&p).map(|()| true),
            None => Ok(false),
        }
    }

    /// Speichert eine neue Version neben dem Projekt: `Name-01`, `Name-02` …
    pub fn save_new_version(&mut self) -> Result<bool, String> {
        let Some(path) = self.project_path.clone() else {
            return Ok(false);
        };
        let next = next_version(&path)
            .ok_or_else(|| crate::i18n::tr("Alle Versionsnummern bis 99 sind belegt").to_owned())?;
        self.save_as(&next).map(|()| true)
    }

    /// Öffnet ein Projekt und lädt seine Audiodateien. Fehlende Dateien
    /// werden gemeldet; ihre Events bleiben erhalten, klingen aber nicht.
    pub fn open(&mut self, path: &Path) -> Result<(), String> {
        let text = std::fs::read_to_string(path).map_err(|e| format!("{}: {e}", path.display()))?;
        let mut project = file::from_json(&text).map_err(|e| format!("{}: {e}", path.display()))?;
        let dir = path.parent().unwrap_or(Path::new("."));
        let mut clips = HashMap::new();
        let mut missing = Vec::new();
        for id in 0..project.pool.len() as u32 {
            let clip = &mut project.pool[id as usize];
            let Some(rel) = clip.path.clone() else {
                missing.push(clip.name.clone());
                continue;
            };
            let abs = dir.join(&rel);
            clip.path = Some(abs.to_string_lossy().into_owned());
            match AudioClip::load(&abs) {
                Ok(loaded) => {
                    let loaded = loaded.resampled(self.sample_rate);
                    let peaks = Peaks::new(&loaded.samples);
                    clips.insert(
                        id,
                        LoadedClip {
                            samples: loaded.samples,
                            peaks,
                        },
                    );
                    project.rescale_clip(id, self.sample_rate);
                }
                Err(_) => missing.push(rel),
            }
        }
        let saved = project.clone();
        self.replace_project(project, clips, Some(path.to_path_buf()));
        self.saved = Some(saved);
        if !missing.is_empty() {
            self.message = Some(crate::i18n::trf(
                "Fehlende Audiodateien: {liste}",
                &[("liste", &missing.join(", "))],
            ));
        }
        Ok(())
    }

    /// Bereich für den Export: zwischen den Locatoren, sonst das ganze Projekt.
    pub fn export_range(&self) -> (u64, u64) {
        if self.right_locator > self.left_locator {
            (self.left_locator, self.right_locator)
        } else {
            (0, self.project.length())
        }
    }

    /// Mixdown (Spec MVP: „Export als WAV, Mixdown der Locator-Range“), 24 Bit. Gibt die
    /// Spitze (linear, vor dem Begrenzen) zurück.
    pub fn export_mixdown(&mut self, path: &Path) -> Result<f32, String> {
        let samples: HashMap<u32, Arc<[f32]>> = self
            .clips
            .iter()
            .map(|(&id, c)| (id, c.samples.clone()))
            .collect();
        let song = Song::build(&self.project, &samples, self.sample_rate);
        let (a, b) = self.export_range();
        let tempo = &self.project.tempo;
        let frames =
            |t: u64| (tempo.seconds(Position(t)) * f64::from(self.sample_rate)).round() as u64;
        let peak =
            export::render_file(&song, self.sample_rate, frames(a), frames(b), path).tr_err()?;
        self.message = (peak > 1.0).then(|| {
            crate::i18n::trf(
                "Export übersteuert (Spitze {db} dBFS) – Ausgang leiser stellen",
                &[("db", &format!("{:+.1}", 20.0 * peak.log10()))],
            )
        });
        Ok(peak)
    }
}

fn with_extension(path: &Path) -> PathBuf {
    if path.extension().is_some_and(|e| e == EXTENSION) {
        path.to_path_buf()
    } else {
        let mut p = path.as_os_str().to_owned();
        p.push(".");
        p.push(EXTENSION);
        PathBuf::from(p)
    }
}

fn file_name_of(src: &Path, fallback: &str) -> String {
    src.file_name().map_or_else(
        || format!("{}.wav", sanitize(fallback)),
        |n| n.to_string_lossy().into_owned(),
    )
}

pub(crate) fn sanitize(name: &str) -> String {
    name.chars()
        .map(|c| {
            if c.is_alphanumeric() || " -_".contains(c) {
                c
            } else {
                '_'
            }
        })
        .collect()
}

/// `dir/name`, bei Bedarf mit Zähler: `name (2).wav`.
pub(crate) fn unique_name(dir: &Path, name: &str) -> PathBuf {
    let candidate = dir.join(name);
    if !candidate.exists() {
        return candidate;
    }
    let p = Path::new(name);
    let stem = p.file_stem().unwrap_or_default().to_string_lossy();
    let ext = p
        .extension()
        .map(|e| format!(".{}", e.to_string_lossy()))
        .unwrap_or_default();
    (2..)
        .map(|n| dir.join(format!("{stem} ({n}){ext}")))
        .find(|c| !c.exists())
        .unwrap_or(candidate)
}

/// `Song.studiocraft` → `Song-01.studiocraft`, `Song-01` → `Song-02` (erste freie Nummer);
/// `None`, wenn alle Nummern bis 99 belegt sind (nie eine vorhandene Version überschreiben).
fn next_version(path: &Path) -> Option<PathBuf> {
    let dir = path.parent().unwrap_or(Path::new("."));
    let stem = path
        .file_stem()
        .unwrap_or_default()
        .to_string_lossy()
        .into_owned();
    let base = match stem.rsplit_once('-') {
        Some((b, n)) if n.len() == 2 && n.chars().all(|c| c.is_ascii_digit()) => b.to_owned(),
        _ => stem,
    };
    (1..100)
        .map(|n| dir.join(format!("{base}-{n:02}.{EXTENSION}")))
        .find(|c| !c.exists())
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A clip without a file whose samples cannot be written (beyond 4 GiB, or as here an odd
    /// stereo sample count that hound refuses to finish) does not stop the save: the project
    /// and the other clips are saved, the message names the clip, it stays in memory and the
    /// project stays changed.
    #[test]
    fn a_clip_that_cannot_be_written_does_not_stop_the_save() {
        let dir = std::env::temp_dir().join(format!("sc-save-part-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let mut w = ProjectWindow::new(Project::default(), None, String::new());
        let clip = |w: &mut ProjectWindow, name: &str, samples: Vec<f32>| {
            let id = w.project.add_clip(studiocraft_project::PoolClip {
                name: name.into(),
                path: None,
                sample_rate: 48_000,
                frames: samples.len() as u64 / 2,
            });
            let peaks = crate::Peaks::new(&samples);
            w.clips.insert(
                id,
                crate::LoadedClip {
                    samples: samples.into(),
                    peaks,
                },
            );
            id
        };
        let good = clip(&mut w, "Gut", vec![0.1, 0.2, 0.3, 0.4]);
        let bad = clip(&mut w, "Kaputt", vec![0.1, 0.2, 0.3]);
        let path = dir.join("Song.studiocraft");
        assert!(w.save_as(&path).is_ok(), "the project is saved");
        let back = file::from_json(&std::fs::read_to_string(&path).unwrap()).unwrap();
        assert!(
            back.pool[good as usize].path.is_some(),
            "the other clip is saved"
        );
        assert!(back.pool[bad as usize].path.is_none());
        let message = w.message().unwrap_or_default().to_owned();
        assert!(message.contains("Kaputt"), "{message}");
        assert!(w.clips.contains_key(&bad), "the clip stays in memory");
        assert!(w.is_modified(), "the project stays changed");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn version_names() {
        let dir = std::env::temp_dir().join(format!("sc-version-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let song = dir.join("Song.studiocraft");
        assert_eq!(next_version(&song), Some(dir.join("Song-01.studiocraft")));
        std::fs::write(dir.join("Song-01.studiocraft"), "").unwrap();
        assert_eq!(next_version(&song), Some(dir.join("Song-02.studiocraft")));
        assert_eq!(
            next_version(&dir.join("Song-01.studiocraft")),
            Some(dir.join("Song-02.studiocraft"))
        );
        assert_eq!(
            with_extension(Path::new("/x/Lied")),
            PathBuf::from("/x/Lied.studiocraft")
        );
    }
}
