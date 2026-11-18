//! Aufnahme (Spec-Abschnitt „Transport und Aufnahme“).
//!
//! Ablauf: Spuren mit R sind aufnahmebereit. Start (● oder `*`) öffnet bei
//! Bedarf den Eingang, startet den Recorder und den Transport im Aufnahmemodus.
//! Beim Stopp entsteht pro aufnahmebereiter Spur eine eigene WAV-Datei aus
//! ihren Eingangskanälen und daraus Events:
//!
//! - **Cycle:** jeder Durchgang wird ein eigener Take; der letzte liegt oben.
//! - **Punch-In/-Out:** nur der Bereich ab dem linken bzw. bis zum rechten
//!   Locator bleibt.
//! - **Über bestehenden Events:** der neue Take liegt obenauf und klingt dort;
//!   die alten bleiben darunter erhalten.

use std::path::{Path, PathBuf};

use studiocraft_engine::{AudioClip, Command, PreparedInput, Recording, export, input};
use studiocraft_project::{PoolClip, TrackKind};

use crate::{
    LoadedClip, ProjectWindow,
    session::{sanitize, unique_name},
    waveform::Peaks,
};

/// Ein Durchgang einer Aufnahme: Projektposition, Offset in der Aufnahme, Länge (Frames).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct Pass {
    pub start: u64,
    pub offset: u64,
    pub length: u64,
}

/// Teilt eine Aufnahme in Cycle-Durchgänge. `cycle` = (links, rechts) in Frames.
pub(crate) fn passes(start: u64, frames: u64, cycle: Option<(u64, u64)>) -> Vec<Pass> {
    let Some((l, r)) = cycle.filter(|&(l, r)| r > l && start < r) else {
        return (frames > 0)
            .then_some(Pass {
                start,
                offset: 0,
                length: frames,
            })
            .into_iter()
            .collect();
    };
    let mut out = Vec::new();
    let first = frames.min(r - start);
    if first > 0 {
        out.push(Pass {
            start,
            offset: 0,
            length: first,
        });
    }
    let mut off = first;
    while off < frames {
        let len = (frames - off).min(r - l);
        out.push(Pass {
            start: l,
            offset: off,
            length: len,
        });
        off += len;
    }
    out
}

impl ProjectWindow {
    pub fn is_recording(&self) -> bool {
        self.recording
    }

    fn armed_tracks(&self) -> Vec<usize> {
        (0..self.project.tracks.len())
            .filter(|&i| {
                let t = &self.project.tracks[i];
                t.kind == TrackKind::Audio && t.record
            })
            .collect()
    }

    /// Kanalzahl des Eingangs (2, solange er noch nicht geöffnet ist).
    pub(crate) fn input_channels(&self) -> u16 {
        self.recorder.as_ref().map_or(2, |r| r.channels)
    }

    /// Öffnet den Eingang beim ersten Bedarf (nicht beim Programmstart,
    /// damit kein Mikrofon ohne Grund belegt wird).
    pub(crate) fn ensure_input(&mut self) -> Result<(), String> {
        if self.recorder.is_some() {
            return Ok(());
        }
        let prepared = PreparedInput::open(self.audio_host.as_deref(), self.sample_rate)?;
        let (input, recorder) = prepared.start()?;
        self.input_status = format!("{} · {} Kanäle", input.info.device, input.info.channels);
        self.input = Some(input);
        self.recorder = Some(recorder);
        Ok(())
    }

    /// Ordner für neue Aufnahmen: `Audio/` im Projekt, sonst ein temporärer Ordner
    /// (beim Speichern wandern die Dateien ins Projekt).
    fn recording_dir(&self) -> PathBuf {
        match self.project_path().and_then(Path::parent) {
            Some(dir) => dir.join(studiocraft_project::file::AUDIO_DIR),
            None => std::env::temp_dir().join("studiocraft-aufnahmen"),
        }
    }

    pub(crate) fn start_recording(&mut self) {
        if self.recording || self.transport.is_none() {
            return;
        }
        let armed = self.armed_tracks();
        if armed.is_empty() {
            self.message = Some("Keine Audio-Spur aufnahmebereit (R im Spurkopf)".into());
            return;
        }
        if let Err(e) = self.ensure_input() {
            self.message = Some(format!("Aufnahme nicht möglich: {e}"));
            return;
        }
        let dir = self.recording_dir();
        if let Err(e) = std::fs::create_dir_all(&dir) {
            self.message = Some(format!("{}: {e}", dir.display()));
            return;
        }
        let path = unique_name(&dir, "Aufnahme.wav");
        if let Some(r) = &self.recorder {
            self.overruns_at_start = r.overruns.load(std::sync::atomic::Ordering::Relaxed);
            r.start(&path);
        }
        self.send(Command::Record);
        self.recording = true;
        self.message = None;
    }

    /// Beendet die Aufnahme (falls eine läuft) und legt die Takes an.
    pub(crate) fn stop_transport(&mut self) {
        self.send(Command::Stop);
        if !self.recording {
            return;
        }
        self.recording = false;
        let Some(rec) = self.recorder.as_ref() else {
            return;
        };
        let overruns =
            rec.overruns.load(std::sync::atomic::Ordering::Relaxed) - self.overruns_at_start;
        let result = rec.stop();
        let start = self.transport.as_ref().map_or(0, |t| t.status.rec_start());
        match result.and_then(|r| self.place_recording(&r, start)) {
            Ok(n) if overruns > 0 => {
                self.message = Some(format!(
                    "{n} Take(s) aufgenommen, aber {overruns} Samples verloren (Puffer voll)"
                ))
            }
            Ok(_) => {}
            Err(e) => self.message = Some(format!("Aufnahme: {e}")),
        }
    }

    /// Legt eine fertige Aufnahme auf alle aufnahmebereiten Audio-Spuren.
    /// `start` ist die Projektposition des ersten Frames. Gibt die Zahl der Takes zurück.
    pub(crate) fn place_recording(&mut self, rec: &Recording, start: u64) -> Result<usize, String> {
        let before = self.project.clone();
        let rate = self.sample_rate;
        let tempo = self.project.tempo.clone();
        let to_frames = |t: u64| tempo.frames(t, rate);
        let cycle = self
            .cycle
            .then(|| (to_frames(self.left_locator), to_frames(self.right_locator)));
        let takes = passes(start, rec.frames, cycle);
        let dir = rec.path.parent().map(Path::to_path_buf).unwrap_or_default();
        let stem = rec
            .path
            .file_stem()
            .unwrap_or_default()
            .to_string_lossy()
            .into_owned();
        let mut placed = 0;

        for track in self.armed_tracks() {
            let t = &self.project.tracks[track];
            let samples = input::extract_channels(rec, t.input.first, t.input.stereo)?;
            let file = unique_name(&dir, &format!("{stem} - {}.wav", sanitize(&t.name)));
            export::write_wav_f32(&samples, rec.sample_rate, &file)?;
            let clip = AudioClip {
                sample_rate: rec.sample_rate,
                samples: samples.into(),
            }
            .resampled(rate);
            let frames = clip.frames();
            let id = self.project.add_clip(PoolClip {
                name: file
                    .file_stem()
                    .unwrap_or_default()
                    .to_string_lossy()
                    .into_owned(),
                path: Some(file.to_string_lossy().into_owned()),
                sample_rate: rate,
                frames,
            });
            let peaks = Peaks::new(&clip.samples);
            self.clips.insert(
                id,
                LoadedClip {
                    samples: clip.samples,
                    peaks,
                },
            );

            let base = self.project.tracks[track].name.clone();
            let existing = self.project.tracks[track].events.len();
            for (k, pass) in takes.iter().enumerate() {
                let name = format!("{base} {:02}", existing + k + 1);
                let Some(ev) = self.project.place_take(
                    track,
                    tempo.ticks(pass.start, rate),
                    id,
                    pass.offset,
                    tempo.ticks(pass.length, rate).max(1),
                    &name,
                ) else {
                    continue;
                };
                if !self.punch(ev) {
                    self.project.delete(ev);
                    continue;
                }
                placed += 1;
                self.selection = vec![ev];
            }
        }
        // Die Mehrkanal-Rohdatei wird nicht mehr gebraucht.
        let _ = std::fs::remove_file(&rec.path);
        self.checkpoint(before);
        Ok(placed)
    }

    /// Wendet Punch-In/-Out auf einen Take an. `false`: nichts bleibt übrig.
    fn punch(&mut self, ev: studiocraft_project::EventId) -> bool {
        let (l, r) = (self.left_locator, self.right_locator);
        if r <= l || !(self.punch_in || self.punch_out) {
            return true;
        }
        let Some(e) = self.project.event(ev) else {
            return false;
        };
        let (s, end) = (e.start.0, e.end());
        if (self.punch_in && end <= l) || (self.punch_out && s >= r) {
            return false;
        }
        if self.punch_in && s < l {
            self.project.trim_start(ev, l);
        }
        if self.punch_out && end > r {
            self.project.trim_end(ev, r);
        }
        true
    }

    /// Start/Stopp mit Aufnahme berücksichtigen.
    pub(crate) fn toggle_record(&mut self) {
        if self.recording {
            self.stop_transport();
        } else {
            self.start_recording();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn passes_without_and_with_cycle() {
        assert_eq!(
            passes(100, 50, None),
            [Pass {
                start: 100,
                offset: 0,
                length: 50
            }]
        );
        // Cycle 0..100, Start bei 80, 230 Frames: 20 + 100 + 100 + 10.
        assert_eq!(
            passes(80, 230, Some((0, 100))),
            [
                Pass {
                    start: 80,
                    offset: 0,
                    length: 20
                },
                Pass {
                    start: 0,
                    offset: 20,
                    length: 100
                },
                Pass {
                    start: 0,
                    offset: 120,
                    length: 100
                },
                Pass {
                    start: 0,
                    offset: 220,
                    length: 10
                },
            ]
        );
        // Start hinter dem Cycle: eine durchgehende Aufnahme.
        assert_eq!(passes(150, 30, Some((0, 100))).len(), 1);
        assert!(passes(0, 0, None).is_empty());
    }
}
