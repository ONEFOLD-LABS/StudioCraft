//! Abspielliste für den Audio-Thread: alle hörbaren Audio-Events, in Frames
//! umgerechnet. Wird bei jeder Änderung neu gebaut und als Ganzes ausgetauscht.

use std::{collections::HashMap, sync::Arc};

use studiocraft_project::{ClipId, Project, TrackKind};

/// Ein Event, wie der Audio-Thread es braucht.
#[derive(Debug, Clone)]
pub struct PlayEvent {
    pub start: u64,
    pub length: u64,
    /// Startpunkt im Clip in Frames.
    pub offset: u64,
    /// Verschachtelte Stereo-Samples in Engine-Abtastrate.
    pub samples: Arc<[f32]>,
}

#[derive(Debug, Clone, Default)]
pub struct PlayTrack {
    pub gain: f32,
    /// Nach `start` sortiert.
    pub events: Vec<PlayEvent>,
}

#[derive(Debug, Clone, Default)]
pub struct Song {
    pub tracks: Vec<PlayTrack>,
}

impl Song {
    /// Ein einzelner Clip ab Frame 0 (für die Kommandozeile).
    pub fn single(samples: Arc<[f32]>) -> Self {
        let length = (samples.len() / 2) as u64;
        Self {
            tracks: vec![PlayTrack {
                gain: 1.0,
                events: vec![PlayEvent {
                    start: 0,
                    length,
                    offset: 0,
                    samples,
                }],
            }],
        }
    }

    /// Baut die Abspielliste aus dem Projekt. `clips` liefert die Samples je
    /// Clip, bereits in `rate`. Stumme Spuren und Events fallen weg; Solo wirkt.
    pub fn build(project: &Project, clips: &HashMap<ClipId, Arc<[f32]>>, rate: u32) -> Self {
        let tempo = &project.tempo;
        let tracks = project
            .tracks
            .iter()
            .enumerate()
            .filter(|(i, t)| t.kind == TrackKind::Audio && project.audible(*i))
            .map(|(_, t)| {
                let mut events: Vec<PlayEvent> = t
                    .events
                    .iter()
                    .filter(|e| !e.muted)
                    .filter_map(|e| {
                        let samples = clips.get(&e.clip?)?.clone();
                        Some(PlayEvent {
                            start: tempo.frames(e.start.0, rate),
                            length: tempo.frames(e.length, rate),
                            offset: e.offset,
                            samples,
                        })
                    })
                    .collect();
                events.sort_by_key(|e| e.start);
                PlayTrack { gain: 1.0, events }
            })
            .filter(|t| !t.events.is_empty())
            .collect();
        Self { tracks }
    }

    /// Mischt `frames` Frames ab Projektposition `pos` in `out` (wird addiert).
    /// Echtzeitsicher: keine Allokation.
    pub fn mix(&self, out: &mut [f32], pos: u64, frames: usize) {
        let end = pos + frames as u64;
        for track in &self.tracks {
            let g = track.gain;
            // Events beginnen sortiert; alles ab `end` kann übersprungen werden.
            let last = track.events.partition_point(|e| e.start < end);
            for ev in &track.events[..last] {
                let ev_end = ev.start + ev.length;
                if ev_end <= pos {
                    continue;
                }
                let from = pos.max(ev.start);
                let to = end.min(ev_end);
                let src_from = (ev.offset + (from - ev.start)) as usize * 2;
                let n = (to - from) as usize * 2;
                let Some(src) = ev.samples.get(src_from..) else {
                    continue;
                };
                let n = n.min(src.len());
                let dst_from = (from - pos) as usize * 2;
                for (d, s) in out[dst_from..dst_from + n].iter_mut().zip(&src[..n]) {
                    *d += s * g;
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use studiocraft_project::{PoolClip, TICKS_PER_QUARTER};

    fn ones(frames: usize, v: f32) -> Arc<[f32]> {
        vec![v; frames * 2].into()
    }

    #[test]
    fn overlapping_tracks_add_up() {
        let song = Song {
            tracks: vec![
                PlayTrack {
                    gain: 1.0,
                    events: vec![PlayEvent {
                        start: 2,
                        length: 4,
                        offset: 0,
                        samples: ones(10, 0.25),
                    }],
                },
                PlayTrack {
                    gain: 0.5,
                    events: vec![PlayEvent {
                        start: 0,
                        length: 3,
                        offset: 0,
                        samples: ones(10, 1.0),
                    }],
                },
            ],
        };
        let mut out = vec![0.0; 16];
        song.mix(&mut out, 0, 8);
        let left: Vec<f32> = out.chunks(2).map(|c| c[0]).collect();
        assert_eq!(left, [0.5, 0.5, 0.75, 0.25, 0.25, 0.25, 0.0, 0.0]);
    }

    #[test]
    fn offset_and_block_start_inside_event() {
        let samples: Arc<[f32]> = (0..20)
            .flat_map(|i| [i as f32, 0.0])
            .collect::<Vec<_>>()
            .into();
        let song = Song {
            tracks: vec![PlayTrack {
                gain: 1.0,
                events: vec![PlayEvent {
                    start: 10,
                    length: 5,
                    offset: 3,
                    samples,
                }],
            }],
        };
        let mut out = vec![0.0; 8];
        song.mix(&mut out, 12, 4);
        let left: Vec<f32> = out.chunks(2).map(|c| c[0]).collect();
        // Frame 12 ist Event-Frame 2, also Clip-Frame 5; Event endet bei 15.
        assert_eq!(left, [5.0, 6.0, 7.0, 0.0]);
    }

    #[test]
    fn build_respects_mute_solo_and_converts_positions() {
        let mut p = Project::default();
        for n in ["a", "b"] {
            p.add_track(TrackKind::Audio, n);
        }
        let c = p.add_clip(PoolClip {
            name: "c".into(),
            path: None,
            sample_rate: 48_000,
            frames: 100,
        });
        p.add_audio_event(0, TICKS_PER_QUARTER, c).unwrap();
        p.add_audio_event(1, 0, c).unwrap();
        let clips = HashMap::from([(c, ones(100, 1.0))]);

        let song = Song::build(&p, &clips, 48_000);
        assert_eq!(song.tracks.len(), 2);
        assert_eq!(
            song.tracks[0].events[0].start, 24_000,
            "Viertel bei 120 BPM"
        );

        p.tracks[1].solo = true;
        assert_eq!(Song::build(&p, &clips, 48_000).tracks.len(), 1);
        p.tracks[1].mute = true;
        assert_eq!(Song::build(&p, &clips, 48_000).tracks.len(), 0);
    }
}
