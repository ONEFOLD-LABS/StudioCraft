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

use crate::i18n::tr;
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

/// Höchstens so viele nicht aufgenommene Noten für die nachträgliche Aufnahme.
const RETRO_MAX: usize = 20_000;

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
        let prepared = PreparedInput::open_device(
            self.audio_host.as_deref(),
            self.audio_choice.input.as_deref(),
            self.sample_rate,
        )?;
        let (input, recorder, monitor) = prepared.start()?;
        if let Some(t) = &mut self.transport {
            // Nur einmal pro Sitzung; ist die Queue gerade voll, bleibt Monitor stumm.
            t.send(Command::SetMonitorInput(Box::new(monitor)));
        }
        self.input_status = crate::i18n::trf(
            "{device} · {n} Kanäle",
            &[("device", &input.info.device), ("n", &input.info.channels)],
        );
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
    pub(crate) fn recording_dir(&self) -> PathBuf {
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
        if !self.midi_inputs_on {
            self.midi_status = tr("MIDI: aus").into();
            return;
        }
        let Some(t) = &self.transport else { return };
        match studiocraft_engine::midi::MidiInputs::open_all(&t.midi) {
            Ok(m) if m.ports.is_empty() => self.midi_status = tr("MIDI: kein Eingang").into(),
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
                if self.recording && m.recording {
                    self.midi_take.push(m);
                } else if !m.recording {
                    // Für die nachträgliche Aufnahme merken (begrenzt).
                    if self.retro.len() >= RETRO_MAX {
                        self.retro.pop_front();
                    }
                    self.retro.push_back(m);
                }
            }
        }
    }

    /// Untermenü „MIDI-Aufnahme“: Modus ohne und mit Cycle (Projekteinstellung, Undo).
    pub(crate) fn record_mode_menu(&mut self, ui: &mut egui::Ui) {
        ui.menu_button(tr("MIDI-Aufnahme"), |ui| {
            let mut r = self.project.record;
            ui.label(egui::RichText::new(tr("Über bestehenden Parts")).weak());
            for m in studiocraft_project::MidiRecordMode::ALL {
                ui.radio_value(&mut r.midi, m, tr(m.label()));
            }
            ui.separator();
            ui.label(egui::RichText::new(tr("Im Cycle")).weak());
            for m in studiocraft_project::CycleRecordMode::ALL {
                ui.radio_value(&mut r.cycle_midi, m, tr(m.label()));
            }
            if r != self.project.record {
                let before = self.project.clone();
                self.project.record = r;
                self.checkpoint(before);
            }
        });
    }

    /// Nachträgliche Aufnahme (Umschalt + Nummernblock-Mal): die zuletzt gespielten Noten,
    /// die nicht aufgenommen wurden, als neuer Part auf die Keyboard-Spur. Genommen wird der
    /// letzte zusammenhängende Abschnitt: während der Wiedergabe an ihrer Position, im Stopp
    /// ab dem Cursor im gespielten Zeitabstand.
    pub(crate) fn retrospective_record(&mut self) {
        self.sync_midi();
        let Some(track) = self.midi_target() else {
            self.message = Some(tr("Nachträgliche Aufnahme: keine Instrument-Spur gewählt").into());
            return;
        };
        let Some(last) = self.retro.back().copied() else {
            self.message = Some(tr("Nachträgliche Aufnahme: nichts gespielt").into());
            return;
        };
        let n = self
            .retro
            .iter()
            .rev()
            .take_while(|m| m.running == last.running)
            .count();
        let mut take: Vec<studiocraft_engine::midi::RecordedMidi> = self
            .retro
            .iter()
            .skip(self.retro.len() - n)
            .copied()
            .collect();
        if !last.running {
            // Im Stopp stand die Position still: ab dem Cursor im gespielten Abstand.
            let rate = self.sample_rate;
            let base = self.project.tempo.frames(self.cursor, rate);
            let first = take.first().map_or(0, |m| m.clock);
            for m in &mut take {
                m.pos = base + m.clock.wrapping_sub(first);
            }
        }
        // Wiedergabe im Cycle kann zurückspringen: dann nur der letzte Durchgang.
        if let Some(i) = take.windows(2).rposition(|w| w[1].pos < w[0].pos) {
            take.drain(..=i);
        }
        let (start, stop) = (
            take.iter().map(|m| m.pos).min().unwrap_or(0),
            take.iter().map(|m| m.pos).max().unwrap_or(0),
        );
        let before = self.project.tracks[track].events.len();
        self.place_midi_take_mode(track, &take, start, stop.max(start + 1), false);
        if self.project.tracks[track].events.len() > before {
            self.retro.clear();
            self.message = None;
        } else {
            self.message = Some(tr("Nachträgliche Aufnahme: keine vollständige Note").into());
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
            self.message = Some(tr("Keine Spur aufnahmebereit (R im Spurkopf)").into());
            return;
        }
        self.audio_recording = false;
        if !audio.is_empty() {
            if let Err(e) = self.ensure_input() {
                self.message = Some(crate::i18n::trf(
                    "Aufnahme nicht möglich: {e}",
                    &[("e", &e)],
                ));
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
                // Lässt sich die Datei nicht anlegen: Fehler vor dem Start, keine stille
                // Aufnahme ins Leere.
                if let Err(e) = r.start(&path) {
                    self.message = Some(crate::i18n::trf(
                        "Aufnahme nicht möglich: {e}",
                        &[("e", &e)],
                    ));
                    return;
                }
                self.audio_recording = true;
            }
        }
        // Alte, nicht abgeholte Noten verwerfen.
        if let Some(t) = &mut self.transport {
            while t.recorded_midi.pop().is_ok() {}
        }
        self.midi_take.clear();
        // Vorzähler nur aus dem Stopp (Projekteinstellung, Takte in Taktart und Tempo am Cursor).
        let bars = u64::from(self.project.click.precount_bars);
        self.count_in_frames = 0;
        if bars > 0 && !self.is_playing() {
            let tempo = &self.project.tempo;
            let sig = tempo.signature_at(self.cursor);
            let bpm = tempo.bpm_at(self.cursor).max(1.0);
            let rate = f64::from(self.sample_rate);
            let frames_of = |ticks: u64| {
                (ticks as f64 / studiocraft_project::TICKS_PER_QUARTER as f64 * 60.0 / bpm * rate)
                    .round() as u64
            };
            let beat = frames_of(sig.beat_ticks()).max(1);
            let frames = beat * u64::from(sig.numerator) * bars;
            self.count_in_frames = frames;
            self.send(Command::RecordAfterCount {
                frames,
                beat,
                beats_per_bar: sig.numerator,
            });
        } else {
            self.send(Command::Record);
        }
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
        // Im Vorzähler abgebrochen: nichts aufgenommen.
        if self
            .transport
            .as_ref()
            .is_some_and(|t| !t.status.rec_started())
        {
            if let Some(t) = &mut self.transport {
                while t.recorded_midi.pop().is_ok() {}
            }
            self.midi_take.clear();
            if self.audio_recording {
                self.audio_recording = false;
                if let Some(rec) = self.recorder.as_ref()
                    && let Ok(r) = rec.stop()
                {
                    for path in r.files() {
                        let _ = std::fs::remove_file(path);
                    }
                }
            }
            self.message = Some(tr("Aufnahme im Vorzähler abgebrochen").into());
            return;
        }

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
        // Unterbrochene Aufnahme (z. B. Platte voll): der Teil bis dahin wird trotzdem ein Take.
        let mut problem: Option<(String, u64)> = None;
        match result.and_then(|r| {
            problem = r.problem.clone().map(|p| (p, r.frames));
            // Latenzausgleich: dorthin legen, wo das Gespielte zum Gehörten passt.
            let (start, skip) = match (played_at, r.first_captured_at) {
                (Some(play), Some(cap)) => {
                    studiocraft_engine::clock::compensated_start(cursor, play, cap, rate)
                }
                _ => (cursor, 0),
            };
            // Was während des Vorzählers hereinkam, gehört nicht zum Take.
            let (start, skip) = drop_count_in(start, skip, self.count_in_frames);
            self.place_recording(&r, start, skip)
        }) {
            Ok(_) if problem.is_some() => {
                let (e, frames) = problem.unwrap_or_default();
                let seconds = format!("{:.1}", frames as f64 / f64::from(rate.max(1)));
                self.message = Some(crate::i18n::trf(
                    "Aufnahme unterbrochen ({e}): die ersten {s} s sind als Take übernommen",
                    &[("e", &e), ("s", &seconds)],
                ));
            }
            Ok(n) if overruns > 0 => {
                self.message = Some(crate::i18n::trf(
                    "{n} Take(s) aufgenommen, aber {overruns} Samples verloren (Puffer voll)",
                    &[("n", &n), ("overruns", &overruns)],
                ))
            }
            Ok(_) => {}
            Err(e) => self.message = Some(crate::i18n::trf("Aufnahme: {e}", &[("e", &e)])),
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
        self.place_midi_take_mode(track, take, start, stop, true);
    }

    /// Wie [`Self::place_midi_take`]; ohne `with_cycle` gilt der Cycle nicht (nachträgliche
    /// Aufnahme: immer ein neuer Part).
    pub(crate) fn place_midi_take_mode(
        &mut self,
        track: usize,
        take: &[studiocraft_engine::midi::RecordedMidi],
        start: u64,
        stop: u64,
        with_cycle: bool,
    ) {
        let rate = self.sample_rate;
        let tempo = self.project.tempo.clone();
        let ticks = |f: u64| tempo.ticks(f, rate);
        // Noten aus An/Aus-Paaren; offene Noten enden beim Stopp. Im Cycle beginnt ein neuer
        // Durchgang, wenn die Position zurückspringt.
        let mut open: [Option<(u64, u8, u32)>; 128] = [None; 128];
        let mut notes: Vec<(u64, u64, u8, u8, u32)> = Vec::new();
        let (mut pass, mut last_pos) = (0u32, 0u64);
        // Controller (Pitchbend, CC, Aftertouch) mit Position in Frames und Durchgang.
        let mut ctrls: Vec<(u64, studiocraft_project::CtrlKind, u16, u32)> = Vec::new();
        for m in take {
            if m.pos < last_pos {
                pass += 1;
            }
            last_pos = m.pos;
            if let Some(data) = m.ctrl {
                if let Some((kind, value)) = studiocraft_project::CtrlKind::from_midi(data) {
                    ctrls.push((m.pos, kind, value, pass));
                }
                continue;
            }
            let k = usize::from(m.key);
            if m.on {
                if let Some((s, v, p)) = open[k].take() {
                    notes.push((s, m.pos.max(s + 1), m.key, v, p));
                }
                open[k] = Some((m.pos, m.velocity, pass));
            } else if let Some((s, v, p)) = open[k].take() {
                // Im Cycle kann das Aus hinter dem Sprung liegen: dann bis zum rechten Locator.
                let end = if m.pos > s {
                    m.pos
                } else {
                    tempo.frames(self.right_locator, rate).max(s + 1)
                };
                notes.push((s, end, m.key, v, p));
            }
        }
        for (k, o) in open.iter().enumerate() {
            if let Some((s, v, p)) = *o {
                notes.push((s, stop.max(s + 1), k as u8, v, p));
            }
        }
        if notes.is_empty() && ctrls.is_empty() {
            return;
        }
        let bar = self.bar_ticks();
        let cycle = with_cycle && self.cycle && self.right_locator > self.left_locator;
        let (mut part_start, mut part_end) = if cycle {
            (self.left_locator, self.right_locator)
        } else {
            let first = notes
                .iter()
                .map(|n| n.0)
                .chain(ctrls.iter().map(|c| c.0))
                .min()
                .unwrap_or(start)
                .min(start);
            let last = notes
                .iter()
                .map(|n| n.1)
                .chain(ctrls.iter().map(|c| c.0 + 1))
                .max()
                .unwrap_or(stop)
                .max(stop);
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
        let mut take: Vec<studiocraft_project::TakeNote> = notes
            .into_iter()
            .map(
                |(a, b, key, velocity, pass)| studiocraft_project::TakeNote {
                    start: ticks(a),
                    end: ticks(b).max(ticks(a) + 1),
                    key,
                    velocity,
                    pass,
                    ctrl: None,
                },
            )
            .collect();
        take.extend(ctrls.into_iter().map(|(at, kind, value, pass)| {
            studiocraft_project::TakeNote {
                start: ticks(at),
                end: ticks(at),
                key: 0,
                velocity: 0,
                pass,
                ctrl: Some((kind, value)),
            }
        }));
        let name = format!(
            "{} {:02}",
            self.project.tracks[track].name,
            self.project.tracks[track].events.len() + 1
        );
        let before = self.project.clone();
        let mut settings = self.project.record;
        if !with_cycle {
            settings.midi = studiocraft_project::MidiRecordMode::NewPart;
        }
        let ids = self.project.place_midi_recording(
            track,
            &take,
            (part_start, part_end),
            cycle,
            settings,
            &name,
        );
        if ids.is_empty() {
            return;
        }
        self.selection = ids;
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

        let files: Vec<PathBuf> = rec.files().map(Path::to_path_buf).collect();
        for track in self.armed_tracks() {
            let inp = self.project.input_of(track);
            let track_name = sanitize(&self.project.tracks[track].name);
            // Je Aufnahmedatei ein Clip; `(clip, erster Frame, Frames)` in Aufnahme-Frames.
            let mut parts: Vec<(studiocraft_project::ClipId, u64, u64)> = Vec::new();
            let mut at = 0u64;
            for (i, path) in files.iter().enumerate() {
                let samples =
                    input::extract_file_channels(path, rec.channels, inp.first, inp.stereo)?;
                let length = samples.len() as u64 / 2;
                let suffix = if i == 0 {
                    String::new()
                } else {
                    format!("-{:03}", i + 1)
                };
                let file = unique_name(&dir, &format!("{stem} - {track_name}{suffix}.wav"));
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
                parts.push((id, at, length));
                at += length;
            }

            let base = self.project.tracks[track].name.clone();
            let existing = self.project.tracks[track].events.len();
            for (k, pass) in takes.iter().enumerate() {
                let name = format!("{base} {:02}", existing + k + 1);
                // Über mehrere Dateien: aufeinanderfolgende Events, die zusammen den Take bilden.
                let mut pieces = Vec::new();
                for &(id, first, length) in &parts {
                    let lo = pass.offset.max(first);
                    let hi = (pass.offset + pass.length).min(first + length);
                    if lo >= hi {
                        continue;
                    }
                    let from = pass.start + (lo - pass.offset);
                    let Some(ev) = self.project.place_take(
                        track,
                        tempo.ticks(from, rate),
                        id,
                        lo - first,
                        // Länge über das Ende, damit Tempowechsel im Take stimmen.
                        tempo
                            .ticks(from.saturating_add(hi - lo), rate)
                            .saturating_sub(tempo.ticks(from, rate))
                            .max(1),
                        &name,
                    ) else {
                        continue;
                    };
                    if !self.punch(ev) {
                        self.project.delete(ev);
                        continue;
                    }
                    pieces.push(ev);
                }
                // Mehrere Teile: als Audio-Part zusammengefasst, damit der Take ein Ganzes
                // bleibt (verschieben, kopieren, Takes vergleichen).
                if pieces.len() > 1
                    && let Some(part) = self.project.group_audio(&pieces)
                {
                    if let Some(e) = self.project.event_mut(part) {
                        e.name = name.clone();
                    }
                    pieces = vec![part];
                }
                if !pieces.is_empty() {
                    placed += 1;
                    self.selection = pieces;
                }
            }
        }
        // Die Mehrkanal-Rohdateien werden nicht mehr gebraucht.
        for path in &files {
            let _ = std::fs::remove_file(path);
        }
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

impl ProjectWindow {
    /// Verbleibende Zählzeiten des laufenden Vorzählers.
    pub(crate) fn count_in_beats(&self) -> Option<u64> {
        let left = self.transport.as_ref()?.status.count_in();
        if left == 0 || self.count_in_frames == 0 {
            return None;
        }
        let tempo = &self.project.tempo;
        let beats = u64::from(tempo.signature_at(self.cursor).numerator.max(1))
            * u64::from(self.project.click.precount_bars.max(1));
        let beat = (self.count_in_frames / beats).max(1);
        Some(left.div_ceil(beat))
    }

    /// Untermenü „Metronom“: Vorzähler, Lautstärke, Betonung und Klang (Projekteinstellung,
    /// rückgängig machbar).
    pub(crate) fn click_menu(&mut self, ui: &mut egui::Ui) {
        ui.menu_button(tr("Metronom"), |ui| {
            let mut c = self.project.click;
            ui.menu_button(tr("Vorzähler"), |ui| {
                for (bars, label) in [
                    (0u32, tr("Aus")),
                    (1, tr("1 Takt")),
                    (2, tr("2 Takte")),
                    (4, tr("4 Takte")),
                ] {
                    ui.radio_value(&mut c.precount_bars, bars, label);
                }
            });
            ui.horizontal(|ui| {
                let l = ui.label(tr("Lautstärke"));
                ui.add(
                    egui::Slider::new(
                        &mut c.level_db,
                        studiocraft_project::CLICK_DB.0..=studiocraft_project::CLICK_DB.1,
                    )
                    .suffix(" dB"),
                )
                .labelled_by(l.id);
            });
            ui.checkbox(&mut c.accent, tr("Takt-Eins betonen"));
            ui.menu_button(tr("Klang"), |ui| {
                for s in studiocraft_project::ClickSound::ALL {
                    ui.radio_value(&mut c.sound, s, tr(s.label()));
                }
            });
            if c != self.project.click {
                self.click_before
                    .get_or_insert_with(|| self.project.clone());
                self.project.click = c;
            }
            // Ein Undo-Schritt je Geste (Schieberegler: beim Loslassen).
            if ui.ctx().dragged_id().is_none()
                && let Some(before) = self.click_before.take()
            {
                self.checkpoint(before);
            }
        });
    }
}

/// Verwirft die ersten `count_in` Frames einer Aufnahme: (Startposition, übersprungene Frames)
/// vorher und nachher. Der erste Frame der Datei liegt bei `start - skip` (auch negativ).
pub(crate) fn drop_count_in(start: u64, skip: u64, count_in: u64) -> (u64, u64) {
    if count_in == 0 {
        return (start, skip);
    }
    let raw = i128::from(start) - i128::from(skip);
    // Erster behaltener Frame: nach dem Vorzähler und nicht vor Projektbeginn.
    let first = i128::from(count_in).max(-raw);
    let pos = raw + first;
    (
        u64::try_from(pos).unwrap_or(0),
        u64::try_from(first).unwrap_or(0),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_count_in_is_cut_off_the_take() {
        // No count-in: unchanged.
        assert_eq!(drop_count_in(1000, 0, 0), (1000, 0));
        // File starts 2 bars (192 000 frames) plus 480 frames latency before the cursor at
        // 200 000: the take keeps the latency compensation and loses the count-in.
        assert_eq!(
            drop_count_in(200_000 - 192_480, 0, 192_000),
            (199_520, 192_000)
        );
        // Near the project start the file begins before 0: start at 0, skip enough.
        assert_eq!(drop_count_in(0, 150_000, 96_000), (0, 150_000));
        assert_eq!(drop_count_in(0, 50_000, 96_000), (46_000, 96_000));
    }

    #[test]
    fn precount_shows_its_beats_and_stop_during_it_records_nothing() {
        let (mut core, ctl) =
            studiocraft_engine::engine(48_000, studiocraft_project::TempoMap::default());
        let mut w = ProjectWindow::new(
            studiocraft_project::Project::default(),
            Some(ctl),
            String::new(),
        );
        let t = w.project.add_track(TrackKind::Instrument, "Synth");
        w.project.tracks[t].record = true;
        w.project.click.precount_bars = 1;
        w.start_recording();
        let mut buf = vec![0.0f32; 2 * 1000];
        for _ in 0..10 {
            core.process(&mut buf);
        }
        // One bar of 4/4 at 120 BPM: 96 000 frames, beats of 24 000; 10 000 done.
        assert_eq!(w.count_in_beats(), Some(4));
        assert!(!w.is_playing());
        w.stop_transport();
        core.process(&mut buf);
        assert_eq!(
            w.message.as_deref(),
            Some("Aufnahme im Vorzähler abgebrochen")
        );
        assert!(w.project.tracks[t].events.is_empty());
        // Again, past the count-in: the transport records from the cursor.
        w.start_recording();
        for _ in 0..100 {
            core.process(&mut buf);
        }
        assert_eq!(w.count_in_beats(), None);
        assert!(w.transport.as_ref().unwrap().status.rec_started());
        assert!(w.is_playing(), "recording runs after the count-in");
        w.stop_transport();
        core.process(&mut buf);
    }

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
