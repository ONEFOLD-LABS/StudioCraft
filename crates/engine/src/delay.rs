//! Latenzausgleich (Issue #51): Verzögerungen, damit alle Wege gleich spät am Ausgang ankommen.
//!
//! Plugins mit Latenz verspäten ihre Spur. Jeder Weg (Spur → Ausgang/Gruppe, Spur → Send →
//! FX-Kanal) wird so verzögert, dass alle Signale an jedem Bus-Eingang und am Ausgang
//! gleichzeitig ankommen. Die Verzögerungsleitungen werden im Hauptthread angelegt
//! ([`DelayBank`]) und per Befehl in die Engine gegeben; der Audio-Thread allokiert nichts.

use crate::song::{Output, Song};

/// Stereo-Verzögerung um eine feste Zahl Frames (verschachtelt L, R).
pub struct DelayLine {
    buf: Box<[f32]>,
    pos: usize,
}

impl DelayLine {
    /// Verzögerung um `frames` Frames; `None` bei 0.
    pub fn new(frames: u32) -> Option<Self> {
        (frames > 0).then(|| Self {
            buf: vec![0.0; frames as usize * 2].into_boxed_slice(),
            pos: 0,
        })
    }

    pub fn frames(&self) -> usize {
        self.buf.len() / 2
    }

    /// Verzögert `io` (verschachteltes Stereo) an Ort und Stelle. Keine Allokation.
    pub fn process(&mut self, io: &mut [f32]) {
        let len = self.buf.len();
        if len == 0 {
            return;
        }
        for s in io.iter_mut() {
            std::mem::swap(&mut self.buf[self.pos], s);
            self.pos += 1;
            if self.pos == len {
                self.pos = 0;
            }
        }
    }
}

/// Verzögerungen eines Wegs in Frames.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct TrackDelay {
    /// Gemeinsam für alle Wege der Spur (direkt hinter Inserts und EQ).
    pub common: u32,
    /// Zusätzlich für den Ausgang.
    pub direct: u32,
    /// Zusätzlich je Send (in der Reihenfolge von `PlayTrack::sends`).
    pub sends: Vec<u32>,
    /// MIDI-Spuren an externe Geräte: um so viel später senden (statt Verzögerungsleitung).
    pub midi: u32,
}

/// Ergebnis des Latenzausgleichs: Verzögerung je Spur (nach Projektspur) und die Latenz am
/// Ausgang.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Compensation {
    pub tracks: Vec<TrackDelay>,
    pub total: u32,
}

impl Compensation {
    /// Berechnet die Verzögerungen für `song` mit der Plugin-Latenz `latency[slot]` je
    /// Projektspur. Spuren ohne Eintrag zählen mit 0.
    pub fn compute(song: &Song, latency: &[u32], track_count: usize) -> Self {
        let lat = |slot: usize| latency.get(slot).copied().unwrap_or(0);
        // Leser je Bus: die Gruppe bzw. der FX-Kanal, der aus dem Bus-Puffer liest.
        let reader = |bus: usize| song.tracks.iter().position(|t| t.bus == Some(bus));
        // Früheste Ankunft am Eingang (e) und am Ausgang (o) jeder Spur, in Song-Reihenfolge
        // (Busse stehen hinter allem, was hineinspeist).
        let n = song.tracks.len();
        let mut e = vec![0u64; n];
        let mut o = vec![0u64; n];
        for i in 0..n {
            o[i] = e[i] + u64::from(lat(song.tracks[i].slot));
            let t = &song.tracks[i];
            let mut feed = |bus: usize| {
                if let Some(r) = reader(bus)
                    && r > i
                {
                    e[r] = e[r].max(o[i]);
                }
            };
            if let Output::Bus(b) = t.output {
                feed(b);
            }
            for s in &t.sends {
                feed(s.bus);
            }
        }
        let total = (0..n)
            .filter(|&i| song.tracks[i].output == Output::Master)
            .map(|i| o[i])
            .max()
            .unwrap_or(0);
        let mut tracks = vec![TrackDelay::default(); track_count];
        for (i, t) in song.tracks.iter().enumerate() {
            let arrive = |bus: usize| reader(bus).map_or(o[i], |r| e[r]);
            let direct = match t.output {
                Output::Master => total,
                Output::Bus(b) => arrive(b),
                Output::Nowhere => o[i],
            }
            .saturating_sub(o[i]);
            let sends: Vec<u64> = t
                .sends
                .iter()
                .map(|s| arrive(s.bus).saturating_sub(o[i]))
                .collect();
            let common = sends.iter().copied().chain([direct]).min().unwrap_or(0);
            let clamp = |v: u64| u32::try_from(v).unwrap_or(u32::MAX);
            if let Some(slot) = tracks.get_mut(t.slot) {
                *slot = if t.midi_out.is_some() {
                    // Externe Klangerzeuger: Noten so spät senden, wie der Rest am Ausgang ist.
                    TrackDelay {
                        midi: clamp(total.saturating_sub(o[i])),
                        ..TrackDelay::default()
                    }
                } else {
                    TrackDelay {
                        common: clamp(common),
                        direct: clamp(direct - common),
                        sends: sends.iter().map(|&s| clamp(s - common)).collect(),
                        midi: 0,
                    }
                };
            }
        }
        Self {
            tracks,
            total: u32::try_from(total).unwrap_or(u32::MAX),
        }
    }

    /// Ob irgendwo verzögert wird.
    pub fn is_empty(&self) -> bool {
        self.tracks.iter().all(|t| {
            t.common == 0 && t.direct == 0 && t.midi == 0 && t.sends.iter().all(|&s| s == 0)
        })
    }
}

/// Verzögerungsleitungen je Projektspur (im Hauptthread angelegt).
pub struct TrackLines {
    pub common: Option<DelayLine>,
    pub direct: Option<DelayLine>,
    pub sends: Vec<Option<DelayLine>>,
    /// MIDI an externe Geräte: Versatz in Frames.
    pub midi: u32,
}

/// Alle Verzögerungsleitungen der Engine.
#[derive(Default)]
pub struct DelayBank {
    pub tracks: Vec<TrackLines>,
}

impl std::fmt::Debug for DelayBank {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "DelayBank({} Spuren)", self.tracks.len())
    }
}

impl DelayBank {
    pub fn new(c: &Compensation) -> Self {
        Self {
            tracks: c
                .tracks
                .iter()
                .map(|t| TrackLines {
                    common: DelayLine::new(t.common),
                    direct: DelayLine::new(t.direct),
                    sends: t.sends.iter().map(|&s| DelayLine::new(s)).collect(),
                    midi: t.midi,
                })
                .collect(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::song::{PlaySend, PlayTrack};

    #[test]
    fn a_delay_line_delays_by_its_length() {
        let mut d = DelayLine::new(3).unwrap();
        let mut a: Vec<f32> = (1..=8).flat_map(|i| [i as f32, -(i as f32)]).collect();
        d.process(&mut a[..6]);
        d.process(&mut a[6..]);
        let left: Vec<f32> = a.chunks(2).map(|c| c[0]).collect();
        assert_eq!(left, [0.0, 0.0, 0.0, 1.0, 2.0, 3.0, 4.0, 5.0]);
        assert!(DelayLine::new(0).is_none());
    }

    fn track(slot: usize, output: Output, bus: Option<usize>, sends: Vec<PlaySend>) -> PlayTrack {
        PlayTrack {
            slot,
            output,
            bus,
            sends,
            ..PlayTrack::default()
        }
    }

    #[test]
    fn midi_to_external_devices_is_sent_later_by_the_output_latency() {
        let ext = PlayTrack {
            slot: 1,
            midi_out: Some((0, 0)),
            ..PlayTrack::default()
        };
        let song = Song {
            tracks: vec![track(0, Output::Master, None, vec![]), ext],
            ..Song::default()
        };
        let c = Compensation::compute(&song, &[480, 0], 2);
        assert_eq!(c.total, 480);
        assert_eq!(c.tracks[1].midi, 480, "notes leave 480 frames later");
        assert_eq!(
            (c.tracks[1].common, c.tracks[1].direct),
            (0, 0),
            "no delay line"
        );
        assert_eq!(DelayBank::new(&c).tracks[1].midi, 480);
    }

    #[test]
    fn paths_through_groups_and_sends_arrive_together() {
        let send = |bus| PlaySend {
            bus,
            gain: 1.0,
            pre_fader: false,
            model: 0,
        };
        // 0: plain track → master; 1: track with 100 frames → group (bus 0);
        // 2: track → group, sending to the FX (bus 1); 3: group (50) → master;
        // 4: FX channel (30) → master.
        let song = Song {
            tracks: vec![
                track(0, Output::Master, None, vec![]),
                track(1, Output::Bus(0), None, vec![]),
                track(2, Output::Bus(0), None, vec![send(1)]),
                track(3, Output::Master, Some(0), vec![]),
                track(4, Output::Master, Some(1), vec![]),
            ],
            ..Song::default()
        };
        let c = Compensation::compute(&song, &[0, 100, 0, 50, 30], 5);
        // Group input at 100 (track 1), group output at 150 = total.
        assert_eq!(c.total, 150);
        assert_eq!(c.tracks[0].common + c.tracks[0].direct, 150);
        assert_eq!(c.tracks[1].common + c.tracks[1].direct, 0);
        // Track 2: the direct path waits 100 to meet track 1 at the group; its send reaches
        // the FX at once, and the FX output waits 120 (30 + 120 = 150).
        assert_eq!(c.tracks[2].common + c.tracks[2].direct, 100);
        assert_eq!(c.tracks[2].common, 0);
        assert_eq!(c.tracks[2].sends, [0]);
        assert_eq!(c.tracks[3].common + c.tracks[3].direct, 0);
        assert_eq!(c.tracks[4].common + c.tracks[4].direct, 120);
        // Every path reaches the master at 150: own latency + delays along the way.
        let at_master = |path: &[(usize, u32)]| path.iter().map(|&(_, d)| d).sum::<u32>();
        assert_eq!(at_master(&[(0, 150)]), 150);
        assert_eq!(at_master(&[(1, 100), (3, 50)]), 150);
        assert_eq!(at_master(&[(2, 100), (3, 50)]), 150);
        assert_eq!(at_master(&[(2, 0), (4, 30 + 120)]), 150);
        // Without latency nothing is delayed.
        assert!(Compensation::compute(&song, &[], 5).is_empty());
    }
}
