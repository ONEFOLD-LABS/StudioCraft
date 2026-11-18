//! Abspielliste für den Audio-Thread: alle hörbaren Audio-Events, in Frames
//! umgerechnet. Wird bei jeder Änderung neu gebaut und als Ganzes ausgetauscht.

use std::{collections::HashMap, sync::Arc};

use studiocraft_project::{ClipId, Project, TrackKind, db_to_gain};

/// Ein Event, wie der Audio-Thread es braucht.
#[derive(Debug, Clone)]
pub struct PlayEvent {
    pub start: u64,
    pub length: u64,
    /// Startpunkt im Clip in Frames.
    pub offset: u64,
    /// Verschachtelte Stereo-Samples in Engine-Abtastrate.
    pub samples: Arc<[f32]>,
    /// Fade-In und Fade-Out in Frames (linear), bezogen auf das ganze Event.
    pub fade_in: u64,
    pub fade_out: u64,
    /// Ist dies nur ein sichtbarer Ausschnitt eines Events (weil ein darüber
    /// liegendes Event den Rest verdeckt): Abstand zum Anfang des ganzen Events
    /// und dessen Länge. Damit verlaufen die Fades wie beim ganzen Event.
    pub env_offset: u64,
    pub env_length: u64,
}

impl PlayEvent {
    pub fn new(start: u64, length: u64, offset: u64, samples: Arc<[f32]>) -> Self {
        Self {
            start,
            length,
            offset,
            samples,
            fade_in: 0,
            fade_out: 0,
            env_offset: 0,
            env_length: length,
        }
    }

    /// Lautstärkefaktor aus den Fades an Frame `k` des Events.
    #[inline]
    fn fade_gain(&self, k: u64) -> f32 {
        let k = k + self.env_offset;
        let mut g = 1.0;
        if k < self.fade_in {
            g = k as f32 / self.fade_in as f32;
        }
        let rest = self.env_length.saturating_sub(k);
        if rest < self.fade_out {
            g *= rest as f32 / self.fade_out as f32;
        }
        g
    }
}

#[derive(Debug, Clone, Default)]
pub struct PlayTrack {
    /// Faktor links und rechts (Fader und Panorama).
    pub gain_l: f32,
    pub gain_r: f32,
    /// Index der Projektspur, für die Pegelanzeige.
    pub slot: usize,
    /// Nach `start` sortiert.
    pub events: Vec<PlayEvent>,
}

#[derive(Debug, Clone)]
pub struct Song {
    pub tracks: Vec<PlayTrack>,
    /// Faktor des Ausgangsbusses.
    pub master: f32,
}

impl Default for Song {
    fn default() -> Self {
        Self {
            tracks: Vec::new(),
            master: 1.0,
        }
    }
}

impl Song {
    /// Ein einzelner Clip ab Frame 0 (für die Kommandozeile).
    pub fn single(samples: Arc<[f32]>) -> Self {
        let length = (samples.len() / 2) as u64;
        Self {
            tracks: vec![PlayTrack {
                gain_l: 1.0,
                gain_r: 1.0,
                slot: 0,
                events: vec![PlayEvent::new(0, length, 0, samples)],
            }],
            master: 1.0,
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
            .map(|(slot, t)| {
                // Wie in klassischen DAWs klingt auf einer Audiospur bei Überlappung nur das
                // oberste Event (das spätere in der Liste). Verdeckte Teile fallen weg.
                let mut events = Vec::new();
                for (i, e) in t.events.iter().enumerate() {
                    if e.muted {
                        continue;
                    }
                    let Some(samples) = e.clip.and_then(|c| clips.get(&c)) else {
                        continue;
                    };
                    let covers = t.events[i + 1..]
                        .iter()
                        .filter(|l| !l.muted)
                        .map(|l| (l.start.0, l.end()));
                    let ev_start = tempo.frames(e.start.0, rate);
                    let env_length = tempo.frames(e.end(), rate) - ev_start;
                    for (a, b) in visible_parts((e.start.0, e.end()), covers) {
                        let (fa, fb) = (tempo.frames(a, rate), tempo.frames(b, rate));
                        if fb <= fa {
                            continue;
                        }
                        let env_offset = fa - ev_start;
                        events.push(PlayEvent {
                            fade_in: tempo.frames(e.fade_in, rate),
                            fade_out: tempo.frames(e.fade_out, rate),
                            env_offset,
                            env_length,
                            ..PlayEvent::new(fa, fb - fa, e.offset + env_offset, samples.clone())
                        });
                    }
                }
                events.sort_by_key(|e| e.start);
                let (gain_l, gain_r) = t.gains();
                PlayTrack {
                    gain_l,
                    gain_r,
                    slot,
                    events,
                }
            })
            .filter(|t| !t.events.is_empty())
            .collect();
        Self {
            tracks,
            master: db_to_gain(project.master_db),
        }
    }

    /// Mischt alle Spuren mit ihren Faktoren und dem Ausgangsfader in `out`
    /// (wird addiert). Für Tests; der Kern mischt Spur für Spur, um Pegel zu messen.
    pub fn mix(&self, out: &mut [f32], pos: u64, frames: usize) {
        let mut tmp = vec![0.0; frames * 2];
        for track in &self.tracks {
            tmp.fill(0.0);
            track.render(&mut tmp, pos, frames);
            for (o, t) in out.chunks_exact_mut(2).zip(tmp.chunks_exact(2)) {
                o[0] += t[0] * track.gain_l * self.master;
                o[1] += t[1] * track.gain_r * self.master;
            }
        }
    }
}

/// Teile von `range`, die von keinem der `covers` verdeckt werden.
fn visible_parts(range: (u64, u64), covers: impl Iterator<Item = (u64, u64)>) -> Vec<(u64, u64)> {
    let mut parts = vec![range];
    for (ca, cb) in covers {
        parts = parts
            .into_iter()
            .flat_map(|(a, b)| {
                if cb <= a || ca >= b {
                    vec![(a, b)]
                } else {
                    [(a, ca.max(a)), (cb.min(b), b)]
                        .into_iter()
                        .filter(|(x, y)| y > x)
                        .collect()
                }
            })
            .collect();
    }
    parts
}

impl PlayTrack {
    /// Schreibt die Events dieser Spur ohne Fader und Panorama in `out`
    /// (wird addiert). Echtzeitsicher: keine Allokation.
    pub fn render(&self, out: &mut [f32], pos: u64, frames: usize) {
        let end = pos + frames as u64;
        // Events sind nach Start sortiert; alles ab `end` kann übersprungen werden.
        let last = self.events.partition_point(|e| e.start < end);
        for ev in &self.events[..last] {
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
            let dst = &mut out[dst_from..dst_from + n];
            if ev.fade_in == 0 && ev.fade_out == 0 {
                for (d, s) in dst.iter_mut().zip(&src[..n]) {
                    *d += s;
                }
            } else {
                let k0 = from - ev.start;
                for (i, (d, s)) in dst
                    .chunks_exact_mut(2)
                    .zip(src[..n].chunks_exact(2))
                    .enumerate()
                {
                    let g = ev.fade_gain(k0 + i as u64);
                    d[0] += s[0] * g;
                    d[1] += s[1] * g;
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

    fn track(gain: f32, slot: usize, events: Vec<PlayEvent>) -> PlayTrack {
        PlayTrack {
            gain_l: gain,
            gain_r: gain,
            slot,
            events,
        }
    }

    #[test]
    fn overlapping_tracks_add_up() {
        let song = Song {
            tracks: vec![
                track(1.0, 0, vec![PlayEvent::new(2, 4, 0, ones(10, 0.25))]),
                track(0.5, 1, vec![PlayEvent::new(0, 3, 0, ones(10, 1.0))]),
            ],
            master: 1.0,
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
            tracks: vec![track(1.0, 0, vec![PlayEvent::new(10, 5, 3, samples)])],
            master: 1.0,
        };
        let mut out = vec![0.0; 8];
        song.mix(&mut out, 12, 4);
        let left: Vec<f32> = out.chunks(2).map(|c| c[0]).collect();
        // Frame 12 ist Event-Frame 2, also Clip-Frame 5; Event endet bei 15.
        assert_eq!(left, [5.0, 6.0, 7.0, 0.0]);
    }

    #[test]
    fn fades_ramp_linearly() {
        let ev = PlayEvent {
            fade_in: 4,
            fade_out: 2,
            ..PlayEvent::new(0, 8, 0, ones(8, 1.0))
        };
        let song = Song {
            tracks: vec![track(1.0, 0, vec![ev])],
            master: 1.0,
        };
        let mut out = vec![0.0; 16];
        song.mix(&mut out, 0, 8);
        let left: Vec<f32> = out.chunks(2).map(|c| c[0]).collect();
        assert_eq!(left, [0.0, 0.25, 0.5, 0.75, 1.0, 1.0, 1.0, 0.5]);
    }

    #[test]
    fn topmost_event_wins_on_overlap() {
        use studiocraft_project::Position;
        let mut p = Project::default();
        p.add_track(TrackKind::Audio, "a");
        let c = p.add_clip(PoolClip {
            name: "c".into(),
            path: None,
            sample_rate: 48_000,
            frames: 192_000,
        });
        let q = TICKS_PER_QUARTER;
        let old = p.add_audio_event(0, 0, c).unwrap(); // 4 s = 8 Viertel
        let new = p.create_event(0, 0, 1).unwrap();
        // Neuer Take obenauf (spätere Position in der Liste) von Viertel 2 bis 4.
        let ev = p.event_mut(new).unwrap();
        ev.clip = Some(c);
        ev.start = Position(2 * q);
        ev.length = 2 * q;
        ev.offset = 0;
        let i = p.find(new).unwrap().1;
        let taken = p.tracks[0].events.remove(i);
        p.tracks[0].events.push(taken);
        p.set_fades(old, 8 * q, 0);
        let clips = HashMap::from([(c, ones(192_000, 1.0))]);
        let song = Song::build(&p, &clips, 48_000);
        let spans: Vec<(u64, u64, u64)> = song.tracks[0]
            .events
            .iter()
            .map(|e| (e.start, e.length, e.offset))
            .collect();
        // Alter Take sichtbar 0–1 s und 2–4 s, neuer Take 1–2 s.
        assert_eq!(
            spans,
            [
                (0, 48_000, 0),
                (48_000, 48_000, 0),
                (96_000, 96_000, 96_000)
            ]
        );
        // Der zweite Teil des alten Takes setzt den Fade-In des ganzen Events fort.
        let tail = &song.tracks[0].events[2];
        assert_eq!(
            (tail.env_offset, tail.env_length, tail.fade_in),
            (96_000, 192_000, 192_000)
        );
        let mut out = vec![0.0; 2];
        song.mix(&mut out, 96_000, 1);
        assert!(
            (out[0] - 0.5).abs() < 1e-4,
            "Fade-In bei halber Länge: {}",
            out[0]
        );
    }

    #[test]
    fn visible_parts_cut_out_covers() {
        assert_eq!(
            visible_parts((0, 10), [(2, 4), (8, 12)].into_iter()),
            [(0, 2), (4, 8)]
        );
        assert_eq!(visible_parts((0, 10), [(0, 10)].into_iter()), []);
        assert_eq!(visible_parts((5, 10), [(0, 3)].into_iter()), [(5, 10)]);
    }

    #[test]
    fn build_respects_mute_solo_fader_and_converts_positions() {
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
        p.tracks[1].pan = 1.0;
        p.master_db = -6.0;
        let clips = HashMap::from([(c, ones(100, 1.0))]);

        let song = Song::build(&p, &clips, 48_000);
        assert_eq!(song.tracks.len(), 2);
        assert_eq!(
            song.tracks[0].events[0].start, 24_000,
            "Viertel bei 120 BPM"
        );
        assert_eq!(
            (
                song.tracks[1].gain_l,
                song.tracks[1].gain_r,
                song.tracks[1].slot
            ),
            (0.0, 1.0, 1)
        );
        assert!((song.master - 0.501).abs() < 0.001);

        p.tracks[1].solo = true;
        assert_eq!(Song::build(&p, &clips, 48_000).tracks.len(), 1);
        p.tracks[1].mute = true;
        assert_eq!(Song::build(&p, &clips, 48_000).tracks.len(), 0);
    }
}
