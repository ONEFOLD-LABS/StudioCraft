//! Audio-Parts (Issue #184): ein Event auf einer Audiospur, das mehrere Audio-Events
//! zusammenfasst, wie ein MIDI-Part Noten. Verschieben, Kopieren, Ausschneiden, Stummschalten
//! und Fades wirken auf den Part als Ganzes; Wiedergabe und Export lösen ihn in seine Events
//! auf ([`Project::flattened_track`]).
//!
//! Die Events im Part (`Event::children`) haben ihren Anfang relativ zum Part-Anfang und
//! selbst keine Kinder. Was außerhalb des Parts liegt, ist ausgeblendet, nicht gelöscht; nur
//! wenn der Part-Anfang hinter den Anfang eines Events rückt, wird dessen Kopf abgeschnitten
//! (ein Anfang vor dem Part ist nicht darstellbar).

use std::borrow::Cow;

use crate::{Event, EventId, Position, Project, TempoMap, Track, TrackKind};

/// Höchstzahl der Events in einem Part.
pub const MAX_PART_EVENTS: usize = 1000;

impl Event {
    /// Ein Audio-Part (fasst Audio-Events zusammen).
    pub fn is_audio_part(&self) -> bool {
        !self.children.is_empty()
    }
}

/// Clip-Abtastrate eines Events.
fn rate_of(pool: &[crate::PoolClip], ev: &Event) -> Option<u32> {
    ev.clip
        .and_then(|c| pool.get(c as usize))
        .map(|c| c.sample_rate)
}

/// Schneidet `cut` Ticks vom Anfang eines Audio-Events ab (Offset wandert mit); `None`, wenn
/// nichts übrig bleibt.
fn cut_head(ev: &Event, cut: u64, pool: &[crate::PoolClip], tempo: &TempoMap) -> Option<Event> {
    if cut >= ev.length {
        return None;
    }
    let mut e = ev.clone();
    if cut > 0 {
        if let Some(rate) = rate_of(pool, ev) {
            e.offset = ev.shifted_offset(tempo, rate, i64::try_from(cut).unwrap_or(i64::MAX));
        }
        e.length -= cut;
        e.fade_in = e.fade_in.saturating_sub(cut);
        e.fade_out = e.fade_out.min(e.length - e.fade_in.min(e.length));
    }
    Some(e)
}

/// Verschiebt die Events eines Parts um `shift` Ticks nach links (der Part-Anfang rückt um
/// `shift` nach rechts, die Events bleiben an ihrer Stelle im Projekt). Was dabei vor den
/// Part-Anfang geriete, wird abgeschnitten.
pub(crate) fn shift_children(
    children: &mut Vec<Event>,
    shift: i64,
    pool: &[crate::PoolClip],
    tempo: &TempoMap,
) {
    if shift == 0 {
        return;
    }
    let old = std::mem::take(children);
    for c in old {
        let rel = i64::try_from(c.start.0).unwrap_or(i64::MAX) - shift;
        if rel >= 0 {
            children.push(Event {
                start: Position(rel.unsigned_abs()),
                ..c
            });
        } else if let Some(mut e) = cut_head(&c, rel.unsigned_abs(), pool, tempo) {
            e.start = Position(0);
            children.push(e);
        }
    }
}

/// Teilt die Events eines Parts bei `cut` (Ticks ab Part-Anfang) in linke und rechte; ein
/// Event über dem Schnitt wird geteilt.
pub(crate) fn split_children(
    children: &[Event],
    cut: u64,
    pool: &[crate::PoolClip],
    tempo: &TempoMap,
) -> (Vec<Event>, Vec<Event>) {
    let (mut left, mut right) = (Vec::new(), Vec::new());
    for c in children {
        let (s, e) = (c.start.0, c.end());
        if e <= cut {
            left.push(c.clone());
        } else if s >= cut {
            right.push(Event {
                start: Position(s - cut),
                ..c.clone()
            });
        } else {
            let mut l = c.clone();
            l.length = cut - s;
            l.fade_out = 0;
            l.fade_in = l.fade_in.min(l.length);
            left.push(l);
            if let Some(mut r) = cut_head(c, cut - s, pool, tempo) {
                r.start = Position(0);
                r.fade_in = 0;
                right.push(r);
            }
        }
    }
    (left, right)
}

impl Project {
    /// Die Events eines Parts an ihrer Stelle im Projekt, auf den Part zugeschnitten. Stumm,
    /// Lautstärke und Fades des Parts gehen auf sie über.
    pub fn part_children(&self, part: &Event) -> Vec<Event> {
        let (ps, pe) = (part.start.0, part.end());
        let mut out = Vec::new();
        for c in &part.children {
            let a = ps.saturating_add(c.start.0);
            let b = a.saturating_add(c.length).min(pe);
            if a >= b {
                continue;
            }
            let mut e = c.clone();
            e.start = Position(a);
            e.length = b - a;
            e.children.clear();
            e.muted |= part.muted;
            e.gain_db += part.gain_db;
            e.comp = e.comp.max(part.comp);
            e.lane = part.lane;
            // Fades des Parts: wo sie das Event berühren, mindestens so lang wie dort übrig.
            let fade_in_end = ps.saturating_add(part.fade_in);
            if part.fade_in > 0 && a < fade_in_end {
                e.fade_in = e.fade_in.max(fade_in_end - a).min(e.length);
                e.fade_in_curve = part.fade_in_curve;
            }
            let fade_out_start = pe.saturating_sub(part.fade_out);
            if part.fade_out > 0 && b > fade_out_start {
                e.fade_out = e
                    .fade_out
                    .max(b - fade_out_start.max(a))
                    .min(e.length - e.fade_in.min(e.length));
                e.fade_out_curve = part.fade_out_curve;
            }
            out.push(e);
        }
        out
    }

    /// Die Spur mit aufgelösten Audio-Parts (für Wiedergabe und Export). Ohne Parts die
    /// Spur selbst.
    pub fn flattened_track(&self, track: usize) -> Option<Cow<'_, Track>> {
        let t = self.tracks.get(track)?;
        if !t.events.iter().any(Event::is_audio_part) {
            return Some(Cow::Borrowed(t));
        }
        let mut flat = t.clone();
        flat.events = t
            .events
            .iter()
            .flat_map(|e| {
                if e.is_audio_part() {
                    self.part_children(e)
                } else {
                    vec![e.clone()]
                }
            })
            .collect();
        Some(Cow::Owned(flat))
    }

    /// Fasst Audio-Events einer Audiospur zu einem Part zusammen. Alle müssen auf derselben
    /// Spur liegen und einen Clip haben; mindestens eines. Gibt die Id des Parts zurück.
    pub fn group_audio(&mut self, ids: &[EventId]) -> Option<EventId> {
        let mut found: Vec<(usize, usize)> = ids.iter().filter_map(|&id| self.find(id)).collect();
        found.sort_unstable();
        found.dedup();
        let &(t, first) = found.first()?;
        if found.iter().any(|&(tt, _)| tt != t)
            || self.tracks[t].kind != TrackKind::Audio
            || found.len() > MAX_PART_EVENTS
        {
            return None;
        }
        let events = &self.tracks[t].events;
        if found
            .iter()
            .any(|&(_, i)| events[i].clip.is_none() || events[i].is_audio_part())
        {
            return None;
        }
        let start = found.iter().map(|&(_, i)| events[i].start.0).min()?;
        let end = found.iter().map(|&(_, i)| events[i].end()).max()?;
        let mut children: Vec<Event> = found
            .iter()
            .map(|&(_, i)| {
                let e = &events[i];
                Event {
                    start: Position(e.start.0 - start),
                    lane: None,
                    shared: None,
                    ..e.clone()
                }
            })
            .collect();
        // Reihenfolge bleibt: bei Überlappung klingt das spätere (wie auf der Spur).
        children.sort_by_key(|c| c.start.0);
        let id = self.next_event_id();
        let mut part = Event {
            id,
            start: Position(start),
            length: end - start,
            name: "Audio-Part".into(),
            muted: false,
            clip: None,
            offset: 0,
            fade_in: 0,
            fade_out: 0,
            fade_in_curve: Default::default(),
            fade_out_curve: Default::default(),
            gain_db: 0.0,
            notes: Vec::new(),
            controllers: Vec::new(),
            lane: self.tracks[t].events[first].lane,
            comp: 0,
            color: None,
            shared: None,
            musical: None,
            stretch_mode: Default::default(),
            warp: Vec::new(),
            score_quantize: None,
            children: Vec::new(),
        };
        part.children = children;
        for &(_, i) in found.iter().rev() {
            self.tracks[t].events.remove(i);
        }
        let at = first.min(self.tracks[t].events.len());
        self.tracks[t].events.insert(at, part);
        Some(id)
    }

    /// Löst einen Audio-Part wieder in seine Events auf (an ihrer Stelle, auf den Part
    /// zugeschnitten). Gibt die neuen Ids zurück.
    pub fn dissolve_part(&mut self, id: EventId) -> Vec<EventId> {
        let Some((t, i)) = self.find(id) else {
            return Vec::new();
        };
        let part = self.tracks[t].events[i].clone();
        if !part.is_audio_part() {
            return Vec::new();
        }
        let mut events = self.part_children(&part);
        let mut ids = Vec::new();
        for e in &mut events {
            e.id = self.next_event_id();
            e.muted = part.muted && e.muted;
            ids.push(e.id);
        }
        self.tracks[t].events.splice(i..=i, events);
        ids
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{PoolClip, TICKS_PER_QUARTER};

    const Q: u64 = TICKS_PER_QUARTER;

    /// Eine Audiospur mit zwei Events aus einem Clip (48 kHz, 4 s) bei 120 BPM.
    fn setup() -> (Project, EventId, EventId) {
        let mut p = Project::default();
        let t = p.add_track(TrackKind::Audio, "Gitarre");
        let c = p.add_clip(PoolClip {
            name: "Take".into(),
            path: None,
            sample_rate: 48_000,
            frames: 4 * 48_000,
        });
        let a = p.add_audio_event(t, 0, c).unwrap();
        let b = p.add_audio_event(t, 4 * Q, c).unwrap();
        p.event_mut(a).unwrap().length = 2 * Q;
        p.event_mut(b).unwrap().length = 2 * Q;
        p.event_mut(b).unwrap().offset = 24_000;
        (p, a, b)
    }

    #[test]
    fn grouping_and_dissolving_keep_the_events_in_place() {
        let (mut p, a, b) = setup();
        let before: Vec<(u64, u64, u64)> = p.tracks[0]
            .events
            .iter()
            .map(|e| (e.start.0, e.length, e.offset))
            .collect();
        let part = p.group_audio(&[b, a]).unwrap();
        assert_eq!(p.tracks[0].events.len(), 1);
        let ev = p.event(part).unwrap();
        assert!(ev.is_audio_part() && ev.clip.is_none());
        assert_eq!((ev.start.0, ev.length), (0, 6 * Q));
        assert_eq!(ev.children[1].start.0, 4 * Q, "relative to the part");
        // Moving the part moves its events (relative positions).
        p.event_mut(part).unwrap().start = Position(8 * Q);
        let flat = p.flattened_track(0).unwrap();
        let starts: Vec<u64> = flat.events.iter().map(|e| e.start.0).collect();
        assert_eq!(starts, [8 * Q, 12 * Q]);
        p.event_mut(part).unwrap().start = Position(0);
        let ids = p.dissolve_part(part);
        assert_eq!(ids.len(), 2);
        let after: Vec<(u64, u64, u64)> = p.tracks[0]
            .events
            .iter()
            .map(|e| (e.start.0, e.length, e.offset))
            .collect();
        assert_eq!(after, before);
    }

    #[test]
    fn grouping_needs_audio_events_on_one_audio_track() {
        let (mut p, a, _) = setup();
        let m = p.add_track(TrackKind::Instrument, "Synth");
        let part = p.create_event(m, 0, Q).unwrap();
        assert!(p.group_audio(&[a, part]).is_none(), "two tracks");
        assert!(p.group_audio(&[part]).is_none(), "MIDI part");
        assert!(p.group_audio(&[]).is_none());
    }

    #[test]
    fn the_part_mutes_fades_and_hides_what_lies_outside() {
        let (mut p, a, b) = setup();
        let part = p.group_audio(&[a, b]).unwrap();
        let ev = p.event_mut(part).unwrap();
        ev.length = 5 * Q; // the second event is cut at its middle
        ev.fade_in = Q / 2;
        ev.gain_db = -6.0;
        ev.muted = true;
        let flat = p.flattened_track(0).unwrap().into_owned();
        assert_eq!(flat.events[1].length, Q, "hidden behind the part end");
        assert_eq!(flat.events[0].fade_in, Q / 2);
        assert!(flat.events.iter().all(|e| e.muted && e.gain_db == -6.0));
        // Still all there inside the part.
        assert_eq!(p.event(part).unwrap().children[1].length, 2 * Q);
    }

    #[test]
    fn splitting_and_trimming_a_part_cut_its_events_correctly() {
        let (mut p, a, b) = setup();
        let part = p.group_audio(&[a, b]).unwrap();
        // Split through the first event: its right half continues at the clip position.
        let right = p.split(part, Q).unwrap();
        let l = p.event(part).unwrap().clone();
        let r = p.event(right).unwrap().clone();
        assert_eq!(l.children.len(), 1);
        assert_eq!(l.children[0].length, Q);
        assert_eq!(r.children.len(), 2);
        assert_eq!((r.children[0].start.0, r.children[0].offset), (0, 24_000));
        assert_eq!(r.children[1].start.0, 3 * Q);
        // Trimming the right part's start past its first event drops it, keeps the second.
        assert!(p.trim_start(right, 3 * Q));
        let r = p.event(right).unwrap();
        assert_eq!(r.children.len(), 1);
        assert_eq!(r.children[0].start.0, Q);
        let flat = p.flattened_track(0).unwrap();
        let starts: Vec<u64> = flat.events.iter().map(|e| e.start.0).collect();
        assert_eq!(starts, [0, 4 * Q], "the audio stays where it was");
    }

    #[test]
    fn hostile_parts_are_cleaned() {
        let (mut p, a, b) = setup();
        let part = p.group_audio(&[a, b]).unwrap();
        let ev = p.event_mut(part).unwrap();
        let mut nested = ev.children[0].clone();
        nested.children = vec![ev.children[1].clone()];
        ev.children.push(nested);
        ev.children[0].clip = Some(99);
        ev.children[1].gain_db = f32::NAN;
        assert!(p.sanitize() > 0);
        let ev = p.event(part).unwrap();
        assert_eq!(ev.children.len(), 2, "missing clip dropped");
        assert!(ev.children.iter().all(|c| c.children.is_empty()));
        assert!(ev.children.iter().all(|c| c.gain_db.is_finite()));
    }

    #[test]
    fn parts_survive_the_file_and_older_files_load() {
        let (mut p, a, b) = setup();
        p.group_audio(&[a, b]).unwrap();
        let back = crate::file::from_json(&crate::file::to_json(&p)).unwrap();
        assert_eq!(back.tracks[0].events[0].children.len(), 2);
        let plain = crate::file::to_json(&setup().0);
        assert!(!plain.contains("children"));
    }
}
