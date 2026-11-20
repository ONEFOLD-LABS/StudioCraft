//! Projekte als ein Paket aus Bytes: für den Browser, der keine Ordner und Pfade kennt.
//!
//! Ein Bündel ist ein zip mit demselben Aufbau wie ein Projektordner auf der Platte:
//!
//! ```text
//! Mein Song/Mein Song.studiocraft
//! Mein Song/Audio/<Clip>.wav        (32-Bit-Float, verlustfrei)
//! ```
//!
//! Öffnen nimmt so ein zip oder eine einzelne Projektdatei (dann ohne Audio). Dazu kommen
//! Audio-Import und Mixdown als Bytes.

// Nativ nur von Tests benutzt; im Browser von den Datei-Menüs.
#![cfg_attr(not(target_arch = "wasm32"), allow(dead_code))]

use std::collections::HashMap;
use std::io::{Cursor, Read, Write};
use std::path::PathBuf;
use std::sync::Arc;

use studiocraft_engine::{AudioClip, Song, export};
use studiocraft_project::file::{self, AUDIO_DIR, EXTENSION};
use studiocraft_project::{Position, Project};

use crate::session::sanitize;
use crate::{LoadedClip, ProjectWindow, waveform::Peaks};

/// Obergrenze für ein entpacktes Bündel: schützt vor zip-Bomben.
const MAX_UNPACKED: u64 = 4 * 1024 * 1024 * 1024;

impl ProjectWindow {
    /// Name des Projekts ohne Endung (für Dateinamen von Downloads).
    pub(crate) fn project_stem(&self) -> String {
        self.project_path
            .as_ref()
            .and_then(|p| p.file_stem())
            .map_or("Unbenannt".into(), |s| s.to_string_lossy().into_owned())
    }

    /// Das Projekt samt Audio als zip-Bündel: (Dateiname, Bytes).
    pub(crate) fn project_bundle(&mut self) -> Result<(String, Vec<u8>), String> {
        self.capture_plugin_states();
        let stem = sanitize(&self.project_stem());
        let mut stored = self.project.clone();
        let mut audio: Vec<(String, Vec<u8>)> = Vec::new();
        for (id, clip) in stored.pool.iter_mut().enumerate() {
            let Some(loaded) = u32::try_from(id).ok().and_then(|id| self.clips.get(&id)) else {
                clip.path = None;
                continue;
            };
            let mut name = format!("{}.wav", sanitize(&clip.name));
            let mut n = 2;
            while audio.iter().any(|(a, _)| *a == name) {
                name = format!("{}-{n}.wav", sanitize(&clip.name));
                n += 1;
            }
            audio.push((
                name.clone(),
                export::wav_f32_bytes(&loaded.samples, clip.sample_rate)?,
            ));
            clip.path = Some(format!("{AUDIO_DIR}/{name}"));
        }
        let mut zip = zip::ZipWriter::new(Cursor::new(Vec::new()));
        let options = zip::write::SimpleFileOptions::default()
            .compression_method(zip::CompressionMethod::Deflated);
        let err = |e: zip::result::ZipError| format!("zip: {e}");
        zip.start_file(format!("{stem}/{stem}.{EXTENSION}"), options)
            .map_err(err)?;
        zip.write_all(file::to_json(&stored).as_bytes())
            .map_err(|e| format!("zip: {e}"))?;
        for (name, bytes) in &audio {
            zip.start_file(format!("{stem}/{AUDIO_DIR}/{name}"), options)
                .map_err(err)?;
            zip.write_all(bytes).map_err(|e| format!("zip: {e}"))?;
        }
        let bytes = zip.finish().map_err(err)?.into_inner();
        self.saved = Some(self.project.clone());
        self.plugins.changed = false;
        Ok((format!("{stem}.{EXTENSION}.zip"), bytes))
    }

    /// Öffnet ein Bündel (zip) oder eine einzelne Projektdatei aus Bytes. `name` ist der
    /// Dateiname, wie der Browser ihn meldet.
    pub(crate) fn open_bundle(&mut self, name: &str, bytes: &[u8]) -> Result<(), String> {
        let (mut project, files) = if bytes.starts_with(b"PK") {
            read_zip(bytes)?
        } else {
            let text = std::str::from_utf8(bytes).map_err(|e| format!("{name}: {e}"))?;
            let project = file::from_json(text).map_err(|e| format!("{name}: {e}"))?;
            (project, HashMap::new())
        };
        let mut clips = HashMap::new();
        let mut missing = Vec::new();
        for id in 0..project.pool.len() {
            let Ok(id32) = u32::try_from(id) else { break };
            let clip = &mut project.pool[id];
            let rel = clip.path.take().unwrap_or_default().replace('\\', "/");
            let Some(data) = files.get(&rel) else {
                missing.push(clip.name.clone());
                continue;
            };
            let ext = rel.rsplit('.').next();
            match AudioClip::from_bytes(data, ext) {
                Ok(loaded) => {
                    let loaded = loaded.resampled(self.sample_rate);
                    let peaks = Peaks::new(&loaded.samples);
                    clips.insert(
                        id32,
                        LoadedClip {
                            samples: loaded.samples,
                            peaks,
                        },
                    );
                    project.rescale_clip(id32, self.sample_rate);
                }
                Err(_) => missing.push(rel),
            }
        }
        let stem = name
            .trim_end_matches(".zip")
            .trim_end_matches(&format!(".{EXTENSION}"))
            .to_owned();
        let saved = project.clone();
        // Nur der Name zählt (Titel, Downloads); im Browser gibt es keinen Ordner dazu.
        self.replace_project(
            project,
            clips,
            Some(PathBuf::from(format!("{stem}.{EXTENSION}"))),
        );
        self.saved = Some(saved);
        if !missing.is_empty() {
            self.message = Some(format!("Fehlende Audiodateien: {}", missing.join(", ")));
        }
        Ok(())
    }

    /// Importiert eine Audiodatei aus Bytes auf eine neue Spur.
    pub(crate) fn import_audio_bytes(&mut self, name: &str, bytes: &[u8]) -> Result<(), String> {
        let (stem, ext) = match name.rsplit_once('.') {
            Some((s, e)) => (s, Some(e)),
            None => (name, None),
        };
        let clip = AudioClip::from_bytes(bytes, ext).map_err(|e| format!("{name}: {e}"))?;
        self.import_clip(stem, None, clip, None).map(|_| ())
    }

    /// Mixdown des Export-Bereichs als Bytes im Format `ext` (wav, flac, aiff): (Dateiname,
    /// Bytes, Spitze).
    pub(crate) fn mixdown_bytes(&mut self, ext: &str) -> Result<(String, Vec<u8>, f32), String> {
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
        let mix = export::mix_range(&song, frames(a), frames(b));
        let (bytes, peak) = export::encode_audio(&mix, self.sample_rate, ext)?;
        Ok((
            format!("{}.{ext}", sanitize(&self.project_stem())),
            bytes,
            peak,
        ))
    }
}

/// Liest ein Bündel: die erste Projektdatei und alle Dateien, nach Pfad relativ zu ihrem
/// Ordner. Größen werden begrenzt.
fn read_zip(bytes: &[u8]) -> Result<(Project, HashMap<String, Vec<u8>>), String> {
    let err = |e: zip::result::ZipError| format!("zip: {e}");
    let mut archive = zip::ZipArchive::new(Cursor::new(bytes)).map_err(err)?;
    let mut entries: Vec<(String, Vec<u8>)> = Vec::new();
    let mut total = 0u64;
    for i in 0..archive.len() {
        let mut entry = archive.by_index(i).map_err(err)?;
        if entry.is_dir() {
            continue;
        }
        let Some(path) = entry.enclosed_name() else {
            continue;
        };
        let path = path.to_string_lossy().replace('\\', "/");
        total = total.saturating_add(entry.size());
        if total > MAX_UNPACKED {
            return Err("zip: zu groß".into());
        }
        let mut data = Vec::new();
        (&mut entry)
            .take(MAX_UNPACKED)
            .read_to_end(&mut data)
            .map_err(|e| format!("zip: {e}"))?;
        entries.push((path, data));
    }
    let (project_path, project_bytes) = entries
        .iter()
        .find(|(p, _)| p.ends_with(&format!(".{EXTENSION}")))
        .ok_or_else(|| format!("zip: keine .{EXTENSION}-Datei"))?;
    let base = project_path
        .rsplit_once('/')
        .map_or(String::new(), |(d, _)| format!("{d}/"));
    let text = std::str::from_utf8(project_bytes).map_err(|e| format!("{project_path}: {e}"))?;
    let project = file::from_json(text).map_err(|e| format!("{project_path}: {e}"))?;
    let files = entries
        .into_iter()
        .filter_map(|(p, d)| p.strip_prefix(&base).map(|r| (r.to_owned(), d)))
        .collect();
    Ok((project, files))
}
