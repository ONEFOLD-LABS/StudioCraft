//! Standard-MIDI-Dateien (SMF, Format 0 und 1) lesen und schreiben (Issue #47).
//!
//! Import: je Spur der Datei und MIDI-Kanal entsteht eine MIDI-Spur mit einem Part, der Noten,
//! Controller (CC, Pitchbend, Kanal-Aftertouch) und das erste Programm übernimmt. Tempo und
//! Taktarten der Datei lassen sich als Tempo-Map übernehmen. Export: Format 1 mit einer
//! Tempo-Spur und je MIDI- und Instrument-Spur einer Spur der Datei, 960 Ticks pro Viertel.
//!
//! Dateien gelten als fremd: alle Zugriffe sind geprüft, Größen und Mengen begrenzt.

use crate::sanitize::MAX_TICKS;
use crate::{
    CtrlKind, CtrlPoint, Event, Note, Position, Project, TICKS_PER_QUARTER, TempoMap,
    TimeSignature, TrackKind,
};

/// Größte Datei, die gelesen wird.
pub const MAX_FILE_BYTES: usize = 32 << 20;
/// Höchstzahl Spuren, die ein Import anlegt.
pub const MAX_IMPORT_TRACKS: usize = 256;
/// Höchstzahl Noten je importiertem Part.
pub const MAX_NOTES: usize = 200_000;
/// Höchstzahl Nachrichten, die aus einer Datei gelesen werden.
const MAX_MESSAGES: usize = 4_000_000;

/// Warum eine Datei nicht gelesen werden konnte.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SmfError {
    /// Keine MIDI-Datei (Kopf fehlt).
    NotMidi,
    /// Datei endet mitten in einem Abschnitt oder enthält ungültige Daten.
    Corrupt,
    /// Format 2 (unabhängige Sequenzen) wird nicht unterstützt.
    Format2,
    /// Zeitbasis in SMPTE-Frames wird nicht unterstützt.
    Smpte,
    /// Datei größer als [`MAX_FILE_BYTES`].
    TooLarge,
    /// Keine Noten oder Controller in der Datei bzw. keine MIDI-Spuren im Projekt.
    Empty,
}

impl std::fmt::Display for SmfError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(match self {
            Self::NotMidi => "keine MIDI-Datei",
            Self::Corrupt => "MIDI-Datei beschädigt",
            Self::Format2 => "MIDI-Format 2 wird nicht unterstützt",
            Self::Smpte => "SMPTE-Zeitbasis wird nicht unterstützt",
            Self::TooLarge => "MIDI-Datei zu groß",
            Self::Empty => "keine Noten",
        })
    }
}

impl std::error::Error for SmfError {}

/// Eine Spur aus der Datei: Noten und Controller eines Kanals, Positionen in Projekt-Ticks
/// ab Dateianfang.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct MidiTrack {
    pub name: String,
    /// Kanal 1–16.
    pub channel: u8,
    /// Erstes Programm (Program Change) des Kanals.
    pub program: Option<u8>,
    pub notes: Vec<Note>,
    pub controllers: Vec<CtrlPoint>,
}

impl MidiTrack {
    /// Ende der letzten Note oder des letzten Controllers in Ticks.
    pub fn end(&self) -> u64 {
        let notes = self.notes.iter().map(|n| n.end());
        let ctrls = self.controllers.iter().map(|c| c.start + 1);
        notes
            .chain(ctrls)
            .max()
            .map_or(0, |e| u64::try_from(e).unwrap_or(0))
    }
}

/// Inhalt einer MIDI-Datei.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct MidiFile {
    pub tracks: Vec<MidiTrack>,
    /// Tempo und Taktarten der Datei (120 BPM und 4/4, wenn sie keine angibt).
    pub tempo: TempoMap,
    /// Ob die Datei Tempo- oder Taktangaben enthält.
    pub has_tempo: bool,
}

/// Leser über den Bytes einer Datei.
struct Reader<'a> {
    data: &'a [u8],
    pos: usize,
}

impl<'a> Reader<'a> {
    fn byte(&mut self) -> Result<u8, SmfError> {
        let b = *self.data.get(self.pos).ok_or(SmfError::Corrupt)?;
        self.pos += 1;
        Ok(b)
    }

    fn take(&mut self, n: usize) -> Result<&'a [u8], SmfError> {
        let end = self.pos.checked_add(n).ok_or(SmfError::Corrupt)?;
        let s = self.data.get(self.pos..end).ok_or(SmfError::Corrupt)?;
        self.pos = end;
        Ok(s)
    }

    fn u32(&mut self) -> Result<u32, SmfError> {
        let s = self.take(4)?;
        Ok(u32::from_be_bytes([s[0], s[1], s[2], s[3]]))
    }

    /// Zahl variabler Länge (höchstens 4 Bytes, 28 Bit).
    fn var(&mut self) -> Result<u32, SmfError> {
        let mut v = 0u32;
        for _ in 0..4 {
            let b = self.byte()?;
            v = (v << 7) | u32::from(b & 0x7f);
            if b & 0x80 == 0 {
                return Ok(v);
            }
        }
        Err(SmfError::Corrupt)
    }

    fn done(&self) -> bool {
        self.pos >= self.data.len()
    }
}

/// Eine gelesene Nachricht.
#[derive(Debug, Clone, Copy)]
enum Msg {
    /// Kanal-Nachricht: Status (mit Kanal) und Datenbytes.
    Channel([u8; 3]),
    /// Mikrosekunden pro Viertel.
    Tempo(u32),
    /// Zähler und Nenner als Zweierpotenz.
    Signature(u8, u8),
}

struct RawTrack {
    name: Option<String>,
    /// (Tick der Datei, Nachricht), in Dateireihenfolge.
    msgs: Vec<(u64, Msg)>,
}

/// Datenbytes nach einem Kanal-Status.
fn data_len(status: u8) -> usize {
    match status & 0xf0 {
        0xc0 | 0xd0 => 1,
        _ => 2,
    }
}

fn read_track(chunk: &[u8], budget: &mut usize) -> Result<RawTrack, SmfError> {
    let mut r = Reader {
        data: chunk,
        pos: 0,
    };
    let mut track = RawTrack {
        name: None,
        msgs: Vec::new(),
    };
    let mut tick = 0u64;
    let mut running: Option<u8> = None;
    while !r.done() {
        tick = tick.saturating_add(u64::from(r.var()?));
        let first = r.byte()?;
        let (status, first_data) = if first & 0x80 != 0 {
            (first, None)
        } else {
            (running.ok_or(SmfError::Corrupt)?, Some(first))
        };
        match status {
            0x80..=0xef => {
                running = Some(status);
                let d1 = match first_data {
                    Some(d) => d,
                    None => r.byte()?,
                };
                let d2 = if data_len(status) == 2 { r.byte()? } else { 0 };
                if *budget == 0 {
                    continue;
                }
                *budget -= 1;
                track
                    .msgs
                    .push((tick, Msg::Channel([status, d1 & 0x7f, d2 & 0x7f])));
            }
            0xf0 | 0xf7 => {
                // System Exclusive: wird übersprungen.
                running = None;
                let len = r.var()? as usize;
                r.take(len)?;
            }
            0xff => {
                running = None;
                let kind = r.byte()?;
                let len = r.var()? as usize;
                let data = r.take(len)?;
                match kind {
                    0x2f => break,
                    0x03 if track.name.is_none() => {
                        let name = String::from_utf8_lossy(data);
                        let name: String = name.trim().chars().take(64).collect();
                        if !name.is_empty() {
                            track.name = Some(name);
                        }
                    }
                    0x51 if data.len() >= 3 => {
                        let us = u32::from_be_bytes([0, data[0], data[1], data[2]]);
                        track.msgs.push((tick, Msg::Tempo(us)));
                    }
                    0x58 if data.len() >= 2 => {
                        track.msgs.push((tick, Msg::Signature(data[0], data[1])));
                    }
                    _ => {}
                }
            }
            // System Common und Echtzeit gehören nicht in Dateien.
            _ => return Err(SmfError::Corrupt),
        }
    }
    Ok(track)
}

/// Ticks der Datei in Projekt-Ticks (gerundet, begrenzt).
fn scale(tick: u64, ppq: u16) -> u64 {
    let ppq = u128::from(ppq.max(1));
    let t = (u128::from(tick) * u128::from(TICKS_PER_QUARTER) + ppq / 2) / ppq;
    u64::try_from(t).unwrap_or(u64::MAX).min(MAX_TICKS)
}

fn as_i64(t: u64) -> i64 {
    i64::try_from(t).unwrap_or(i64::MAX)
}

/// Liest eine MIDI-Datei.
pub fn read(bytes: &[u8]) -> Result<MidiFile, SmfError> {
    if bytes.len() > MAX_FILE_BYTES {
        return Err(SmfError::TooLarge);
    }
    let mut r = Reader {
        data: bytes,
        pos: 0,
    };
    // RIFF-Hülle (RMID) überspringen: der MIDI-Teil beginnt mit „MThd“.
    let start = bytes
        .windows(4)
        .take(64)
        .position(|w| w == b"MThd")
        .ok_or(SmfError::NotMidi)?;
    r.pos = start + 4;
    let head_len = r.u32()? as usize;
    if head_len < 6 {
        return Err(SmfError::Corrupt);
    }
    let head = r.take(head_len)?;
    let format = u16::from_be_bytes([head[0], head[1]]);
    let division = u16::from_be_bytes([head[4], head[5]]);
    if format == 2 {
        return Err(SmfError::Format2);
    }
    if format > 2 {
        return Err(SmfError::Corrupt);
    }
    if division & 0x8000 != 0 {
        return Err(SmfError::Smpte);
    }
    if division == 0 {
        return Err(SmfError::Corrupt);
    }
    let mut budget = MAX_MESSAGES;
    let mut raw = Vec::new();
    while r.data.len().saturating_sub(r.pos) >= 8 {
        let id = r.take(4)?;
        let id = [id[0], id[1], id[2], id[3]];
        let len = r.u32()? as usize;
        // Ein abgeschnittener letzter Abschnitt zählt mit dem, was da ist.
        let avail = r.data.len() - r.pos;
        let chunk = r.take(len.min(avail))?;
        if &id == b"MTrk" {
            let t = read_track(chunk, &mut budget)?;
            // Nur Spuren mit Inhalt behalten (begrenzt die Arbeit bei sehr vielen Spuren).
            if !t.msgs.is_empty() && raw.len() < usize::from(u16::MAX) {
                raw.push(t);
            }
        }
    }
    Ok(convert(&raw, division))
}

/// Ordnet Noten und Controller nach Spur und Kanal, Tempo und Taktarten zur Tempo-Map.
fn convert(raw: &[RawTrack], ppq: u16) -> MidiFile {
    let mut out = MidiFile::default();
    // Tempo und Taktarten aus allen Spuren, nach Tick sortiert.
    let mut meta: Vec<(u64, Msg)> = raw
        .iter()
        .flat_map(|t| t.msgs.iter())
        .filter(|(_, m)| !matches!(m, Msg::Channel(_)))
        .map(|&(t, m)| (scale(t, ppq), m))
        .collect();
    meta.sort_by_key(|&(t, _)| t);
    meta.truncate(crate::sanitize::MAX_TEMPO_EVENTS);
    let mut tempo = TempoMap::default();
    for (tick, m) in &meta {
        out.has_tempo = true;
        match *m {
            Msg::Tempo(us) if us > 0 => {
                tempo.set_event(*tick, 60_000_000.0 / f64::from(us), false);
            }
            Msg::Signature(num, pow) => {
                if pow > 6 || num == 0 {
                    continue;
                }
                let sig = TimeSignature {
                    numerator: u32::from(num),
                    denominator: 1 << pow,
                }
                .sane();
                let (bar, start, _) = tempo.bar_at(*tick);
                // Ein Wechsel mitten im Takt gilt ab dem nächsten Takt.
                let bar = if *tick == start { bar } else { bar + 1 };
                tempo.set_signature(bar, sig);
            }
            _ => {}
        }
    }
    tempo.use_track = !tempo.events.is_empty();
    out.tempo = tempo;

    for (i, t) in raw.iter().enumerate() {
        if out.tracks.len() >= MAX_IMPORT_TRACKS {
            break;
        }
        if !t.msgs.iter().any(|(_, m)| matches!(m, Msg::Channel(_))) {
            continue;
        }
        let mut by_channel: [Option<MidiTrack>; 16] = Default::default();
        // Offene Noten je Kanal und Tonhöhe: (Start, Anschlag), älteste zuerst.
        let mut open: Vec<Vec<(u64, u8)>> = vec![Vec::new(); 16 * 128];
        let mut last = 0u64;
        for &(tick, m) in &t.msgs {
            let Msg::Channel(d) = m else { continue };
            let tick = scale(tick, ppq);
            last = last.max(tick);
            let ch = usize::from(d[0] & 0x0f);
            let slot = by_channel[ch].get_or_insert_with(|| MidiTrack {
                channel: ch as u8 + 1,
                ..MidiTrack::default()
            });
            match d[0] & 0xf0 {
                0x90 if d[2] > 0 => open[ch * 128 + usize::from(d[1])].push((tick, d[2])),
                0x80 | 0x90 => {
                    let stack = &mut open[ch * 128 + usize::from(d[1])];
                    if !stack.is_empty() {
                        let (start, velocity) = stack.remove(0);
                        if slot.notes.len() < MAX_NOTES {
                            slot.notes.push(Note {
                                pitch: d[1],
                                velocity,
                                start: as_i64(start),
                                length: (tick - start).max(1),
                            });
                        }
                    }
                }
                0xc0 => {
                    slot.program.get_or_insert(d[1]);
                }
                _ => {
                    if let Some((kind, value)) = CtrlKind::from_midi(d)
                        && slot.controllers.len() < crate::controllers::MAX_CTRL_POINTS
                    {
                        slot.controllers.push(CtrlPoint {
                            kind,
                            start: as_i64(tick),
                            value,
                        });
                    }
                }
            }
        }
        // Noten ohne Ende klingen bis zum Ende der Spur.
        for (key, stack) in open.iter().enumerate() {
            let ch = key / 128;
            for &(start, velocity) in stack {
                if let Some(slot) = &mut by_channel[ch]
                    && slot.notes.len() < MAX_NOTES
                {
                    slot.notes.push(Note {
                        pitch: (key % 128) as u8,
                        velocity,
                        start: as_i64(start),
                        length: last.saturating_sub(start).max(1),
                    });
                }
            }
        }
        let used: Vec<MidiTrack> = by_channel
            .into_iter()
            .flatten()
            .filter(|s| !s.notes.is_empty() || !s.controllers.is_empty())
            .collect();
        let several = used.len() > 1;
        let base = t.name.clone().unwrap_or_else(|| format!("MIDI {}", i + 1));
        for mut s in used {
            if out.tracks.len() >= MAX_IMPORT_TRACKS {
                break;
            }
            s.name = if several {
                format!("{base} {}", s.channel)
            } else {
                base.clone()
            };
            s.notes.sort_by_key(|n| (n.start, n.pitch));
            s.controllers.sort_by_key(|c| c.start);
            out.tracks.push(s);
        }
    }
    out
}

impl Project {
    /// Legt für jede Spur von `file` eine MIDI-Spur mit einem Part ab `start` an (Noten auf
    /// ihren Positionen der Datei). Mit `adopt_tempo` gilt danach die Tempo-Map der Datei.
    /// Gibt die neuen Spuren zurück.
    pub fn import_midi(&mut self, file: &MidiFile, start: u64, adopt_tempo: bool) -> Vec<usize> {
        if adopt_tempo && file.has_tempo {
            self.set_tempo_map(file.tempo.clone());
        }
        let start = start.min(MAX_TICKS);
        let mut added = Vec::new();
        for t in &file.tracks {
            let end = start.saturating_add(t.end()).min(MAX_TICKS);
            // Part bis zum Ende des Takts mit dem letzten Ereignis.
            let (bar, _, _) = self.tempo.bar_at(end.saturating_sub(1).max(start));
            let part_end = self.tempo.bar_start(bar + 1).max(start + 1);
            let i = self.add_track(TrackKind::Midi, t.name.clone());
            self.tracks[i].midi_channel = t.channel.clamp(1, 16);
            self.tracks[i].program = t.program.map(|p| p & 0x7f);
            let id = self.next_event_id();
            self.tracks[i].events.push(Event {
                id,
                start: Position(start),
                length: part_end - start,
                name: t.name.clone(),
                muted: false,
                clip: None,
                offset: 0,
                fade_in: 0,
                fade_out: 0,
                fade_in_curve: crate::FadeCurve::Linear,
                fade_out_curve: crate::FadeCurve::Linear,
                gain_db: 0.0,
                notes: t.notes.clone(),
                controllers: t.controllers.clone(),
                lane: None,
                comp: 0,
                color: None,
                shared: None,
                musical: None,
                stretch_mode: crate::StretchMode::Standard,
                warp: Vec::new(),
            });
            added.push(i);
        }
        added
    }

    /// Schreibt die MIDI- und Instrument-Spuren als MIDI-Datei (Format 1, 960 Ticks pro
    /// Viertel). Stumme Parts und ausgeblendete Noten fehlen; Noten über das Part-Ende hinaus
    /// werden dort beendet.
    pub fn export_midi(&self) -> Result<Vec<u8>, SmfError> {
        let tracks: Vec<usize> = self
            .display_order()
            .into_iter()
            .map(|(i, _)| i)
            .filter(|&i| matches!(self.tracks[i].kind, TrackKind::Midi | TrackKind::Instrument))
            .collect();
        if tracks.is_empty() {
            return Err(SmfError::Empty);
        }
        let mut out = Vec::new();
        out.extend_from_slice(b"MThd");
        out.extend_from_slice(&6u32.to_be_bytes());
        out.extend_from_slice(&1u16.to_be_bytes());
        let count = u16::try_from(tracks.len() + 1).unwrap_or(u16::MAX);
        out.extend_from_slice(&count.to_be_bytes());
        out.extend_from_slice(&(TICKS_PER_QUARTER as u16).to_be_bytes());
        write_chunk(&mut out, self.tempo_messages());
        for &i in tracks.iter().take(usize::from(u16::MAX) - 1) {
            write_chunk(&mut out, self.track_messages(i));
        }
        Ok(out)
    }

    /// Tempo-Spur: Taktarten und Tempo; Rampen als Stufen je Sechzehntel.
    fn tempo_messages(&self) -> Vec<(u64, u8, Vec<u8>)> {
        let tm = &self.tempo;
        let mut msgs = Vec::new();
        let sig = |s: TimeSignature| {
            let s = s.sane();
            let pow = s.denominator.trailing_zeros() as u8;
            vec![0xff, 0x58, 4, s.numerator as u8, pow, 24, 8]
        };
        msgs.push((0, 0, sig(tm.signature)));
        for e in &tm.signatures {
            msgs.push((tm.bar_start(e.bar), 0, sig(e.signature())));
        }
        let tempo = |bpm: f64| {
            let us =
                (60_000_000.0 / bpm.clamp(crate::BPM_RANGE.0, crate::BPM_RANGE.1)).round() as u32;
            let b = us.min(0xff_ffff).to_be_bytes();
            vec![0xff, 0x51, 3, b[1], b[2], b[3]]
        };
        msgs.push((0, 1, tempo(tm.bpm_at(0))));
        if tm.use_track {
            let step = TICKS_PER_QUARTER / 4;
            let mut prev = 0u64;
            for e in &tm.events {
                if e.ramp {
                    let mut t = prev + step;
                    while t < e.tick && msgs.len() < 200_000 {
                        msgs.push((t, 1, tempo(tm.bpm_at(t))));
                        t += step;
                    }
                }
                msgs.push((e.tick, 1, tempo(e.bpm)));
                prev = e.tick;
            }
        }
        msgs
    }

    /// Nachrichten einer Spur: Name, Programm, Controller und Noten.
    fn track_messages(&self, i: usize) -> Vec<(u64, u8, Vec<u8>)> {
        let t = &self.tracks[i];
        let ch = t.midi_channel.clamp(1, 16) - 1;
        let mut msgs = Vec::new();
        let name: Vec<u8> = t.name.bytes().take(127).collect();
        let mut meta = vec![0xff, 0x03, name.len() as u8];
        meta.extend_from_slice(&name);
        msgs.push((0, 0, meta));
        if let Some(p) = t.program {
            msgs.push((0, 1, vec![0xc0 | ch, p & 0x7f]));
        }
        for e in t.events.iter().filter(|e| !e.muted) {
            let len = as_i64(e.length);
            let at = |rel: i64| e.start.0.saturating_add(rel.unsigned_abs());
            for c in &e.controllers {
                if (0..len).contains(&c.start) {
                    let [s, a, b] = c.kind.midi(c.value);
                    let data = if data_len(s) == 1 {
                        vec![s | ch, a]
                    } else {
                        vec![s | ch, a, b]
                    };
                    msgs.push((at(c.start), 2, data));
                }
            }
            for n in &e.notes {
                let (a, b) = (n.start.max(0), n.end().min(len));
                if b <= a {
                    continue;
                }
                let p = n.pitch & 0x7f;
                msgs.push((at(a), 3, vec![0x90 | ch, p, n.velocity.clamp(1, 127)]));
                // Note-Off vor einem Note-On am selben Tick.
                msgs.push((at(b), 1, vec![0x80 | ch, p, 0]));
            }
        }
        msgs
    }
}

/// Schreibt eine Spur: Nachrichten nach (Tick, Rang) sortiert, mit End-of-Track.
fn write_chunk(out: &mut Vec<u8>, mut msgs: Vec<(u64, u8, Vec<u8>)>) {
    msgs.sort_by_key(|(t, rank, _)| (*t, *rank));
    let mut body = Vec::new();
    let mut now = 0u64;
    for (t, _, data) in msgs {
        write_var(&mut body, t - now);
        body.extend_from_slice(&data);
        now = t;
    }
    write_var(&mut body, 0);
    body.extend_from_slice(&[0xff, 0x2f, 0]);
    out.extend_from_slice(b"MTrk");
    out.extend_from_slice(&u32::try_from(body.len()).unwrap_or(u32::MAX).to_be_bytes());
    out.extend_from_slice(&body);
}

/// Zahl variabler Länge; größere Abstände als 28 Bit werden über leere Schritte verteilt.
fn write_var(out: &mut Vec<u8>, mut v: u64) {
    const MAX: u64 = 0x0fff_ffff;
    while v > MAX {
        // Ein Abstand mit einer wirkungslosen Meta-Nachricht (Text der Länge 0).
        push_var(out, MAX as u32);
        out.extend_from_slice(&[0xff, 0x01, 0]);
        v -= MAX;
    }
    push_var(out, v as u32);
}

fn push_var(out: &mut Vec<u8>, v: u32) {
    let mut buf = [0u8; 4];
    let mut n = 0;
    let mut x = v;
    loop {
        buf[n] = (x & 0x7f) as u8;
        n += 1;
        x >>= 7;
        if x == 0 {
            break;
        }
    }
    for (k, b) in buf[..n].iter().enumerate().rev() {
        out.push(b | if k > 0 { 0x80 } else { 0 });
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Eine Datei aus einer Kopfzeile und Spuren (Rohbytes ohne End-of-Track).
    fn smf(format: u16, ppq: u16, tracks: &[&[u8]]) -> Vec<u8> {
        let mut out = b"MThd".to_vec();
        out.extend_from_slice(&6u32.to_be_bytes());
        out.extend_from_slice(&format.to_be_bytes());
        out.extend_from_slice(&(tracks.len() as u16).to_be_bytes());
        out.extend_from_slice(&ppq.to_be_bytes());
        for t in tracks {
            out.extend_from_slice(b"MTrk");
            out.extend_from_slice(&(t.len() as u32 + 4).to_be_bytes());
            out.extend_from_slice(t);
            out.extend_from_slice(&[0, 0xff, 0x2f, 0]);
        }
        out
    }

    #[test]
    fn reads_notes_with_running_status_and_scales_ticks() {
        // 480 Ticks pro Viertel; Note-Off als Note-On mit Anschlag 0 (Running Status).
        let track: &[u8] = &[
            0x00, 0xff, 0x03, 4, b'L', b'e', b'a', b'd', // Name
            0x00, 0xc2, 5, // Programm 5 auf Kanal 3
            0x00, 0x92, 60, 100, // Note an
            0x83, 0x60, 60, 0, // 480 später: aus (Running Status)
            0x00, 64, 90, // gleich wieder: E4 an
            0x81, 0x70, 0x82, 64, 0, // 240 später: aus per Note-Off
        ];
        let f = read(&smf(0, 480, &[track])).unwrap();
        assert_eq!(f.tracks.len(), 1);
        let t = &f.tracks[0];
        assert_eq!(
            (t.name.as_str(), t.channel, t.program),
            ("Lead", 3, Some(5))
        );
        assert_eq!(
            t.notes,
            vec![
                Note {
                    pitch: 60,
                    velocity: 100,
                    start: 0,
                    length: 960
                },
                Note {
                    pitch: 64,
                    velocity: 90,
                    start: 960,
                    length: 480
                },
            ]
        );
        assert!(!f.has_tempo);
    }

    #[test]
    fn format_0_splits_channels_and_reads_controllers() {
        let track: &[u8] = &[
            0x00, 0x90, 36, 100, // Kanal 1
            0x00, 0x99, 42, 80, // Kanal 10
            0x00, 0xb0, 7, 90, // CC 7 auf Kanal 1
            0x00, 0xe0, 0, 0x40, // Pitchbend Mitte
            0x60, 0x80, 36, 0, 0x00, 0x89, 42, 0,
        ];
        let f = read(&smf(0, 96, &[track])).unwrap();
        assert_eq!(f.tracks.len(), 2);
        assert_eq!(f.tracks[0].name, "MIDI 1 1");
        assert_eq!(f.tracks[1].channel, 10);
        assert_eq!(f.tracks[1].notes[0].length, 960);
        let kinds: Vec<_> = f.tracks[0]
            .controllers
            .iter()
            .map(|c| (c.kind, c.value))
            .collect();
        assert_eq!(
            kinds,
            vec![(CtrlKind::Cc(7), 90), (CtrlKind::PitchBend, 8192)]
        );
    }

    #[test]
    fn reads_tempo_and_signatures_from_the_conductor_track() {
        let conductor: &[u8] = &[
            0x00, 0xff, 0x58, 4, 3, 2, 24, 8, // 3/4
            0x00, 0xff, 0x51, 3, 0x07, 0xa1, 0x20, // 500000 µs = 120 BPM
            // Nach zwei 3/4-Takten (2 × 3 × 96 = 576 Ticks): 90 BPM und 6/8
            0x84, 0x40, 0xff, 0x51, 3, 0x0a, 0x2c, 0x2b, 0x00, 0xff, 0x58, 4, 6, 3, 24, 8,
        ];
        let notes: &[u8] = &[0x00, 0x90, 60, 100, 0x60, 0x80, 60, 0];
        let f = read(&smf(1, 96, &[conductor, notes])).unwrap();
        assert!(f.has_tempo);
        assert_eq!(f.tracks.len(), 1);
        let tm = &f.tempo;
        assert_eq!(
            tm.signature,
            TimeSignature {
                numerator: 3,
                denominator: 4
            }
        );
        assert!((tm.bpm - 120.0).abs() < 1e-9);
        assert!(tm.use_track);
        assert_eq!(tm.events[0].tick, 5760);
        assert!((tm.events[0].bpm - 90.0).abs() < 0.01);
        assert_eq!(tm.signatures[0].bar, 3);
        assert_eq!(
            tm.signatures[0].signature(),
            TimeSignature {
                numerator: 6,
                denominator: 8
            }
        );
    }

    #[test]
    fn hostile_files_are_rejected_without_panicking() {
        assert_eq!(read(b""), Err(SmfError::NotMidi));
        assert_eq!(read(b"RIFF1234"), Err(SmfError::NotMidi));
        assert_eq!(read(&smf(2, 96, &[])), Err(SmfError::Format2));
        assert_eq!(read(&smf(1, 0xe728, &[])), Err(SmfError::Smpte));
        assert_eq!(read(&smf(1, 0, &[])), Err(SmfError::Corrupt));
        // Running Status ohne vorigen Status, zu lange Zahl, System Common.
        assert_eq!(
            read(&smf(0, 96, &[&[0x00, 60, 100]])),
            Err(SmfError::Corrupt)
        );
        assert_eq!(
            read(&smf(0, 96, &[&[0xff, 0xff, 0xff, 0xff, 0x7f]])),
            Err(SmfError::Corrupt)
        );
        assert_eq!(
            read(&smf(0, 96, &[&[0x00, 0xf2, 0, 0]])),
            Err(SmfError::Corrupt)
        );
        // Meta-Nachricht, die länger ist als die Spur.
        assert_eq!(
            read(&smf(0, 96, &[&[0x00, 0xff, 0x01, 0x7f]])),
            Err(SmfError::Corrupt)
        );
        // Abgeschnittene Datei: was lesbar ist, zählt nicht als Absturz.
        let mut cut = smf(0, 96, &[&[0x00, 0x90, 60, 100, 0x60, 0x80, 60, 0]]);
        cut.truncate(cut.len() - 6);
        let _ = read(&cut);
        // Jede Kürzung und jedes geänderte Byte einer gültigen Datei: kein Absturz.
        let good = smf(
            1,
            96,
            &[&[
                0x00, 0xff, 0x51, 3, 7, 0xa1, 0x20, 0x00, 0x90, 60, 100, 0x60, 0x80, 60, 0,
            ]],
        );
        for n in 0..good.len() {
            let _ = read(&good[..n]);
            for v in [0x00, 0x7f, 0x80, 0xff] {
                let mut b = good.clone();
                b[n] = v;
                let _ = read(&b);
            }
        }
    }

    #[test]
    fn unterminated_notes_end_with_the_track() {
        let track: &[u8] = &[0x00, 0x90, 60, 100, 0x60, 0x90, 62, 100, 0x60, 0xb0, 1, 3];
        let f = read(&smf(0, 96, &[track])).unwrap();
        let ends: Vec<_> = f.tracks[0].notes.iter().map(|n| n.end()).collect();
        assert_eq!(ends, vec![1920, 1920]);
    }

    #[test]
    fn import_creates_midi_tracks_with_bar_long_parts() {
        let track: &[u8] = &[
            0x00, 0xff, 0x03, 4, b'B', b'a', b's', b's', 0x00, 0xc0, 33, 0x00, 0x90, 40, 100, 0x81,
            0x40, 0x80, 40, 0,
        ];
        let f = read(&smf(0, 96, &[track])).unwrap();
        let mut p = Project::default();
        let added = p.import_midi(&f, 3840, true);
        assert_eq!(added, vec![0]);
        let t = &p.tracks[0];
        assert_eq!(
            (t.kind, t.name.as_str(), t.program),
            (TrackKind::Midi, "Bass", Some(33))
        );
        let e = &t.events[0];
        assert_eq!((e.start.0, e.length), (3840, 3840));
        assert_eq!(e.notes[0].length, 1920);
        // Ohne Tempo in der Datei bleibt das Projekttempo.
        assert_eq!(p.tempo, TempoMap::default());
    }

    #[test]
    fn export_then_import_round_trips() {
        let mut p = Project::default();
        p.tempo.bpm = 100.0;
        p.tempo.set_signature(
            2,
            TimeSignature {
                numerator: 7,
                denominator: 8,
            },
        );
        p.tempo.set_event(7680, 140.0, false);
        p.tempo.use_track = true;
        let a = p.add_track(TrackKind::Instrument, "Keys");
        let b = p.add_track(TrackKind::Midi, "Drums");
        p.add_track(TrackKind::Audio, "Vox");
        p.tracks[b].midi_channel = 10;
        p.tracks[b].program = Some(3);
        let ev = p.create_event(a, 960, 3840).unwrap();
        let e = p.event_mut(ev).unwrap();
        e.notes = vec![
            Note {
                pitch: 60,
                velocity: 100,
                start: 0,
                length: 480,
            },
            // Ragt über das Part-Ende hinaus: endet dort.
            Note {
                pitch: 67,
                velocity: 80,
                start: 3600,
                length: 960,
            },
            // Vor dem Part-Anfang ausgeblendet: fehlt.
            Note {
                pitch: 70,
                velocity: 80,
                start: -480,
                length: 240,
            },
        ];
        e.controllers = vec![CtrlPoint {
            kind: CtrlKind::Cc(1),
            start: 240,
            value: 64,
        }];
        let ev = p.create_event(b, 0, 960).unwrap();
        p.event_mut(ev).unwrap().notes = vec![Note {
            pitch: 36,
            velocity: 127,
            start: 0,
            length: 120,
        }];
        let muted = p.create_event(b, 1920, 960).unwrap();
        let m = p.event_mut(muted).unwrap();
        m.muted = true;
        m.notes = vec![Note {
            pitch: 38,
            velocity: 127,
            start: 0,
            length: 120,
        }];

        let bytes = p.export_midi().unwrap();
        let f = read(&bytes).unwrap();
        assert!((f.tempo.bpm - 100.0).abs() < 0.01);
        assert_eq!(f.tempo.signatures, p.tempo.signatures);
        assert_eq!(f.tempo.events[0].tick, 7680);
        assert_eq!(f.tracks.len(), 2);
        let keys = &f.tracks[0];
        assert_eq!((keys.name.as_str(), keys.channel), ("Keys", 1));
        let notes: Vec<_> = keys
            .notes
            .iter()
            .map(|n| (n.pitch, n.start, n.length))
            .collect();
        assert_eq!(notes, vec![(60, 960, 480), (67, 4560, 240)]);
        assert_eq!(keys.controllers[0].start, 1200);
        let drums = &f.tracks[1];
        assert_eq!((drums.channel, drums.program), (10, Some(3)));
        assert_eq!(drums.notes.len(), 1);
    }

    #[test]
    fn export_needs_a_midi_track() {
        let mut p = Project::default();
        p.add_track(TrackKind::Audio, "Vox");
        assert_eq!(p.export_midi(), Err(SmfError::Empty));
    }

    #[test]
    fn tempo_ramps_are_written_as_steps() {
        let mut p = Project::default();
        p.add_track(TrackKind::Midi, "M");
        p.tempo.set_event(3840, 180.0, true);
        p.tempo.use_track = true;
        let f = read(&p.export_midi().unwrap()).unwrap();
        // Je Sechzehntel ein Schritt bis zum Zielpunkt.
        assert_eq!(f.tempo.events.len(), 3840 / 240);
        assert!((f.tempo.events.last().unwrap().bpm - 180.0).abs() < 0.01);
    }

    #[test]
    fn variable_length_numbers() {
        for (v, bytes) in [
            (0u32, vec![0x00]),
            (0x7f, vec![0x7f]),
            (0x80, vec![0x81, 0x00]),
            (0x0fff_ffff, vec![0xff, 0xff, 0xff, 0x7f]),
        ] {
            let mut out = Vec::new();
            push_var(&mut out, v);
            assert_eq!(out, bytes);
            let mut r = Reader { data: &out, pos: 0 };
            assert_eq!(r.var().unwrap(), v);
        }
    }
}
