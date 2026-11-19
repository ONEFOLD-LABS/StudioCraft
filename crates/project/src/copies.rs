//! Kopieren und Wiederholen (Spec „Raster, Snap und Bearbeitung“ → Verschieben und Kopieren;
//! Issue #6): geteilte Kopien und Wiederholen einer Auswahl.
//!
//! Geteilte Kopien haben dieselbe Gruppe (`Event::shared`) und teilen ihren Inhalt: die Noten
//! eines MIDI-Parts bzw. den Clip und Versatz eines Audio-Events. Position und Länge bleiben je
//! Kopie eigen. [`Project::sync_shared`] gleicht nach einer Bearbeitung ab.

use crate::{EventId, Position, Project};

impl Project {
    fn next_shared_group(&self) -> u32 {
        self.tracks
            .iter()
            .flat_map(|t| t.events.iter().filter_map(|e| e.shared))
            .max()
            .map_or(1, |g| g.saturating_add(1))
    }

    /// Macht `copy` zur geteilten Kopie von `original` (legt die Gruppe bei Bedarf an).
    pub fn link_shared(&mut self, original: EventId, copy: EventId) -> bool {
        if original == copy {
            return false;
        }
        let group = match self.event(original).map(|e| e.shared) {
            Some(Some(g)) => g,
            Some(None) => self.next_shared_group(),
            None => return false,
        };
        let content = self
            .event(original)
            .map(|e| (e.notes.clone(), e.controllers.clone(), e.clip, e.offset));
        let (Some((notes, controllers, clip, offset)), true) =
            (content, self.event(copy).is_some())
        else {
            return false;
        };
        if let Some(e) = self.event_mut(original) {
            e.shared = Some(group);
        }
        if let Some(e) = self.event_mut(copy) {
            e.shared = Some(group);
            e.notes = notes;
            e.controllers = controllers;
            e.clip = clip;
            e.offset = offset;
        }
        true
    }

    /// Löst ein Event aus seiner Gruppe (es wird eine gewöhnliche Kopie).
    pub fn unlink_shared(&mut self, id: EventId) {
        if let Some(e) = self.event_mut(id) {
            e.shared = None;
        }
    }

    /// Übernimmt geänderten Inhalt geteilter Kopien in ihre ganze Gruppe: Ein Event, dessen
    /// Noten oder Clip sich gegenüber `before` geändert haben, gibt sie an alle Kopien weiter.
    pub fn sync_shared(&mut self, before: &Project) {
        let mut changed = Vec::new();
        for t in &self.tracks {
            for e in &t.events {
                let Some(g) = e.shared else { continue };
                let differs = before.event(e.id).is_none_or(|b| {
                    b.notes != e.notes
                        || b.controllers != e.controllers
                        || b.clip != e.clip
                        || b.offset != e.offset
                });
                if differs && before.event(e.id).is_some() {
                    changed.push((
                        g,
                        e.id,
                        e.notes.clone(),
                        e.controllers.clone(),
                        e.clip,
                        e.offset,
                    ));
                }
            }
        }
        for (g, src, notes, controllers, clip, offset) in changed {
            for t in &mut self.tracks {
                for e in t
                    .events
                    .iter_mut()
                    .filter(|e| e.shared == Some(g) && e.id != src)
                {
                    e.notes.clone_from(&notes);
                    e.controllers.clone_from(&controllers);
                    e.clip = clip;
                    e.offset = offset;
                }
            }
        }
    }

    /// Wiederholt die Events `ids` `count` Mal direkt hintereinander (der Abstand ist die Spanne
    /// der Auswahl). Mit `shared` werden es geteilte Kopien. Gibt die neuen Ids zurück.
    pub fn repeat(&mut self, ids: &[EventId], count: u32, shared: bool) -> Vec<EventId> {
        let events: Vec<(usize, crate::Event)> = ids
            .iter()
            .filter_map(|&id| {
                let (t, i) = self.find(id)?;
                Some((t, self.tracks[t].events[i].clone()))
            })
            .collect();
        let (Some(first), Some(end)) = (
            events.iter().map(|(_, e)| e.start.0).min(),
            events.iter().map(|(_, e)| e.end()).max(),
        ) else {
            return Vec::new();
        };
        let span = end - first;
        let mut out = Vec::new();
        for k in 1..=u64::from(count.min(999)) {
            for (t, e) in &events {
                let mut c = e.clone();
                c.start = Position(e.start.0.saturating_add(span.saturating_mul(k)));
                if let Some(nid) = self.insert_event(*t, c) {
                    if shared {
                        self.link_shared(e.id, nid);
                    } else {
                        self.unlink_shared(nid);
                    }
                    out.push(nid);
                }
            }
        }
        out
    }
}

#[cfg(test)]
mod tests {
    use crate::{Note, Project, TrackKind};

    fn note(pitch: u8) -> Note {
        Note {
            pitch,
            velocity: 100,
            start: 0,
            length: 480,
        }
    }

    #[test]
    fn editing_one_shared_copy_changes_all() {
        let mut p = Project::default();
        let t = p.add_track(TrackKind::Instrument, "Synth");
        let a = p.create_event(t, 0, 3840).unwrap();
        p.event_mut(a).unwrap().notes = vec![note(60)];
        let copies = p.repeat(&[a], 2, true);
        assert_eq!(copies.len(), 2);
        let starts: Vec<u64> = p.tracks[t].events.iter().map(|e| e.start.0).collect();
        assert_eq!(starts, [0, 3840, 7680]);
        let before = p.clone();
        p.event_mut(copies[1]).unwrap().notes.push(note(67));
        p.sync_shared(&before);
        for id in [a, copies[0], copies[1]] {
            assert_eq!(p.event(id).unwrap().notes.len(), 2, "all copies follow");
        }
        // Plain repeats are independent.
        let plain = p.repeat(&[a], 1, false);
        let before = p.clone();
        p.event_mut(plain[0]).unwrap().notes.clear();
        p.sync_shared(&before);
        assert_eq!(p.event(a).unwrap().notes.len(), 2);
        // Saved and loaded with their group.
        let back = crate::file::from_json(&crate::file::to_json(&p)).unwrap();
        assert_eq!(
            back.event(copies[0]).unwrap().shared,
            p.event(copies[0]).unwrap().shared
        );
    }
}
