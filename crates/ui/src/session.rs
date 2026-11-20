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
                // Keine Datei (z. B. synthetisch erzeugt): Samples als WAV ablegen.
                _ => {
                    let loaded = self.clips.get(&(id as u32)).ok_or_else(|| {
                        crate::i18n::trf("{name}: keine Audiodaten", &[("name", &clip.name)])
                    })?;
                    let target = unique_name(&audio, &format!("{}.wav", sanitize(&clip.name)));
                    export::write_wav_f32(&loaded.samples, clip.sample_rate, &target)?;
                    PathBuf::from(AUDIO_DIR).join(target.file_name().unwrap_or_default())
                }
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
        self.save_as(&next_version(&path)).map(|()| true)
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

    /// Mixdown (Spec MVP: „Export als WAV, Mixdown der Locator-Range“), 24 Bit.
    pub fn export_mixdown(&mut self, path: &Path) -> Result<(), String> {
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
        let peak = export::render_file(&song, self.sample_rate, frames(a), frames(b), path)?;
        self.message = (peak > 1.0).then(|| {
            crate::i18n::trf(
                "Export übersteuert (Spitze {db} dBFS) – Ausgang leiser stellen",
                &[("db", &format!("{:+.1}", 20.0 * peak.log10()))],
            )
        });
        Ok(())
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

/// `Song.studiocraft` → `Song-01.studiocraft`, `Song-01` → `Song-02` (erste freie Nummer).
fn next_version(path: &Path) -> PathBuf {
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
        .unwrap_or_else(|| dir.join(format!("{base}-99.{EXTENSION}")))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn version_names() {
        let dir = std::env::temp_dir().join(format!("sc-version-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let song = dir.join("Song.studiocraft");
        assert_eq!(next_version(&song), dir.join("Song-01.studiocraft"));
        std::fs::write(dir.join("Song-01.studiocraft"), "").unwrap();
        assert_eq!(next_version(&song), dir.join("Song-02.studiocraft"));
        assert_eq!(
            next_version(&dir.join("Song-01.studiocraft")),
            dir.join("Song-02.studiocraft")
        );
        assert_eq!(
            with_extension(Path::new("/x/Lied")),
            PathBuf::from("/x/Lied.studiocraft")
        );
    }
}
