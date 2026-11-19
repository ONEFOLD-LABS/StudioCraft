//! MIDI-Aufnahmemodi (Spec „Aufnahmemodi“, Issue #14): wohin aufgenommene Noten kommen.

use crate::{EventId, Note, Project};

/// Aufnahme über bestehende Parts (ohne Cycle).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum MidiRecordMode {
    /// Neuer Part obenauf.
    #[default]
    NewPart,
    /// In den vorhandenen Part an dieser Stelle mischen.
    Merge,
    /// Noten im aufgenommenen Bereich ersetzen (darunterliegende Noten löschen).
    Replace,
}

/// Aufnahme im Cycle.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CycleRecordMode {
    /// Je Durchgang ein Part, der letzte obenauf.
    #[default]
    Stacked,
    /// Alle Durchgänge in einem Part.
    Mix,
    /// Nur der letzte Durchgang bleibt.
    Replace,
}

impl MidiRecordMode {
    pub const ALL: [Self; 3] = [Self::NewPart, Self::Merge, Self::Replace];
    pub fn label(self) -> &'static str {
        match self {
            Self::NewPart => "Neuer Part",
            Self::Merge => "Mischen",
            Self::Replace => "Ersetzen",
        }
    }
}

impl CycleRecordMode {
    pub const ALL: [Self; 3] = [Self::Stacked, Self::Mix, Self::Replace];
    pub fn label(self) -> &'static str {
        match self {
            Self::Stacked => "Gestapelt",
            Self::Mix => "Mix",
            Self::Replace => "Ersetzen",
        }
    }
}

/// Aufnahme-Einstellungen eines Projekts.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, serde::Serialize, serde::Deserialize)]
#[serde(default)]
pub struct RecordSettings {
    pub midi: MidiRecordMode,
    pub cycle_midi: CycleRecordMode,
}

impl RecordSettings {
    pub fn is_default(&self) -> bool {
        *self == Self::default()
    }
}

/// Eine aufgenommene Note in Ticks, mit ihrem Cycle-Durchgang (0 ohne Cycle).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TakeNote {
    pub start: u64,
    pub end: u64,
    pub key: u8,
    pub velocity: u8,
    pub pass: u32,
}

impl Project {
    /// Neuer Part über `range` obenauf mit den Noten darin. `None`, wenn keine hineinfällt.
    fn take_part(
        &mut self,
        track: usize,
        (a, b): (u64, u64),
        notes: &[TakeNote],
        name: &str,
    ) -> Option<EventId> {
        let notes: Vec<Note> = notes
            .iter()
            .filter(|n| n.start >= a && n.start < b)
            .map(|n| Note {
                pitch: n.key.min(127),
                velocity: n.velocity.clamp(1, 127),
                start: (n.start - a) as i64,
                length: n.end.saturating_sub(n.start).max(1),
            })
            .collect();
        if notes.is_empty() {
            return None;
        }
        let id = self.create_event(track, a, b.checked_sub(a)?)?;
        let (t, i) = self.find(id)?;
        // Obenauf: zuletzt in der Liste.
        let mut ev = self.tracks[t].events.remove(i);
        ev.name = name.to_owned();
        ev.notes = notes;
        self.tracks[t].events.push(ev);
        Some(id)
    }

    /// Legt aufgenommene Noten nach den Aufnahmemodi ab. `range`: Bereich des Parts (im
    /// Cycle der Cycle-Bereich). Gibt die neuen bzw. geänderten Parts zurück.
    pub fn place_midi_recording(
        &mut self,
        track: usize,
        notes: &[TakeNote],
        range: (u64, u64),
        cycle: bool,
        settings: RecordSettings,
        name: &str,
    ) -> Vec<EventId> {
        if notes.is_empty() || range.1 <= range.0 || track >= self.tracks.len() {
            return Vec::new();
        }
        if cycle {
            return match settings.cycle_midi {
                CycleRecordMode::Mix => self
                    .take_part(track, range, notes, name)
                    .into_iter()
                    .collect(),
                CycleRecordMode::Replace => {
                    let last = notes.iter().map(|n| n.pass).max().unwrap_or(0);
                    let only: Vec<TakeNote> =
                        notes.iter().copied().filter(|n| n.pass == last).collect();
                    self.take_part(track, range, &only, name)
                        .into_iter()
                        .collect()
                }
                CycleRecordMode::Stacked => {
                    let last = notes.iter().map(|n| n.pass).max().unwrap_or(0);
                    (0..=last)
                        .filter_map(|p| {
                            let pass: Vec<TakeNote> =
                                notes.iter().copied().filter(|n| n.pass == p).collect();
                            self.take_part(track, range, &pass, &format!("{name} ({})", p + 1))
                        })
                        .collect()
                }
            };
        }
        match settings.midi {
            MidiRecordMode::NewPart => self
                .take_part(track, range, notes, name)
                .into_iter()
                .collect(),
            MidiRecordMode::Replace => {
                // Noten, die im aufgenommenen Bereich beginnen, aus allen Parts der Spur.
                let (a, b) = range;
                for ev in &mut self.tracks[track].events {
                    let start = ev.start.0 as i64;
                    ev.notes.retain(|n| {
                        let at = start + n.start;
                        at < a as i64 || at >= b as i64
                    });
                }
                self.take_part(track, range, notes, name)
                    .into_iter()
                    .collect()
            }
            MidiRecordMode::Merge => {
                let (a, b) = range;
                let target = self.tracks[track]
                    .events
                    .iter()
                    .rev()
                    .find(|e| !e.muted && e.clip.is_none() && e.start.0 < b && e.end() > a)
                    .map(|e| e.id);
                let Some(id) = target else {
                    return self
                        .take_part(track, range, notes, name)
                        .into_iter()
                        .collect();
                };
                let Some(ev) = self.event_mut(id) else {
                    return Vec::new();
                };
                // Part bei Bedarf vergrößern, damit alle Noten hineinpassen.
                let new_start = ev.start.0.min(a);
                let shift = (ev.start.0 - new_start) as i64;
                let new_end = ev.end().max(b);
                for n in &mut ev.notes {
                    n.start += shift;
                }
                ev.start = crate::Position(new_start);
                ev.length = new_end - new_start;
                for n in notes {
                    ev.notes.push(Note {
                        pitch: n.key.min(127),
                        velocity: n.velocity.clamp(1, 127),
                        start: n.start as i64 - new_start as i64,
                        length: n.end.saturating_sub(n.start).max(1),
                    });
                }
                ev.notes.sort_by_key(|n| (n.start, n.pitch));
                vec![id]
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::TrackKind;

    fn note(start: u64, key: u8, pass: u32) -> TakeNote {
        TakeNote {
            start,
            end: start + 240,
            key,
            velocity: 100,
            pass,
        }
    }

    fn pitches(p: &Project, id: EventId) -> Vec<(i64, u8)> {
        p.event(id)
            .unwrap()
            .notes
            .iter()
            .map(|n| (n.start, n.pitch))
            .collect()
    }

    #[test]
    fn new_part_merge_and_replace_without_cycle() {
        let mut p = Project::default();
        let t = p.add_track(TrackKind::Instrument, "Synth");
        let old = p.create_event(t, 0, 3840).unwrap();
        p.event_mut(old).unwrap().notes = vec![
            Note {
                pitch: 40,
                velocity: 90,
                start: 0,
                length: 240,
            },
            Note {
                pitch: 41,
                velocity: 90,
                start: 1920,
                length: 240,
            },
        ];
        let take = [note(960, 60, 0)];
        let s = |midi| RecordSettings {
            midi,
            ..Default::default()
        };
        // New part: on top, the old one stays.
        let mut a = p.clone();
        let ids = a.place_midi_recording(
            t,
            &take,
            (0, 3840),
            false,
            s(MidiRecordMode::NewPart),
            "Synth 02",
        );
        assert_eq!(a.tracks[t].events.len(), 2);
        assert_eq!(a.tracks[t].events.last().unwrap().id, ids[0]);
        // Merge: into the old part, sorted.
        let mut m = p.clone();
        let ids = m.place_midi_recording(t, &take, (0, 3840), false, s(MidiRecordMode::Merge), "x");
        assert_eq!(ids, [old]);
        assert_eq!(pitches(&m, old), [(0, 40), (960, 60), (1920, 41)]);
        // Merge beyond the part grows it.
        let mut g = p.clone();
        g.place_midi_recording(
            t,
            &[note(4000, 62, 0)],
            (3840, 7680),
            false,
            s(MidiRecordMode::Merge),
            "x",
        );
        assert_eq!(g.tracks[t].events.len(), 2, "no overlap: a new part");
        let mut g2 = p.clone();
        g2.place_midi_recording(
            t,
            &[note(3000, 62, 0)],
            (1920, 5760),
            false,
            s(MidiRecordMode::Merge),
            "x",
        );
        assert_eq!(g2.event(old).unwrap().length, 5760);
        // Replace: old notes in the range go, new part on top.
        let mut r = p.clone();
        r.place_midi_recording(
            t,
            &take,
            (960, 3840),
            false,
            s(MidiRecordMode::Replace),
            "x",
        );
        assert_eq!(pitches(&r, old), [(0, 40)], "note at 1920 replaced, 0 kept");
        assert_eq!(r.tracks[t].events.len(), 2);
    }

    #[test]
    fn audio_recorded_over_an_event_gets_a_new_lane_and_covers_the_old_one() {
        let mut p = Project::default();
        let t = p.add_track(TrackKind::Audio, "Gesang");
        let c = p.add_clip(crate::PoolClip {
            name: "take".into(),
            path: None,
            sample_rate: 48_000,
            frames: 480_000,
        });
        let old = p.place_take(t, 0, c, 0, 7680, "Gesang 01").unwrap();
        let new = p.place_take(t, 1920, c, 0, 3840, "Gesang 02").unwrap();
        let (lanes, count) = p.tracks[t].lanes();
        assert_eq!(count, 2, "the new take has its own lane");
        assert_ne!(lanes[0], lanes[1]);
        let parts = p.tracks[t].audible_parts();
        let i = |id| p.tracks[t].events.iter().position(|e| e.id == id).unwrap();
        assert_eq!(
            parts[i(old)],
            [(0, 1920), (5760, 7680)],
            "old event kept, silent under the take"
        );
        assert_eq!(parts[i(new)], [(1920, 5760)]);
        assert!(!p.event(old).unwrap().muted);
    }

    #[test]
    fn cycle_passes_stack_mix_or_replace() {
        let mut p = Project::default();
        let t = p.add_track(TrackKind::Instrument, "Synth");
        let take = [note(0, 60, 0), note(960, 62, 1), note(1920, 64, 2)];
        let s = |cycle_midi| RecordSettings {
            cycle_midi,
            ..Default::default()
        };
        let mut st = p.clone();
        let ids = st.place_midi_recording(
            t,
            &take,
            (0, 3840),
            true,
            s(CycleRecordMode::Stacked),
            "Synth",
        );
        assert_eq!(ids.len(), 3, "one part per pass");
        assert_eq!(pitches(&st, ids[2]), [(1920, 64)]);
        assert_eq!(
            st.tracks[t].events.last().unwrap().id,
            ids[2],
            "last pass on top"
        );
        assert_eq!(st.event(ids[0]).unwrap().name, "Synth (1)");
        let mut mx = p.clone();
        let ids =
            mx.place_midi_recording(t, &take, (0, 3840), true, s(CycleRecordMode::Mix), "Synth");
        assert_eq!(pitches(&mx, ids[0]).len(), 3);
        let ids = p.place_midi_recording(
            t,
            &take,
            (0, 3840),
            true,
            s(CycleRecordMode::Replace),
            "Synth",
        );
        assert_eq!(pitches(&p, ids[0]), [(1920, 64)], "only the last pass");
    }
}
