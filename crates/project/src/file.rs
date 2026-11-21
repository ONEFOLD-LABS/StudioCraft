//! Projektdatei: offenes, versioniertes JSON (Spec „Grundsatzentscheidungen“:
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
    // Fremde Eingabe: Werte und Ids in gültige Bereiche (Issue #49).
    project.sanitize();
    Ok(project)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{PoolClip, TrackKind};

    /// `isolated` (Plugin-Bridge) steht nur in der Datei, wenn es gesetzt ist; ältere Dateien
    /// ohne das Feld laden mit `false`.
    #[test]
    fn isolated_inserts_round_trip_and_old_files_load() {
        let mut p = Project::default();
        let t = p.add_track(TrackKind::Audio, "A");
        let slot = |isolated| crate::InsertSlot {
            plugin_id: "x".into(),
            name: "X".into(),
            bypass: false,
            params: Vec::new(),
            state: None,
            isolated,
        };
        p.tracks[t].inserts.push(slot(false));
        p.tracks[t].inserts.push(slot(true));
        let text = to_json(&p);
        assert_eq!(text.matches("\"isolated\"").count(), 1);
        let back = from_json(&text).unwrap();
        assert!(!back.tracks[t].inserts[0].isolated);
        assert!(back.tracks[t].inserts[1].isolated);
        // A file of an older version: no field at all.
        fn strip(v: &mut serde_json::Value) {
            match v {
                serde_json::Value::Object(o) => {
                    o.remove("isolated");
                    o.values_mut().for_each(strip);
                }
                serde_json::Value::Array(a) => a.iter_mut().for_each(strip),
                _ => {}
            }
        }
        let mut v: serde_json::Value = serde_json::from_str(&text).unwrap();
        strip(&mut v);
        let old = v.to_string();
        assert!(!old.contains("isolated"));
        assert!(!from_json(&old).unwrap().tracks[t].inserts[1].isolated);
        // Switching it is a different plugin instance (the chain is rebuilt).
        assert!(!slot(false).same_plugin(&slot(true)));
    }

    /// A project that uses every numeric field the file format has.
    fn rich_project() -> Project {
        let mut p = Project::demo(3);
        let c = p.add_clip(PoolClip {
            name: "take".into(),
            path: Some("Audio/take.wav".into()),
            sample_rate: 48_000,
            frames: 96_000,
        });
        let a = p.add_track(TrackKind::Audio, "Gesang");
        let id = p.add_audio_event(a, 960, c).unwrap();
        p.set_fades(id, 480, 960);
        p.event_mut(id).unwrap().lane = Some(1);
        let i = p.add_track(TrackKind::Instrument, "Synth");
        let part = p.create_event(i, 0, 3840).unwrap();
        p.event_mut(part).unwrap().notes = vec![crate::Note {
            pitch: 60,
            velocity: 100,
            start: -120,
            length: 480,
            spelling: 0,
        }];
        p.tracks[a].automation.push(crate::AutomationLane {
            param: crate::AutoParam::Volume,
            points: vec![crate::AutoPoint {
                tick: 960,
                value: -6.0,
            }],
            muted: false,
            visible: true,
            writing: false,
        });
        p.add_marker(7680);
        p.add_cycle_marker(0, 3840);
        p
    }

    /// Everything that computes with loaded values; must not panic.
    fn exercise(mut p: Project) {
        let _ = p.length();
        let _ = p.tempo.signature.bar_ticks();
        let _ = p.tempo.frames(p.length(), 48_000);
        for t in &p.tracks {
            let _ = t.lanes();
            let _ = t.audible_parts();
            for e in &t.events {
                let _ = e.end();
                let _ = e.contains(e.start.0);
                for n in &e.notes {
                    let _ = n.end();
                }
            }
            for l in &t.automation {
                let _ = l.value_at(960);
            }
        }
        let ids: Vec<crate::EventId> = p
            .tracks
            .iter()
            .flat_map(|t| &t.events)
            .map(|e| e.id)
            .collect();
        for id in ids {
            p.quantize_event(id, &crate::Quantize::default());
        }
        if !p.tracks.is_empty() {
            let _ = p.create_event(0, 0, 960);
        }
        let _ = p.add_track(TrackKind::Audio, "neu");
        p.add_marker(0);
        let _ = to_json(&p);
    }

    #[test]
    fn hostile_numbers_and_truncated_files_load_safely() {
        let json = to_json(&rich_project());
        // Every number in the file, replaced in turn by extreme values.
        let bytes = json.as_bytes();
        let mut spans = Vec::new();
        let mut i = 0;
        while i < bytes.len() {
            if bytes[i].is_ascii_digit()
                || (bytes[i] == b'-' && bytes.get(i + 1).is_some_and(u8::is_ascii_digit))
            {
                let start = i;
                i += 1;
                while i < bytes.len() && (bytes[i].is_ascii_digit() || b".eE+-".contains(&bytes[i]))
                {
                    i += 1;
                }
                spans.push((start, i));
            } else {
                i += 1;
            }
        }
        assert!(
            spans.len() > 40,
            "rich project has numbers: {}",
            spans.len()
        );
        let hostile = [
            "18446744073709551615",
            "9223372036854775807",
            "-9223372036854775808",
            "4294967295",
            "65535",
            "0",
            "1e308",
            "-1e308",
        ];
        let mut loaded = 0;
        for &(a, b) in &spans {
            for h in hostile {
                let text = format!("{}{h}{}", &json[..a], &json[b..]);
                if let Ok(p) = from_json(&text) {
                    loaded += 1;
                    let mut again = p.clone();
                    assert_eq!(again.sanitize(), 0, "sanitize is stable after loading");
                    exercise(p);
                }
            }
        }
        assert!(loaded > spans.len(), "most variants still load: {loaded}");
        // Cut off anywhere: an error, never a panic.
        for cut in (0..json.len()).step_by(7) {
            if json.is_char_boundary(cut) {
                assert!(from_json(&json[..cut]).is_err());
            }
        }
    }

    #[test]
    fn extreme_values_are_pulled_into_range() {
        let mut p = rich_project();
        p.tempo.signature.denominator = 0;
        p.tempo.bpm = f64::NAN;
        let t = p.tracks.len() - 2;
        let e = &mut p.tracks[t].events[0];
        e.start = crate::Position(u64::MAX);
        e.length = u64::MAX;
        e.lane = Some(u32::MAX);
        e.id = u64::MAX;
        p.tracks[t].volume_db = f32::INFINITY;
        p.tracks[t].id = u32::MAX;
        let fixed = p.sanitize();
        assert!(fixed >= 8, "{fixed}");
        let e = &p.tracks[t].events[0];
        assert!(e.end() <= 2 * crate::sanitize::MAX_TICKS);
        assert_eq!(e.lane, Some(crate::sanitize::MAX_LANE));
        assert_eq!(p.tempo.signature.denominator, 4);
        assert_eq!(p.tempo.bpm, 120.0);
        assert!(p.tracks[t].volume_db <= crate::MAX_DB);
        exercise(p);
    }

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
        let e = p.event_mut(id).unwrap();
        e.gain_db = -4.5;
        e.fade_in_curve = crate::FadeCurve::SCurve;
        e.fade_out_curve = crate::FadeCurve::Exponential;
        p.tracks[t].volume_db = -3.5;
        p.tracks[t].pan = 0.25;
        p.master_db = -1.0;
        let json = to_json(&p);
        assert!(json.contains("\"format\": \"studiocraft-project\""));
        assert!(json.contains("\"fade_in_curve\": \"s_curve\""), "{json}");
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
