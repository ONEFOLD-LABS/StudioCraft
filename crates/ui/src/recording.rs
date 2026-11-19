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
        let (input, recorder, monitor) = prepared.start()?;
        if let Some(t) = &mut self.transport {
            // Nur einmal pro Sitzung; ist die Queue gerade voll, bleibt Monitor stumm.
            t.send(Command::SetMonitorInput(Box::new(monitor)));
        }
        self.input_status = format!("{} · {} Kanäle", input.info.device, input.info.channels);
        self.input = Some(input);
        self.recorder = Some(recorder);
        Ok(())
    }

    /// Monitor einer Audio-Spur ein- oder ausschalten.
    pub(crate) fn set_monitor(&mut self, track: usize, on: bool) {
        if let Some(t) = self
            .project
            .tracks
            .get_mut(track)
            .filter(|t| t.kind == TrackKind::Audio)
        {
            t.monitor = on;
            self.dirty = true;
        }
    }

    /// Hört eine Spur mit, wird der Eingang geöffnet (erst dann, damit kein Mikrofon ohne
    /// Grund belegt ist). Klappt das nicht, gehen alle Monitore aus und die Statuszeile
    /// nennt den Grund.
    pub(crate) fn sync_monitor_input(&mut self) {
        if self.transport.is_none()
            || self.recorder.is_some()
            || !self.project.tracks.iter().any(|t| t.monitor)
        {
            return;
        }
        if let Err(e) = self.ensure_input() {
            self.input_status = e;
            for t in &mut self.project.tracks {
                t.monitor = false;
            }
        }
    }

    /// Ordner für neue Aufnahmen: `Audio/` im Projekt, sonst ein temporärer Ordner
    /// (beim Speichern wandern die Dateien ins Projekt).
    fn recording_dir(&self) -> PathBuf {
        match self.project_path().and_then(Path::parent) {
            Some(dir) => dir.join(studiocraft_project::file::AUDIO_DIR),
            None => std::env::temp_dir().join("studiocraft-aufnahmen"),
        }
    }

    fn armed_instruments(&self) -> Vec<usize> {
        (0..self.project.tracks.len())
            .filter(|&i| {
                let t = &self.project.tracks[i];
                t.kind == TrackKind::Instrument && t.record
            })
            .collect()
    }

    /// Ziel des MIDI-Keyboards: aufnahmebereite Instrument-Spur, sonst die ausgewählte.
    pub(crate) fn midi_target(&self) -> Option<usize> {
        self.armed_instruments().first().copied().or_else(|| {
            self.selected_track.filter(|&t| {
                self.project
                    .tracks
                    .get(t)
                    .is_some_and(|t| t.kind == TrackKind::Instrument)
            })
        })
    }

    /// Öffnet alle MIDI-Eingänge (Keyboard). Ohne Engine nichts zu tun.
    pub fn open_midi_inputs(&mut self) {
        let Some(t) = &self.transport else { return };
        match studiocraft_engine::midi::MidiInputs::open_all(&t.midi) {
            Ok(m) if m.ports.is_empty() => self.midi_status = "MIDI: kein Eingang".into(),
            Ok(m) => {
                self.midi_status = format!("MIDI: {}", m.ports.join(", "));
                self.midi_inputs = Some(m);
            }
            Err(e) => self.midi_status = e,
        }
    }

    /// Je Frame: Keyboard-Ziel nachführen, aufgenommene Noten abholen.
    pub(crate) fn sync_midi(&mut self) {
        let target = self.midi_target();
        if self.midi_target_sent != Some(target) {
            self.midi_target_sent = Some(target);
            self.send(Command::SetMidiTarget(target));
        }
        if let Some(t) = &mut self.transport {
            while let Ok(m) = t.recorded_midi.pop() {
                if self.recording {
                    self.midi_take.push(m);
                }
            }
        }
    }

    pub(crate) fn start_recording(&mut self) {
        if self.recording || self.transport.is_none() {
            return;
        }
        self.play_from = self.cursor;
        let audio = self.armed_tracks();
        let midi = self.armed_instruments();
        if audio.is_empty() && midi.is_empty() {
            self.message = Some("Keine Spur aufnahmebereit (R im Spurkopf)".into());
            return;
        }
        self.audio_recording = false;
        if !audio.is_empty() {
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
                self.audio_recording = true;
            }
        }
        // Alte, nicht abgeholte Noten verwerfen.
        if let Some(t) = &mut self.transport {
            while t.recorded_midi.pop().is_ok() {}
        }
        self.midi_take.clear();
        self.send(Command::Record);
        self.recording = true;
        self.message = None;
    }

    /// Beendet die Aufnahme (falls eine läuft) und legt die Takes an.
    pub(crate) fn stop_transport(&mut self) {
        let stop_pos = self.transport.as_ref().map_or(0, |t| t.status.cursor());
        self.send(Command::Stop);
        if !self.recording {
            return;
        }
        self.recording = false;
        let status = self
            .transport
            .as_ref()
            .map(|t| (t.status.rec_start(), t.status.rec_played_at()));
        let (cursor, played_at) = status.unwrap_or((0, None));

        // MIDI: noch Unterwegs-Befindliches abholen, dann Part anlegen.
        if let Some(t) = &mut self.transport {
            while let Ok(m) = t.recorded_midi.pop() {
                self.midi_take.push(m);
            }
        }
        let take = std::mem::take(&mut self.midi_take);
        if let Some(track) = self.armed_instruments().first().copied()
            && !take.is_empty()
        {
            self.place_midi_take(track, &take, cursor, stop_pos.max(cursor));
        }

        if !self.audio_recording {
            return;
        }
        self.audio_recording = false;
        let Some(rec) = self.recorder.as_ref() else {
            return;
        };
        let overruns =
            rec.overruns.load(std::sync::atomic::Ordering::Relaxed) - self.overruns_at_start;
        let result = rec.stop();
        let rate = self.sample_rate;
        match result.and_then(|r| {
            // Latenzausgleich: dorthin legen, wo das Gespielte zum Gehörten passt.
            let (start, skip) = match (played_at, r.first_captured_at) {
                (Some(play), Some(cap)) => {
                    studiocraft_engine::clock::compensated_start(cursor, play, cap, rate)
                }
                _ => (cursor, 0),
            };
            self.place_recording(&r, start, skip)
        }) {
            Ok(n) if overruns > 0 => {
                self.message = Some(format!(
                    "{n} Take(s) aufgenommen, aber {overruns} Samples verloren (Puffer voll)"
                ))
            }
            Ok(_) => {}
            Err(e) => self.message = Some(format!("Aufnahme: {e}")),
        }
    }

    /// Baut aus aufgenommenen Noten einen Part (obenauf). `start`/`stop`: Aufnahmebereich
    /// in Frames. Im Cycle umfasst der Part den Cycle-Bereich und mischt alle Durchgänge.
    pub(crate) fn place_midi_take(
        &mut self,
        track: usize,
        take: &[studiocraft_engine::midi::RecordedMidi],
        start: u64,
        stop: u64,
    ) {
        let rate = self.sample_rate;
        let tempo = self.project.tempo.clone();
        let ticks = |f: u64| tempo.ticks(f, rate);
        // Noten aus An/Aus-Paaren; offene Noten enden beim Stopp.
        let mut open: [Option<(u64, u8)>; 128] = [None; 128];
        let mut notes: Vec<(u64, u64, u8, u8)> = Vec::new();
        for m in take {
            let k = usize::from(m.key);
            if m.on {
                if let Some((s, v)) = open[k].take() {
                    notes.push((s, m.pos.max(s + 1), m.key, v));
                }
                open[k] = Some((m.pos, m.velocity));
            } else if let Some((s, v)) = open[k].take() {
                // Im Cycle kann das Aus hinter dem Sprung liegen: dann bis zum rechten Locator.
                let end = if m.pos > s {
                    m.pos
                } else {
                    tempo.frames(self.right_locator, rate).max(s + 1)
                };
                notes.push((s, end, m.key, v));
            }
        }
        for (k, o) in open.iter().enumerate() {
            if let Some((s, v)) = *o {
                notes.push((s, stop.max(s + 1), k as u8, v));
            }
        }
        if notes.is_empty() {
            return;
        }
        let bar = self.bar_ticks();
        let (mut part_start, mut part_end) = if self.cycle && self.right_locator > self.left_locator
        {
            (self.left_locator, self.right_locator)
        } else {
            let first = notes.iter().map(|n| n.0).min().unwrap_or(start).min(start);
            let last = notes.iter().map(|n| n.1).max().unwrap_or(stop).max(stop);
            (ticks(first) / bar * bar, ticks(last).div_ceil(bar) * bar)
        };
        if self.punch_in && self.right_locator > self.left_locator {
            part_start = part_start.max(self.left_locator);
        }
        if self.punch_out && self.right_locator > self.left_locator {
            part_end = part_end.min(self.right_locator);
        }
        if part_end <= part_start {
            return;
        }
        let before = self.project.clone();
        let Some(id) = self
            .project
            .create_event(track, part_start, part_end - part_start)
        else {
            return;
        };
        // Obenauf legen (zuletzt in der Liste) und Noten eintragen.
        let i = self.project.find(id).map(|(_, i)| i).unwrap_or(0);
        let mut ev = self.project.tracks[track].events.remove(i);
        ev.name = format!(
            "{} {:02}",
            self.project.tracks[track].name,
            self.project.tracks[track].events.len() + 1
        );
        ev.notes = notes
            .into_iter()
            .map(|(a, b, key, v)| studiocraft_project::Note {
                pitch: key,
                velocity: v.max(1),
                start: ticks(a) as i64 - part_start as i64,
                length: ticks(b).saturating_sub(ticks(a)).max(1),
            })
            .filter(|n| n.start >= 0 && n.start < (part_end - part_start) as i64)
            .collect();
        self.project.tracks[track].events.push(ev);
        self.selection = vec![id];
        self.checkpoint(before);
    }

    /// `skip`: so viele Frames am Anfang der Aufnahme fallen weg (lagen vor Projektbeginn).
    pub(crate) fn place_recording(
        &mut self,
        rec: &Recording,
        start: u64,
        skip: u64,
    ) -> Result<usize, String> {
        let before = self.project.clone();
        let rate = self.sample_rate;
        let tempo = self.project.tempo.clone();
        let to_frames = |t: u64| tempo.frames(t, rate);
        let cycle = self
            .cycle
            .then(|| (to_frames(self.left_locator), to_frames(self.right_locator)));
        let takes: Vec<Pass> = passes(start, rec.frames.saturating_sub(skip), cycle)
            .into_iter()
            .map(|p| Pass {
                offset: p.offset + skip,
                ..p
            })
            .collect();
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
