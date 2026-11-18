//! Spuren und Events (Spec-Abschnitte „Begriffe und Datenmodell“ und „Spurtypen“).

use crate::{Position, TICKS_PER_QUARTER, TempoMap};

pub type EventId = u64;

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
    pub events: Vec<Event>,
}

#[derive(Debug, Clone)]
pub struct Project {
    pub tracks: Vec<Track>,
    pub tempo: TempoMap,
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
        right.start = Position(at);
        right.length = ev.end() - at;
        ev.length = at - ev.start.0;
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
            },
        );
        Some(id)
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

    #[test]
    fn no_events_on_group_tracks() {
        let mut p = Project::demo(7);
        assert_eq!(p.tracks[6].kind, TrackKind::Group);
        assert_eq!(p.create_event(6, 0, 960), None);
    }
}
