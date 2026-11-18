//! Projektdatei: offenes, versioniertes JSON (Spec „Bewusste Abweichungen“:
//! diff- und git-freundlich). Audio liegt daneben im Ordner `Audio/`.

use std::fmt;

use serde::{Deserialize, Serialize};

use crate::Project;

/// Kennung im Dateikopf, damit fremde JSON-Dateien klar abgelehnt werden.
pub const FORMAT: &str = "studiocraft-project";
/// Aktuelle Version des Dateiformats.
pub const VERSION: u32 = 1;
/// Dateiendung der Projektdatei.
pub const EXTENSION: &str = "studiocraft";
/// Unterordner für Audiodateien im Projektordner.
pub const AUDIO_DIR: &str = "Audio";

#[derive(Serialize, Deserialize)]
struct Header {
    format: String,
    version: u32,
}

#[derive(Serialize, Deserialize)]
struct FileV1 {
    format: String,
    version: u32,
    project: Project,
}

#[derive(Debug)]
pub enum FileError {
    Json(serde_json::Error),
    NotAProject,
    NewerVersion(u32),
}

impl fmt::Display for FileError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Json(e) => write!(f, "Projektdatei beschädigt: {e}"),
            Self::NotAProject => write!(f, "keine StudioCraft-Projektdatei"),
            Self::NewerVersion(v) => {
                write!(
                    f,
                    "Projekt stammt aus einer neueren Version (Format {v}, bekannt bis {VERSION})"
                )
            }
        }
    }
}

impl std::error::Error for FileError {}

pub fn to_json(project: &Project) -> String {
    let file = FileV1 {
        format: FORMAT.into(),
        version: VERSION,
        project: project.clone(),
    };
    // Serialisierung eigener Typen ohne Map-Schlüssel kann nicht scheitern.
    serde_json::to_string_pretty(&file).unwrap_or_default()
}

pub fn from_json(text: &str) -> Result<Project, FileError> {
    let header: Header = serde_json::from_str(text).map_err(|_| FileError::NotAProject)?;
    if header.format != FORMAT {
        return Err(FileError::NotAProject);
    }
    if header.version > VERSION {
        return Err(FileError::NewerVersion(header.version));
    }
    let file: FileV1 = serde_json::from_str(text).map_err(FileError::Json)?;
    let mut project = file.project;
    project.ensure_track_ids();
    Ok(project)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{PoolClip, TrackKind};

    #[test]
    fn roundtrip_keeps_everything() {
        let mut p = Project::demo(8);
        let c = p.add_clip(PoolClip {
            name: "take".into(),
            path: Some("Audio/take.wav".into()),
            sample_rate: 48_000,
            frames: 96_000,
        });
        let t = p.add_track(TrackKind::Audio, "Gesang");
        let id = p.add_audio_event(t, 0, c).unwrap();
        p.set_fades(id, 480, 960);
        p.tracks[t].volume_db = -3.5;
        p.tracks[t].pan = 0.25;
        p.master_db = -1.0;
        let json = to_json(&p);
        assert!(json.contains("\"format\": \"studiocraft-project\""));
        let back = from_json(&json).unwrap();
        assert_eq!(back, p);
        // Neue Events bekommen nach dem Laden weiter eindeutige Ids.
        let mut back = back;
        let new = back.create_event(t, 0, 960).unwrap();
        assert!(p.event(new).is_none());
    }

    #[test]
    fn rejects_foreign_and_newer_files() {
        assert!(matches!(
            from_json("{\"hallo\": 1}"),
            Err(FileError::NotAProject)
        ));
        assert!(matches!(
            from_json("{\"format\": \"anderes\", \"version\": 1}"),
            Err(FileError::NotAProject)
        ));
        let newer = to_json(&Project::default()).replace("\"version\": 1", "\"version\": 99");
        assert!(matches!(
            from_json(&newer),
            Err(FileError::NewerVersion(99))
        ));
    }
}
