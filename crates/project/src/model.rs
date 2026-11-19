//! Spuren und Events (Spec-Abschnitte „Begriffe und Datenmodell“ und „Spurtypen“).

use crate::{Position, TICKS_PER_QUARTER, TempoMap};

pub type EventId = u64;
/// Index eines Clips im Pool.
pub type ClipId = u32;

/// Eintrag im Pool: eine Audio-Datei (Spec: „Pool“, „Audio-Clip“).
/// Die Samples selbst hält die Engine; das Modell kennt nur die Eckdaten.
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct PoolClip {
    pub name: String,
    pub path: Option<String>,
    /// Abtastrate der geladenen Samples.
    pub sample_rate: u32,
    pub frames: u64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub enum TrackKind {
    Audio,
    Instrument,
    Midi,
    Group,
    Fx,
    Folder,
}

impl TrackKind {
    pub fn label(self) -> &'static str {
        match self {
            Self::Audio => "Audio",
            Self::Instrument => "Instrument",
            Self::Midi => "MIDI",
            Self::Group => "Gruppe",
            Self::Fx => "FX-Kanal",
            Self::Folder => "Ordner",
        }
    }

    /// Ob auf dieser Spur Events liegen können.
    pub fn holds_events(self) -> bool {
        matches!(self, Self::Audio | Self::Instrument | Self::Midi)
    }
}

/// Ein Event auf einer Spur. Positionen und Längen in Ticks.
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct Event {
    pub id: EventId,
    pub start: Position,
    pub length: u64,
    pub name: String,
    pub muted: bool,
    /// Audio-Events: Clip im Pool. `None` bei MIDI-Parts und leeren Events.
    pub clip: Option<ClipId>,
    /// Audio-Events: Startpunkt im Clip in Frames des Clips.
    pub offset: u64,
    /// Fade-In und Fade-Out in Ticks (nicht-destruktiv, linear).
    pub fade_in: u64,
    pub fade_out: u64,
    /// MIDI-Parts: Noten, Position relativ zum Part-Anfang.
    #[serde(default)]
    pub notes: Vec<Note>,
}

/// Eine MIDI-Note in einem Part (Spec „MIDI-Part“: Noten relativ zum Part-Start).
/// `start` darf negativ sein oder hinter dem Part-Ende liegen: Beim Kürzen eines
/// Parts bleiben Noten erhalten und sind nur ausgeblendet, wie in klassischen DAWs.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct Note {
    pub pitch: u8,
    pub velocity: u8,
    pub start: i64,
    pub length: u64,
}

impl Note {
    pub fn end(&self) -> i64 {
        self.start + self.length as i64
    }
}

impl Event {
    pub fn end(&self) -> u64 {
        self.start.0 + self.length
    }

    pub fn contains(&self, ticks: u64) -> bool {
        (self.start.0..self.end()).contains(&ticks)
    }
}

#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct Track {
    /// Feste Kennung (für Routing-Ziele); bleibt beim Umsortieren gleich.
    #[serde(default)]
    pub id: TrackId,
    pub name: String,
    pub kind: TrackKind,
    pub color: [u8; 3],
    pub mute: bool,
    pub solo: bool,
    pub record: bool,
    /// Eingang mithören (Audio-Spuren): das Eingangssignal läuft durch den Kanal der Spur.
    #[serde(default)]
    pub monitor: bool,
    /// Automation lesen (R) und schreiben (W); wirken, sobald es Automationsspuren gibt.
    #[serde(default)]
    pub read: bool,
    #[serde(default)]
    pub write: bool,
    /// Fader in dB; [`MIN_DB`] und darunter gilt als −∞.
    pub volume_db: f32,
    /// Panorama: −1 links, 0 Mitte, +1 rechts (Stereo-Balance).
    pub pan: f32,
    /// Aufnahme-Eingang (Audio-Spuren).
    #[serde(default)]
    pub input: TrackInput,
    /// Insert-Effekte in Signalfluss-Reihenfolge.
    #[serde(default)]
    pub inserts: Vec<InsertSlot>,
    /// Instrument-Spuren: das Instrument-Plugin.
    #[serde(default)]
    pub instrument: Option<InsertSlot>,
    /// Ausgang: `None` = Summe (Ausgangsbus), sonst eine Gruppen-Spur.
    #[serde(default)]
    pub output: Option<TrackId>,
    /// Sends an FX-Kanäle oder Gruppen (Spec: Sends 1–8).
    #[serde(default)]
    pub sends: Vec<SendSlot>,
    pub events: Vec<Event>,
}

/// Kennung einer Spur.
pub type TrackId = u32;

/// Ein Send: ein Teil des Signals an einen FX-Kanal oder eine Gruppe.
#[derive(Debug, Clone, Copy, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct SendSlot {
    pub target: TrackId,
    pub level_db: f32,
    /// Vor dem Fader abgreifen (sonst danach).
    pub pre_fader: bool,
    pub on: bool,
}

/// Ein Insert-Effekt: welches Plugin, ob umgangen, gesetzte Parameter.
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct InsertSlot {
    /// CLAP-Plugin-Id, z. B. `in.lsp-plug.para_equalizer_x8_stereo`.
    pub plugin_id: String,
    /// Anzeigename (auch wenn das Plugin fehlt).
    pub name: String,
    pub bypass: bool,
    /// Vom Standard abweichende Parameter: (Id, Wert).
    pub params: Vec<(u32, f64)>,
    /// Kompletter Plugin-Zustand (`clap.state`), Base64. Wird beim Speichern erfasst.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub state: Option<String>,
}

impl InsertSlot {
    pub fn set_param(&mut self, id: u32, value: f64) {
        match self.params.iter_mut().find(|(i, _)| *i == id) {
            Some(p) => p.1 = value,
            None => self.params.push((id, value)),
        }
    }

    /// Gleiches Plugin an gleicher Stelle (Parameter und Bypass dürfen abweichen).
    pub fn same_plugin(&self, other: &Self) -> bool {
        self.plugin_id == other.plugin_id
    }
}

/// Welche Eingangskanäle eine Audio-Spur aufnimmt.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct TrackInput {
    /// Erster Kanal, ab 0.
    pub first: u16,
    /// `true`: `first` und `first + 1` als Stereo, sonst Mono.
    pub stereo: bool,
}

impl Default for TrackInput {
    fn default() -> Self {
        Self {
            first: 0,
            stereo: true,
        }
    }
}

impl TrackInput {
    /// Anzeige wie „Eingang 1/2“ oder „Eingang 2“ (Kanäle ab 1 gezählt).
    pub fn label(self) -> String {
        if self.stereo {
            format!("Eingang {}/{}", self.first + 1, self.first + 2)
        } else {
            format!("Eingang {}", self.first + 1)
        }
    }

    /// Alle sinnvollen Eingänge bei `channels` Gerätekanälen: Stereopaare, dann Mono.
    pub fn options(channels: u16) -> Vec<Self> {
        let pairs = (0..channels / 2).map(|i| Self {
            first: 2 * i,
            stereo: true,
        });
        let monos = (0..channels).map(|c| Self {
            first: c,
            stereo: false,
        });
        pairs.chain(monos).collect()
    }
}

/// Untere Grenze des Faders; darunter ist der Kanal stumm (−∞).
pub const MIN_DB: f32 = -60.0;
/// Obere Grenze des Faders.
pub const MAX_DB: f32 = 6.0;

/// dB in linearen Faktor; [`MIN_DB`] und darunter ergibt 0.
pub fn db_to_gain(db: f32) -> f32 {
    if db <= MIN_DB {
        0.0
    } else {
        10f32.powf(db / 20.0)
    }
}

impl Track {
    /// Faktoren für links und rechts aus Fader und Stereo-Balance.
    pub fn gains(&self) -> (f32, f32) {
        let g = db_to_gain(self.volume_db);
        let p = self.pan.clamp(-1.0, 1.0);
        let l = if p > 0.0 { 1.0 - p } else { 1.0 };
        let r = if p < 0.0 { 1.0 + p } else { 1.0 };
        (g * l, g * r)
    }
}

#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct Project {
    pub tracks: Vec<Track>,
    pub tempo: TempoMap,
    pub pool: Vec<PoolClip>,
    /// Fader des Ausgangsbusses in dB.
    pub master_db: f32,
    next_id: EventId,
    #[serde(default)]
    next_track_id: TrackId,
}

const PALETTE: [[u8; 3]; 8] = [
    [214, 84, 84],
    [222, 150, 60],
    [204, 190, 70],
    [96, 180, 96],
    [70, 170, 170],
    [80, 130, 214],
    [150, 100, 214],
    [200, 90, 160],
];

impl Default for Project {
    fn default() -> Self {
        Self {
            tracks: Vec::new(),
            pool: Vec::new(),
            master_db: 0.0,
            tempo: TempoMap::default(),
            next_id: 1,
            next_track_id: 1,
        }
    }
}

impl Project {
    fn new_track_id(&mut self) -> TrackId {
        self.next_track_id = self.next_track_id.max(1);
        let id = self.next_track_id;
        self.next_track_id += 1;
        id
    }

    /// Fehlende oder doppelte Spur-Kennungen vergeben (ältere Dateien).
    pub fn ensure_track_ids(&mut self) {
        let mut seen = std::collections::HashSet::new();
        self.next_track_id = self
            .next_track_id
            .max(self.tracks.iter().map(|t| t.id).max().unwrap_or(0) + 1);
        for i in 0..self.tracks.len() {
            if self.tracks[i].id == 0 || !seen.insert(self.tracks[i].id) {
                self.tracks[i].id = self.new_track_id();
                seen.insert(self.tracks[i].id);
            }
        }
    }

    pub fn track_index(&self, id: TrackId) -> Option<usize> {
        self.tracks.iter().position(|t| t.id == id)
    }

    /// Ob eine Spur ein Bus ist (Gruppe oder FX-Kanal): Ziel für Sends bzw. Ausgänge.
    pub fn is_bus(kind: TrackKind) -> bool {
        matches!(kind, TrackKind::Group | TrackKind::Fx)
    }

    /// Ob ein Signal von `from` nach `to` geführt werden darf, ohne eine
    /// Rückkopplungsschleife zu bilden (über Ausgänge und Sends).
    pub fn can_route(&self, from: TrackId, to: TrackId) -> bool {
        if from == to {
            return false;
        }
        // Erreicht `to` (über seine Wege) wieder `from`? Dann Schleife.
        let mut stack = vec![to];
        let mut seen = std::collections::HashSet::new();
        while let Some(id) = stack.pop() {
            if id == from {
                return false;
            }
            if !seen.insert(id) {
                continue;
            }
            if let Some(t) = self.track_index(id).map(|i| &self.tracks[i]) {
                stack.extend(t.output);
                stack.extend(t.sends.iter().filter(|s| s.on).map(|s| s.target));
            }
        }
        true
    }

    pub fn next_event_id(&mut self) -> EventId {
        let id = self.next_id;
        self.next_id += 1;
        id
    }

    /// Demo-Projekt mit `n` Spuren und verteilten Events, für Prototyp und Tests.
    pub fn demo(n: usize) -> Self {
        let mut p = Self::default();
        let bar = p.tempo.signature.bar_ticks();
        for i in 0..n {
            let kind = match i % 7 {
                0..=2 => TrackKind::Audio,
                3 | 4 => TrackKind::Instrument,
                5 => TrackKind::Midi,
                _ => TrackKind::Group,
            };
            let mut track = Track {
                id: 0,
                name: format!("{} {}", kind.label(), i + 1),
                kind,
                color: PALETTE[i % PALETTE.len()],
                mute: false,
                solo: false,
                record: false,
                monitor: false,
                read: false,
                write: false,
                volume_db: 0.0,
                pan: 0.0,
                input: TrackInput::default(),
                inserts: Vec::new(),
                instrument: None,
                output: None,
                sends: Vec::new(),
                events: Vec::new(),
            };
            if kind.holds_events() {
                // Deterministisch verteilte Events: 1–4 Takte lang, mit Lücken.
                let mut at = (i as u64 % 4) * bar;
                let mut k = 0u64;
                while at < 64 * bar {
                    let len = (1 + (i as u64 + k) % 4) * bar;
                    let id = p.next_event_id();
                    track.events.push(Event {
                        id,
                        start: Position(at),
                        length: len,
                        name: format!("{} #{}", kind.label(), k + 1),
                        muted: false,
                        clip: None,
                        offset: 0,
                        fade_in: 0,
                        fade_out: 0,
                        notes: Vec::new(),
                    });
                    at += len + ((i as u64 + 2 * k) % 3) * bar;
                    k += 1;
                }
            }
            track.id = p.new_track_id();
            p.tracks.push(track);
        }
        p
    }

    pub fn event_at(&self, track: usize, ticks: u64) -> Option<EventId> {
        self.tracks
            .get(track)?
            .events
            .iter()
            .rev()
            .find(|e| e.contains(ticks))
            .map(|e| e.id)
    }

    pub fn find(&self, id: EventId) -> Option<(usize, usize)> {
        self.tracks
            .iter()
            .enumerate()
            .find_map(|(t, track)| track.events.iter().position(|e| e.id == id).map(|i| (t, i)))
    }

    pub fn event(&self, id: EventId) -> Option<&Event> {
        let (t, i) = self.find(id)?;
        Some(&self.tracks[t].events[i])
    }

    pub fn event_mut(&mut self, id: EventId) -> Option<&mut Event> {
        let (t, i) = self.find(id)?;
        Some(&mut self.tracks[t].events[i])
    }

    /// Fügt eine Kopie von `ev` mit neuer Id auf Spur `track` ein (Einfügen aus der
    /// Zwischenablage). `None`, wenn die Spur fehlt oder keine Events trägt.
    pub fn insert_event(&mut self, track: usize, mut ev: Event) -> Option<EventId> {
        if !self.tracks.get(track)?.kind.holds_events() {
            return None;
        }
        ev.id = self.next_event_id();
        let id = ev.id;
        let events = &mut self.tracks[track].events;
        let pos = events.partition_point(|e| e.start.0 <= ev.start.0);
        events.insert(pos, ev);
        Some(id)
    }

    /// Teilt ein Event an `at`. Gibt die Id des neuen rechten Teils zurück.
    pub fn split(&mut self, id: EventId, at: u64) -> Option<EventId> {
        let new_id = self.next_id;
        let (t, i) = self.find(id)?;
        let ev = &mut self.tracks[t].events[i];
        if at <= ev.start.0 || at >= ev.end() {
            return None;
        }
        let mut right = ev.clone();
        right.id = new_id;
        if let Some(c) = ev.clip.and_then(|c| self.pool.get(c as usize)) {
            right.offset = ev.offset + self.tempo.frames(at - ev.start.0, c.sample_rate);
        }
        right.start = Position(at);
        right.length = ev.end() - at;
        ev.length = at - ev.start.0;
        // Noten: was ab dem Schnitt beginnt, wandert in den rechten Part.
        let cut = (at - ev.start.0) as i64;
        right.notes = ev
            .notes
            .iter()
            .filter(|n| n.start >= cut)
            .map(|n| Note {
                start: n.start - cut,
                ..*n
            })
            .collect();
        ev.notes.retain(|n| n.start < cut);
        // Fade-In bleibt links, Fade-Out wandert nach rechts.
        right.fade_in = 0;
        ev.fade_out = 0;
        ev.fade_in = ev.fade_in.min(ev.length);
        right.fade_out = right.fade_out.min(right.length);
        self.tracks[t].events.insert(i + 1, right);
        self.next_id += 1;
        Some(new_id)
    }

    pub fn delete(&mut self, id: EventId) -> bool {
        match self.find(id) {
            Some((t, i)) => {
                self.tracks[t].events.remove(i);
                true
            }
            None => false,
        }
    }

    /// Verschiebt (oder kopiert) ein Event auf Spur `track` an `start`.
    /// Gibt die Id des Events an der neuen Stelle zurück.
    pub fn move_event(
        &mut self,
        id: EventId,
        track: usize,
        start: u64,
        copy: bool,
    ) -> Option<EventId> {
        if !self.tracks.get(track)?.kind.holds_events() {
            return None;
        }
        let (t, i) = self.find(id)?;
        let mut ev = if copy {
            let mut c = self.tracks[t].events[i].clone();
            c.id = self.next_event_id();
            c
        } else {
            self.tracks[t].events.remove(i)
        };
        ev.start = Position(start);
        let new_id = ev.id;
        let events = &mut self.tracks[track].events;
        let pos = events.partition_point(|e| e.start.0 <= start);
        events.insert(pos, ev);
        Some(new_id)
    }

    /// Neues leeres Event (Zeichnen-Werkzeug).
    pub fn create_event(&mut self, track: usize, start: u64, length: u64) -> Option<EventId> {
        let kind = self.tracks.get(track)?.kind;
        if !kind.holds_events() || length == 0 {
            return None;
        }
        let id = self.next_event_id();
        let name = if kind == TrackKind::Audio {
            "Audio"
        } else {
            "Part"
        }
        .to_owned();
        let events = &mut self.tracks[track].events;
        let pos = events.partition_point(|e| e.start.0 <= start);
        events.insert(
            pos,
            Event {
                id,
                start: Position(start),
                length,
                name,
                muted: false,
                clip: None,
                offset: 0,
                fade_in: 0,
                fade_out: 0,
                notes: Vec::new(),
            },
        );
        Some(id)
    }

    /// Wie weit ein Audio-Event höchstens nach links (Start früher) und
    /// insgesamt reichen darf, in Ticks: begrenzt durch den Clip.
    fn clip_bounds(&self, ev: &Event) -> Option<(u64, u64)> {
        let c = self.pool.get(ev.clip? as usize)?;
        let before = self.tempo.ticks(ev.offset, c.sample_rate);
        let total = self.tempo.ticks(c.frames, c.sample_rate);
        Some((before, total))
    }

    /// Snap-Modus „Schieben“: Das Event wandert innerhalb seiner Spur an die Stelle, an der
    /// seine Mitte bei `new_start` läge. Die Events zwischen alter und neuer Stelle rücken ohne
    /// Lücke zusammen, beginnend am Anfang des frühesten von ihnen. Andere Events bleiben.
    /// Gibt zurück, ob sich die Reihenfolge geändert hat.
    pub fn shuffle_event(&mut self, id: EventId, new_start: u64) -> bool {
        let Some((t, from)) = self.find(id) else {
            return false;
        };
        let events = &mut self.tracks[t].events;
        let ev = events.remove(from);
        let mid = new_start.saturating_add(ev.length / 2);
        let to = events.partition_point(|e| e.start.0.saturating_add(e.length / 2) < mid);
        events.insert(to, ev);
        if to == from {
            return false;
        }
        let (a, b) = (from.min(to), from.max(to));
        let mut pos = events[a..=b].iter().map(|e| e.start.0).min().unwrap_or(0);
        for e in &mut events[a..=b] {
            e.start = Position(pos);
            pos = pos.saturating_add(e.length);
        }
        true
    }

    /// Verschiebt den Anfang eines Events (Länge ändern am linken Rand).
    /// Bei Audio bleibt der Inhalt an seiner Stelle; der Offset wandert mit.
    pub fn trim_start(&mut self, id: EventId, new_start: u64) -> bool {
        let Some(ev) = self.event(id) else {
            return false;
        };
        let end = ev.end();
        let mut start = new_start.min(end.saturating_sub(1));
        let bounds = self.clip_bounds(ev);
        let rate = ev
            .clip
            .and_then(|c| self.pool.get(c as usize))
            .map(|c| c.sample_rate);
        if let Some((before, _)) = bounds {
            start = start.max(ev.start.0.saturating_sub(before));
        }
        let old_start = ev.start.0;
        let tempo = self.tempo.clone();
        let Some(ev) = self.event_mut(id) else {
            return false;
        };
        if let Some(rate) = rate {
            if start >= old_start {
                ev.offset += tempo.frames(start - old_start, rate);
            } else {
                ev.offset = ev
                    .offset
                    .saturating_sub(tempo.frames(old_start - start, rate));
            }
        }
        // Noten bleiben an ihrer Stelle im Projekt.
        let shift = start as i64 - ev.start.0 as i64;
        for n in &mut ev.notes {
            n.start -= shift;
        }
        ev.start = Position(start);
        ev.length = end - start;
        ev.fade_in = ev.fade_in.min(ev.length);
        ev.fade_out = ev.fade_out.min(ev.length - ev.fade_in);
        true
    }

    /// Verschiebt das Ende eines Events (Länge ändern am rechten Rand).
    pub fn trim_end(&mut self, id: EventId, new_end: u64) -> bool {
        let Some(ev) = self.event(id) else {
            return false;
        };
        let mut end = new_end.max(ev.start.0 + 1);
        if let Some((before, total)) = self.clip_bounds(ev) {
            // Rest des Clips ab dem Event-Anfang.
            end = end.min(ev.start.0 + total.saturating_sub(before));
        }
        let Some(ev) = self.event_mut(id) else {
            return false;
        };
        ev.length = end - ev.start.0;
        ev.fade_out = ev.fade_out.min(ev.length);
        ev.fade_in = ev.fade_in.min(ev.length - ev.fade_out);
        true
    }

    /// Setzt die Fades; zusammen höchstens so lang wie das Event.
    pub fn set_fades(&mut self, id: EventId, fade_in: u64, fade_out: u64) -> bool {
        let Some(ev) = self.event_mut(id) else {
            return false;
        };
        ev.fade_in = fade_in.min(ev.length);
        ev.fade_out = fade_out.min(ev.length - ev.fade_in);
        true
    }

    pub fn add_track(&mut self, kind: TrackKind, name: impl Into<String>) -> usize {
        let i = self.tracks.len();
        let id = self.new_track_id();
        self.tracks.push(Track {
            id,
            name: name.into(),
            kind,
            color: PALETTE[i % PALETTE.len()],
            mute: false,
            solo: false,
            record: false,
            monitor: false,
            read: false,
            write: false,
            volume_db: 0.0,
            pan: 0.0,
            input: TrackInput::default(),
            inserts: Vec::new(),
            instrument: None,
            output: None,
            sends: Vec::new(),
            events: Vec::new(),
        });
        i
    }

    pub fn add_clip(&mut self, clip: PoolClip) -> ClipId {
        self.pool.push(clip);
        (self.pool.len() - 1) as ClipId
    }

    /// Legt ein Audio-Event an, das den ganzen Clip abspielt.
    pub fn add_audio_event(&mut self, track: usize, start: u64, clip: ClipId) -> Option<EventId> {
        let c = self.pool.get(clip as usize)?;
        if self.tracks.get(track)?.kind != TrackKind::Audio {
            return None;
        }
        let length = self.tempo.ticks(c.frames, c.sample_rate).max(1);
        let name = c.name.clone();
        let id = self.next_event_id();
        let events = &mut self.tracks[track].events;
        let pos = events.partition_point(|e| e.start.0 <= start);
        events.insert(
            pos,
            Event {
                id,
                start: Position(start),
                length,
                name,
                muted: false,
                clip: Some(clip),
                offset: 0,
                fade_in: 0,
                fade_out: 0,
                notes: Vec::new(),
            },
        );
        Some(id)
    }

    /// Rechnet einen Clip auf eine andere Abtastrate um (Länge im Pool und
    /// Offsets der Events), z. B. wenn ein Projekt mit anderer Geräterate geladen wird.
    pub fn rescale_clip(&mut self, clip: ClipId, rate: u32) {
        let Some(c) = self.pool.get_mut(clip as usize) else {
            return;
        };
        if c.sample_rate == rate || c.sample_rate == 0 {
            return;
        }
        let f = f64::from(rate) / f64::from(c.sample_rate);
        c.frames = (c.frames as f64 * f).round() as u64;
        c.sample_rate = rate;
        for ev in self.tracks.iter_mut().flat_map(|t| t.events.iter_mut()) {
            if ev.clip == Some(clip) {
                ev.offset = (ev.offset as f64 * f).round() as u64;
            }
        }
    }

    /// Legt einen Take obenauf (ans Ende der Liste, damit er bei Überlappung
    /// klingt): ein Ausschnitt eines Clips ab `offset` Frames, `length` in Ticks.
    pub fn place_take(
        &mut self,
        track: usize,
        start: u64,
        clip: ClipId,
        offset: u64,
        length: u64,
        name: &str,
    ) -> Option<EventId> {
        if self.tracks.get(track)?.kind != TrackKind::Audio
            || self.pool.get(clip as usize).is_none()
            || length == 0
        {
            return None;
        }
        let id = self.next_event_id();
        self.tracks[track].events.push(Event {
            id,
            start: Position(start),
            length,
            name: name.to_owned(),
            muted: false,
            clip: Some(clip),
            offset,
            fade_in: 0,
            fade_out: 0,
            notes: Vec::new(),
        });
        Some(id)
    }

    /// Das zeitlich nächste Event auf derselben Spur (beginnt am oder nach dem Start von `id`).
    fn next_on_track(&self, id: EventId) -> Option<EventId> {
        let (t, i) = self.find(id)?;
        let me = &self.tracks[t].events[i];
        self.tracks[t]
            .events
            .iter()
            .filter(|e| e.id != id && (e.start.0, e.id) > (me.start.0, me.id))
            .min_by_key(|e| (e.start.0, e.id))
            .map(|e| e.id)
    }

    /// Kleben (Werkzeug 4): verbindet ein Event mit dem nächsten auf der Spur.
    /// MIDI-Parts werden zu einem Part mit allen Noten (Lücken bleiben leer).
    /// Audio nur, wenn das Material direkt anschließt (gleicher Clip, nahtlos).
    /// Gibt das verbleibende Event zurück.
    pub fn glue(&mut self, id: EventId) -> Result<EventId, &'static str> {
        let next = self
            .next_on_track(id)
            .ok_or("kein folgendes Event auf der Spur")?;
        let (a, b) = (
            self.event(id).cloned().ok_or("Event fehlt")?,
            self.event(next).cloned().ok_or("Event fehlt")?,
        );
        let midi = a.clip.is_none() && b.clip.is_none();
        if !midi {
            let rate = a
                .clip
                .and_then(|c| self.pool.get(c as usize))
                .map(|c| c.sample_rate)
                .ok_or("Clip fehlt")?;
            let seamless = a.clip == b.clip
                && b.start.0 == a.end()
                && b.offset == a.offset + self.tempo.frames(a.length, rate);
            if !seamless {
                return Err("Audio lässt sich nur kleben, wenn das Material direkt anschließt");
            }
        }
        let shift = (b.start.0 - a.start.0) as i64;
        let end = a.end().max(b.end());
        let ev = self.event_mut(id).ok_or("Event fehlt")?;
        ev.length = end - ev.start.0;
        ev.fade_out = b.fade_out;
        ev.notes.extend(b.notes.iter().map(|n| Note {
            start: n.start + shift,
            ..*n
        }));
        self.delete(next);
        Ok(id)
    }

    /// Löscht alles im Zeitbereich `t0..t1` auf den Spuren `rows` (Bereichsauswahl
    /// + Entf). Angeschnittene Events werden an den Rändern geteilt.
    pub fn delete_range(&mut self, rows: std::ops::RangeInclusive<usize>, t0: u64, t1: u64) {
        if t1 <= t0 {
            return;
        }
        for t in rows {
            let Some(track) = self.tracks.get(t) else {
                continue;
            };
            let hit: Vec<EventId> = track
                .events
                .iter()
                .filter(|e| e.start.0 < t1 && e.end() > t0)
                .map(|e| e.id)
                .collect();
            for id in hit {
                let mut cur = id;
                if self.event(cur).is_some_and(|e| e.start.0 < t0)
                    && let Some(right) = self.split(cur, t0)
                {
                    cur = right;
                }
                if self.event(cur).is_some_and(|e| e.end() > t1) {
                    self.split(cur, t1);
                }
                self.delete(cur);
            }
        }
    }

    /// Neues Tempo. Wie in klassischen DAWs: Audio-Events behalten ihre Zeitposition
    /// (lineare Zeitbasis), MIDI-Parts ihre Taktposition (musikalisch).
    pub fn set_tempo(&mut self, bpm: f64) {
        let bpm = bpm.clamp(20.0, 300.0);
        let f = bpm / self.tempo.bpm;
        if (f - 1.0).abs() < 1e-12 {
            return;
        }
        let scale = |t: u64| (t as f64 * f).round() as u64;
        for t in self
            .tracks
            .iter_mut()
            .filter(|t| t.kind == TrackKind::Audio)
        {
            for e in &mut t.events {
                e.start = Position(scale(e.start.0));
                e.length = scale(e.length).max(1);
                e.fade_in = scale(e.fade_in).min(e.length);
                e.fade_out = scale(e.fade_out).min(e.length - e.fade_in);
            }
        }
        self.tempo.bpm = bpm;
    }

    /// Ob eine Spur zu hören ist: nicht stumm, und bei aktivem Solo selbst Solo.
    /// Gruppen und FX-Kanäle sind solo-sicher (sonst fehlte bei Solo der Hall).
    pub fn audible(&self, track: usize) -> bool {
        let any_solo = self.tracks.iter().any(|t| t.solo);
        self.tracks
            .get(track)
            .is_some_and(|t| !t.mute && (!any_solo || t.solo || Self::is_bus(t.kind)))
    }

    /// Ende des letzten Events in Ticks (mindestens ein Takt).
    pub fn length(&self) -> u64 {
        self.tracks
            .iter()
            .flat_map(|t| t.events.iter().map(Event::end))
            .max()
            .unwrap_or(0)
            .max(4 * TICKS_PER_QUARTER)
    }
}

#[cfg(test)]
mod tests {

    #[test]
    fn shuffle_swaps_neighbours_without_gaps() {
        let mut p = Project::default();
        let t = p.add_track(TrackKind::Audio, "A");
        let a = p.create_event(t, 0, 960).expect("a");
        let b = p.create_event(t, 1920, 480).expect("b");
        let c = p.create_event(t, 2400, 960).expect("c");
        let far = p.create_event(t, 9600, 960).expect("far");
        // Drop A behind B: B moves to 0, A follows directly.
        assert!(p.shuffle_event(a, 2000));
        let order: Vec<(EventId, u64)> = p.tracks[t]
            .events
            .iter()
            .map(|e| (e.id, e.start.0))
            .collect();
        assert_eq!(order, [(b, 0), (a, 480), (c, 2400), (far, 9600)]);
        // Dropping in place changes nothing.
        assert!(!p.shuffle_event(c, 2400));
        assert!(!p.shuffle_event(12345, 0));
    }

    use super::*;

    fn one_event() -> (Project, EventId) {
        let mut p = Project::demo(1);
        p.tracks[0].events.clear();
        let id = p.create_event(0, 0, 4 * TICKS_PER_QUARTER).unwrap();
        (p, id)
    }

    #[test]
    fn demo_has_events_on_event_tracks_only() {
        let p = Project::demo(14);
        for t in &p.tracks {
            assert_eq!(t.kind.holds_events(), !t.events.is_empty(), "{}", t.name);
        }
    }

    #[test]
    fn split_keeps_total_length() {
        let (mut p, id) = one_event();
        let right = p.split(id, TICKS_PER_QUARTER).unwrap();
        assert_eq!(p.event(id).unwrap().length, TICKS_PER_QUARTER);
        assert_eq!(p.event(right).unwrap().start, Position(TICKS_PER_QUARTER));
        assert_eq!(p.event(right).unwrap().length, 3 * TICKS_PER_QUARTER);
        assert_eq!(p.split(id, 0), None, "am Rand wird nicht geteilt");
    }

    #[test]
    fn move_and_copy() {
        let mut p = Project::demo(2);
        p.tracks[1].kind = TrackKind::Audio;
        p.tracks.iter_mut().for_each(|t| t.events.clear());
        let id = p.create_event(0, 0, 960).unwrap();
        let copy = p.move_event(id, 1, 1920, true).unwrap();
        assert_ne!(copy, id);
        assert_eq!(p.tracks[0].events.len(), 1);
        assert_eq!(p.find(copy), Some((1, 0)));
        let moved = p.move_event(id, 1, 0, false).unwrap();
        assert_eq!(moved, id);
        assert!(p.tracks[0].events.is_empty());
        assert_eq!(p.tracks[1].events[0].id, id, "nach Start sortiert");
    }

    fn pool_project() -> (Project, ClipId) {
        let mut p = Project::default();
        p.add_track(TrackKind::Audio, "Gesang");
        // 2 s bei 48 kHz = 1 Takt bei 120 BPM
        let c = p.add_clip(PoolClip {
            name: "take".into(),
            path: None,
            sample_rate: 48_000,
            frames: 96_000,
        });
        (p, c)
    }

    #[test]
    fn audio_event_spans_clip() {
        let (mut p, c) = pool_project();
        let id = p.add_audio_event(0, 0, c).unwrap();
        assert_eq!(p.event(id).unwrap().length, p.tempo.signature.bar_ticks());
        assert_eq!(p.event(id).unwrap().clip, Some(c));
    }

    #[test]
    fn split_moves_offset_of_right_part() {
        let (mut p, c) = pool_project();
        let id = p.add_audio_event(0, 0, c).unwrap();
        let right = p.split(id, TICKS_PER_QUARTER).unwrap();
        // Eine Viertel bei 120 BPM = 0,5 s = 24 000 Frames.
        assert_eq!(p.event(right).unwrap().offset, 24_000);
        assert_eq!(p.event(id).unwrap().offset, 0);
    }

    #[test]
    fn solo_and_mute_decide_audibility() {
        let mut p = Project::default();
        for n in ["a", "b", "c"] {
            p.add_track(TrackKind::Audio, n);
        }
        assert!(p.audible(0) && p.audible(1));
        p.tracks[1].mute = true;
        assert!(!p.audible(1));
        p.tracks[2].solo = true;
        assert!(!p.audible(0) && !p.audible(1) && p.audible(2));
    }

    #[test]
    fn fader_and_balance_gains() {
        let mut p = Project::default();
        p.add_track(TrackKind::Audio, "a");
        let t = &mut p.tracks[0];
        assert_eq!(t.gains(), (1.0, 1.0));
        t.volume_db = -6.0;
        let (l, r) = t.gains();
        assert!((l - 0.501).abs() < 0.001 && l == r);
        t.volume_db = 0.0;
        t.pan = -0.5;
        assert_eq!(t.gains(), (1.0, 0.5), "links: rechter Kanal leiser");
        t.volume_db = MIN_DB;
        assert_eq!(t.gains(), (0.0, 0.0));
    }

    #[test]
    fn trimming_keeps_audio_in_place_and_respects_clip() {
        let (mut p, c) = pool_project(); // Clip = 1 Takt
        let bar = p.tempo.signature.bar_ticks();
        let q = TICKS_PER_QUARTER;
        let id = p.add_audio_event(0, bar, c).unwrap();
        // Anfang eine Viertel nach rechts: Offset wächst um 0,5 s.
        assert!(p.trim_start(id, bar + q));
        let ev = p.event(id).unwrap();
        assert_eq!(
            (ev.start.0, ev.length, ev.offset),
            (bar + q, bar - q, 24_000)
        );
        // Zurück nach links geht nur bis zum Clip-Anfang.
        p.trim_start(id, 0);
        let ev = p.event(id).unwrap();
        assert_eq!((ev.start.0, ev.offset), (bar, 0));
        // Ende nicht über das Clip-Ende hinaus.
        p.trim_end(id, 10 * bar);
        assert_eq!(p.event(id).unwrap().end(), 2 * bar);
        p.trim_end(id, bar + q);
        assert_eq!(p.event(id).unwrap().length, q);
    }

    #[test]
    fn fades_are_clamped_and_split_correctly() {
        let (mut p, c) = pool_project();
        let bar = p.tempo.signature.bar_ticks();
        let id = p.add_audio_event(0, 0, c).unwrap();
        p.set_fades(id, bar, bar);
        let ev = p.event(id).unwrap();
        assert_eq!(
            (ev.fade_in, ev.fade_out),
            (bar, 0),
            "zusammen höchstens Event-Länge"
        );
        p.set_fades(id, 960, 960);
        let right = p.split(id, bar / 2).unwrap();
        assert_eq!(
            (p.event(id).unwrap().fade_in, p.event(id).unwrap().fade_out),
            (960, 0)
        );
        assert_eq!(
            (
                p.event(right).unwrap().fade_in,
                p.event(right).unwrap().fade_out
            ),
            (0, 960)
        );
    }

    #[test]
    fn input_options_for_two_channels() {
        let o = TrackInput::options(2);
        let labels: Vec<String> = o.iter().map(|i| i.label()).collect();
        assert_eq!(labels, ["Eingang 1/2", "Eingang 1", "Eingang 2"]);
    }

    #[test]
    fn notes_follow_split_and_trim() {
        let mut p = Project::default();
        let t = p.add_track(TrackKind::Instrument, "Synth");
        let q = TICKS_PER_QUARTER;
        let id = p.create_event(t, 4 * q, 4 * q).unwrap();
        let ev = p.event_mut(id).unwrap();
        for (i, pitch) in [60u8, 62, 64, 65].into_iter().enumerate() {
            ev.notes.push(Note {
                pitch,
                velocity: 100,
                start: (i as u64 * q) as i64,
                length: q,
            });
        }
        // Anfang eine Viertel nach rechts: die Noten bleiben an ihrem Platz.
        p.trim_start(id, 5 * q);
        let ev = p.event(id).unwrap();
        assert_eq!(
            ev.notes[0].start,
            -(q as i64),
            "erste Note ist jetzt ausgeblendet"
        );
        assert_eq!(ev.notes[1].start, 0);
        // Teilen bei Viertel 7: E und F gehen in den rechten Part.
        let right = p.split(id, 7 * q).unwrap();
        let pitches = |id| {
            p.event(id)
                .unwrap()
                .notes
                .iter()
                .map(|n| n.pitch)
                .collect::<Vec<_>>()
        };
        assert_eq!(pitches(id), [60, 62, 64]);
        assert_eq!(pitches(right), [65]);
        assert_eq!(p.event(right).unwrap().notes[0].start, 0);
    }

    #[test]
    fn tempo_change_keeps_audio_time_and_midi_bars() {
        let (mut p, c) = pool_project(); // 2 s Audio = 1 Takt bei 120 BPM
        let bar = p.tempo.signature.bar_ticks();
        let a = p.add_audio_event(0, bar, c).unwrap(); // bei 2 s
        let m = p.add_track(TrackKind::Instrument, "Synth");
        let part = p.create_event(m, bar, bar).unwrap();
        let secs = |p: &Project, id| {
            let e = p.event(id).unwrap();
            (
                p.tempo.seconds(e.start),
                p.tempo.seconds(Position(e.length)),
            )
        };
        assert_eq!(secs(&p, a), (2.0, 2.0));
        p.set_tempo(60.0);
        assert_eq!(secs(&p, a), (2.0, 2.0), "Audio bleibt bei 2 s und 2 s lang");
        assert_eq!(
            p.event(part).unwrap().start.0,
            bar,
            "MIDI-Part bleibt bei Takt 2"
        );
        assert_eq!(secs(&p, part), (4.0, 4.0), "… und dauert nun 4 s");
    }

    #[test]
    fn track_ids_and_loop_free_routing() {
        let mut p = Project::default();
        let a = p.add_track(TrackKind::Audio, "Gesang");
        let fx = p.add_track(TrackKind::Fx, "Hall");
        let g = p.add_track(TrackKind::Group, "Gruppe");
        let (ia, ifx, ig) = (p.tracks[a].id, p.tracks[fx].id, p.tracks[g].id);
        assert!(ia != ifx && ifx != ig && ia > 0);
        assert!(p.can_route(ia, ifx));
        p.tracks[fx].output = Some(ig);
        assert!(
            !p.can_route(ig, ifx),
            "Gruppe → Hall → Gruppe wäre eine Schleife"
        );
        assert!(!p.can_route(ig, ig));
        // Ältere Dateien ohne Kennung bekommen eine.
        p.tracks[a].id = 0;
        p.tracks[g].id = ifx;
        p.ensure_track_ids();
        let ids: std::collections::HashSet<_> = p.tracks.iter().map(|t| t.id).collect();
        assert_eq!(ids.len(), 3);
        assert!(!ids.contains(&0));
    }

    #[test]
    fn buses_stay_audible_under_solo() {
        let mut p = Project::default();
        p.add_track(TrackKind::Audio, "a");
        p.add_track(TrackKind::Audio, "b");
        p.add_track(TrackKind::Fx, "Hall");
        p.tracks[0].solo = true;
        assert!(p.audible(0) && !p.audible(1) && p.audible(2));
    }

    #[test]
    fn delete_range_cuts_out_the_middle() {
        let q = TICKS_PER_QUARTER;
        let (mut p, c) = pool_project(); // Clip = 1 Takt = 4 Viertel
        let id = p.add_audio_event(0, 0, c).unwrap();
        p.delete_range(0..=0, q, 2 * q);
        let spans: Vec<(u64, u64, u64)> = p.tracks[0]
            .events
            .iter()
            .map(|e| (e.start.0, e.length, e.offset))
            .collect();
        assert_eq!(
            spans,
            [(0, q, 0), (2 * q, 2 * q, 48_000)],
            "Mitte weg, rechter Teil passt zum Material"
        );
        assert_eq!(p.tracks[0].events[0].id, id);
        // Ganz umschlossen: verschwindet.
        p.delete_range(0..=0, 0, 10 * q);
        assert!(p.tracks[0].events.is_empty());
    }

    #[test]
    fn glue_parts_and_seamless_audio() {
        let q = TICKS_PER_QUARTER;
        // MIDI: zwei Parts mit Lücke werden einer, Noten an ihrer Stelle.
        let mut p = Project::default();
        let t = p.add_track(TrackKind::Instrument, "Synth");
        let a = p.create_event(t, 0, 2 * q).unwrap();
        let b = p.create_event(t, 4 * q, 2 * q).unwrap();
        p.event_mut(a).unwrap().notes.push(Note {
            pitch: 60,
            velocity: 100,
            start: 0,
            length: q,
        });
        p.event_mut(b).unwrap().notes.push(Note {
            pitch: 64,
            velocity: 100,
            start: 0,
            length: q,
        });
        assert_eq!(p.glue(a), Ok(a));
        let ev = p.event(a).unwrap();
        assert_eq!(ev.length, 6 * q);
        assert_eq!(
            ev.notes
                .iter()
                .map(|n| (n.pitch, n.start))
                .collect::<Vec<_>>(),
            [(60, 0), (64, 4 * q as i64)]
        );
        assert!(p.event(b).is_none());
        assert!(p.glue(a).is_err(), "nichts mehr dahinter");
        // Audio: Teilen und wieder kleben ergibt das Original.
        let (mut p, c) = pool_project();
        let id = p.add_audio_event(0, 0, c).unwrap();
        let original = p.event(id).unwrap().clone();
        p.split(id, q);
        assert_eq!(p.glue(id), Ok(id));
        assert_eq!(p.event(id).unwrap(), &original);
        // Audio mit Lücke: nicht klebbar.
        let r = p.split(id, q).unwrap();
        p.move_event(r, 0, 3 * q, false);
        assert!(p.glue(id).is_err());
    }

    #[test]
    fn no_events_on_group_tracks() {
        let mut p = Project::demo(7);
        assert_eq!(p.tracks[6].kind, TrackKind::Group);
        assert_eq!(p.create_event(6, 0, 960), None);
    }
}
