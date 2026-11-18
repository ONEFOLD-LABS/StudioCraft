//! Spuren und Events (Spec-Abschnitte „Begriffe und Datenmodell“ und „Spurtypen“).

use crate::{Position, TICKS_PER_QUARTER, TempoMap};

pub type EventId = u64;
/// Index eines Clips im Pool.
pub type ClipId = u32;

/// Eintrag im Pool: eine Audio-Datei (Spec: „Pool“, „Audio-Clip“).
/// Die Samples selbst hält die Engine; das Modell kennt nur die Eckdaten.
#[derive(Debug, Clone, PartialEq)]
pub struct PoolClip {
    pub name: String,
    pub path: Option<String>,
    /// Abtastrate der geladenen Samples.
    pub sample_rate: u32,
    pub frames: u64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
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
#[derive(Debug, Clone, PartialEq)]
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
}

impl Event {
    pub fn end(&self) -> u64 {
        self.start.0 + self.length
    }

    pub fn contains(&self, ticks: u64) -> bool {
        (self.start.0..self.end()).contains(&ticks)
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct Track {
    pub name: String,
    pub kind: TrackKind,
    pub color: [u8; 3],
    pub mute: bool,
    pub solo: bool,
    pub record: bool,
    /// Fader in dB; [`MIN_DB`] und darunter gilt als −∞.
    pub volume_db: f32,
    /// Panorama: −1 links, 0 Mitte, +1 rechts (Stereo-Balance).
    pub pan: f32,
    pub events: Vec<Event>,
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

#[derive(Debug, Clone, PartialEq)]
pub struct Project {
    pub tracks: Vec<Track>,
    pub tempo: TempoMap,
    pub pool: Vec<PoolClip>,
    /// Fader des Ausgangsbusses in dB.
    pub master_db: f32,
    next_id: EventId,
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
        }
    }
}

impl Project {
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
                name: format!("{} {}", kind.label(), i + 1),
                kind,
                color: PALETTE[i % PALETTE.len()],
                mute: false,
                solo: false,
                record: false,
                volume_db: 0.0,
                pan: 0.0,
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
                    });
                    at += len + ((i as u64 + 2 * k) % 3) * bar;
                    k += 1;
                }
            }
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
        self.tracks.push(Track {
            name: name.into(),
            kind,
            color: PALETTE[i % PALETTE.len()],
            mute: false,
            solo: false,
            record: false,
            volume_db: 0.0,
            pan: 0.0,
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
            },
        );
        Some(id)
    }

    /// Ob eine Spur zu hören ist: nicht stumm, und bei aktivem Solo selbst Solo.
    pub fn audible(&self, track: usize) -> bool {
        let any_solo = self.tracks.iter().any(|t| t.solo);
        self.tracks
            .get(track)
            .is_some_and(|t| !t.mute && (!any_solo || t.solo))
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
    fn no_events_on_group_tracks() {
        let mut p = Project::demo(7);
        assert_eq!(p.tracks[6].kind, TrackKind::Group);
        assert_eq!(p.create_event(6, 0, 960), None);
    }
}
