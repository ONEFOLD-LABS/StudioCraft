//! Warp-Marker für Audio, das dem Projekttempo folgt (Issue #55): Ein Marker heftet eine
//! Stelle im Clip (Frame) an eine musikalische Stelle (Ticks ab dem Clip-Anfang). Zwischen
//! zwei Markern wird gleichmäßig gedehnt, außerhalb gilt das Tempo der Datei. So lassen sich
//! Aufnahmen mit schwankendem Tempo ins Raster bringen.
//!
//! Alle Umrechnungen zwischen Clip-Frames und Takten eines solchen Events laufen über
//! [`ContentMap`]; Schneiden, Trimmen und Kleben verschieben den Clip-Offset damit, die Engine
//! dehnt danach.

use crate::{Event, TICKS_PER_QUARTER, TempoMap};

/// Höchstzahl Warp-Marker je Event.
pub const MAX_WARP_POINTS: usize = 1024;
/// Raster, auf das ein neuer Marker seine musikalische Stelle legt (Sechzehntel).
pub const WARP_GRID: u64 = TICKS_PER_QUARTER / 4;

/// Ein Warp-Marker: Frame `frame` des Clips liegt auf `tick` Ticks ab dem Clip-Anfang.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, serde::Serialize, serde::Deserialize)]
pub struct WarpPoint {
    pub frame: u64,
    pub tick: u64,
}

/// Abbildung zwischen Clip-Frames und Ticks ab dem Clip-Anfang für ein Event, das dem Tempo
/// folgt.
#[derive(Debug, Clone, PartialEq)]
pub struct ContentMap {
    /// Ticks je Frame im Tempo der Datei (außerhalb der Marker).
    k: f64,
    warp: Vec<WarpPoint>,
}

impl ContentMap {
    /// Abbildung für Tempo `bpm` der Datei bei `rate`, mit Markern `warp` (bereinigt).
    pub fn new(bpm: f64, rate: u32, warp: &[WarpPoint]) -> Option<Self> {
        if !(bpm.is_finite() && bpm > 0.0 && rate > 0) {
            return None;
        }
        let k = bpm / 60.0 * TICKS_PER_QUARTER as f64 / f64::from(rate);
        let mut warp = warp.to_vec();
        clean(&mut warp);
        Some(Self { k, warp })
    }

    /// Ticks ab dem Clip-Anfang an Frame `f`.
    pub fn tick_at(&self, f: f64) -> f64 {
        let w = &self.warp;
        let (Some(first), Some(last)) = (w.first(), w.last()) else {
            return f * self.k;
        };
        if f <= first.frame as f64 {
            return first.tick as f64 - (first.frame as f64 - f) * self.k;
        }
        if f >= last.frame as f64 {
            return last.tick as f64 + (f - last.frame as f64) * self.k;
        }
        let i = w.partition_point(|p| (p.frame as f64) <= f);
        let (a, b) = (w[i - 1], w[i]);
        let x = (f - a.frame as f64) / (b.frame - a.frame) as f64;
        a.tick as f64 + x * (b.tick - a.tick) as f64
    }

    /// Frame an `t` Ticks ab dem Clip-Anfang (Umkehrung von [`Self::tick_at`]).
    pub fn frame_at(&self, t: f64) -> f64 {
        let w = &self.warp;
        let (Some(first), Some(last)) = (w.first(), w.last()) else {
            return t / self.k;
        };
        if t <= first.tick as f64 {
            return first.frame as f64 - (first.tick as f64 - t) / self.k;
        }
        if t >= last.tick as f64 {
            return last.frame as f64 + (t - last.tick as f64) / self.k;
        }
        let i = w.partition_point(|p| (p.tick as f64) <= t);
        let (a, b) = (w[i - 1], w[i]);
        let x = (t - a.tick as f64) / (b.tick - a.tick) as f64;
        a.frame as f64 + x * (b.frame - a.frame) as f64
    }

    pub fn points(&self) -> &[WarpPoint] {
        &self.warp
    }
}

/// Sortiert die Marker nach Frame und behält nur solche, deren Ticks streng steigen (keine
/// Rückwärts-Abbildung); höchstens [`MAX_WARP_POINTS`].
pub fn clean(warp: &mut Vec<WarpPoint>) {
    warp.sort_by_key(|p| (p.frame, p.tick));
    let mut out: Vec<WarpPoint> = Vec::with_capacity(warp.len().min(MAX_WARP_POINTS));
    for &p in warp.iter() {
        if out.len() >= MAX_WARP_POINTS {
            break;
        }
        if out
            .last()
            .is_none_or(|l| p.frame > l.frame && p.tick > l.tick)
        {
            out.push(p);
        }
    }
    *warp = out;
}

fn to_u64(v: f64) -> u64 {
    if v.is_finite() && v > 0.0 {
        v.round().min(u64::MAX as f64) as u64
    } else {
        0
    }
}

impl Event {
    /// Abbildung des Inhalts, wenn das Event dem Tempo folgt.
    pub fn content_map(&self, rate: u32) -> Option<ContentMap> {
        ContentMap::new(self.musical?, rate, &self.warp)
    }

    /// Clip-Offset, wenn der Anfang des Inhalts um `delta` Ticks wandert (Schneiden, Trimmen).
    /// Folgt das Event dem Tempo, zählt der Inhalt nach seiner Abbildung, sonst über die
    /// Tempo-Map.
    pub fn shifted_offset(&self, tempo: &TempoMap, rate: u32, delta: i64) -> u64 {
        match self.content_map(rate) {
            Some(m) => to_u64(m.frame_at(m.tick_at(self.offset as f64) + delta as f64)),
            None => {
                let f = tempo.frames(delta.unsigned_abs(), rate);
                if delta >= 0 {
                    self.offset.saturating_add(f)
                } else {
                    self.offset.saturating_sub(f)
                }
            }
        }
    }

    /// Ticks, die der Inhalt von Frame `a` bis `b` des Clips dauert.
    pub fn content_span_ticks(&self, tempo: &TempoMap, rate: u32, a: u64, b: u64) -> u64 {
        match self.content_map(rate) {
            Some(m) => to_u64(m.tick_at(b as f64) - m.tick_at(a as f64)),
            None => tempo.ticks(b.saturating_sub(a), rate),
        }
    }
}

impl crate::Project {
    /// Setzt einen Warp-Marker an Frame `frame` des Clips; er rückt auf die nächste Sechzehntel
    /// im Projekt (nach der bisherigen Abbildung). Der erste Marker setzt dazu einen Anker am
    /// Anfang des Events, damit dessen Anfang bleibt, wo er ist. Nur für Events, die dem Tempo
    /// folgen. Gibt den Index des Markers zurück.
    pub fn add_warp(&mut self, id: crate::EventId, frame: u64) -> Option<usize> {
        let rate = self.clip_rate(id)?;
        let ev = self.event(id)?;
        let m = ev.content_map(rate)?;
        let first = ev.warp.is_empty();
        let anchor = WarpPoint {
            frame: ev.offset,
            tick: to_u64(m.tick_at(ev.offset as f64)),
        };
        // Bezug: Ticks ab Clip-Anfang am Event-Anfang (mit Anker genau dessen Wert).
        let base = if first {
            anchor.tick as f64
        } else {
            m.tick_at(ev.offset as f64)
        };
        // Stelle im Projekt, auf die Sechzehntel gerundet, zurück in Ticks ab Clip-Anfang.
        let start = ev.start.0 as f64;
        let at = start + m.tick_at(frame as f64) - base;
        let snapped = (at / WARP_GRID as f64).round() * WARP_GRID as f64;
        let t = snapped - start + base;
        if t < 0.0 {
            return None;
        }
        let tick = to_u64(t);
        let needs_anchor = first && frame != ev.offset;
        let ev = self.event_mut(id)?;
        if ev.warp.len() + 2 > MAX_WARP_POINTS || ev.warp.iter().any(|p| p.tick == tick) {
            return None;
        }
        if needs_anchor {
            if anchor.tick == tick {
                return None;
            }
            ev.warp.push(anchor);
        }
        ev.warp.push(WarpPoint { frame, tick });
        clean(&mut ev.warp);
        ev.warp
            .iter()
            .position(|p| p.frame == frame && p.tick == tick)
    }

    /// Verschiebt Marker `i` auf Frame `frame` (seine musikalische Stelle bleibt), begrenzt
    /// zwischen die Nachbarn.
    pub fn move_warp(&mut self, id: crate::EventId, i: usize, frame: u64) -> bool {
        let Some(ev) = self.event_mut(id) else {
            return false;
        };
        let lo = i
            .checked_sub(1)
            .and_then(|j| ev.warp.get(j))
            .map_or(0, |p| p.frame + 1);
        let hi = ev
            .warp
            .get(i + 1)
            .map_or(u64::MAX, |p| p.frame.saturating_sub(1));
        let Some(p) = ev.warp.get_mut(i) else {
            return false;
        };
        if lo > hi {
            return false;
        }
        p.frame = frame.clamp(lo, hi);
        true
    }

    /// Löscht Marker `i`.
    pub fn remove_warp(&mut self, id: crate::EventId, i: usize) -> bool {
        match self.event_mut(id) {
            Some(ev) if i < ev.warp.len() => {
                ev.warp.remove(i);
                true
            }
            _ => false,
        }
    }

    fn clip_rate(&self, id: crate::EventId) -> Option<u32> {
        let c = self.event(id)?.clip?;
        Some(self.pool.get(c as usize)?.sample_rate)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{PoolClip, Project, TrackKind};

    #[test]
    fn mapping_without_markers_uses_the_file_tempo() {
        // 120 BPM bei 48 kHz: 24 000 Frames sind eine Viertel.
        let m = ContentMap::new(120.0, 48_000, &[]).unwrap();
        assert!((m.tick_at(24_000.0) - 960.0).abs() < 1e-9);
        assert!((m.frame_at(1920.0) - 48_000.0).abs() < 1e-6);
        assert!(ContentMap::new(f64::NAN, 48_000, &[]).is_none());
        assert!(ContentMap::new(120.0, 0, &[]).is_none());
    }

    #[test]
    fn markers_bend_time_between_them_and_invert() {
        // Die zweite Viertel des Clips ist zu spät gespielt: Frame 30 000 gehört auf Tick 960.
        let warp = [
            WarpPoint { frame: 0, tick: 0 },
            WarpPoint {
                frame: 30_000,
                tick: 960,
            },
        ];
        let m = ContentMap::new(120.0, 48_000, &warp).unwrap();
        assert!((m.tick_at(15_000.0) - 480.0).abs() < 1e-9);
        assert!((m.tick_at(30_000.0) - 960.0).abs() < 1e-9);
        // Dahinter wieder im Tempo der Datei.
        assert!((m.tick_at(54_000.0) - 1920.0).abs() < 1e-9);
        for t in [-500.0, 0.0, 300.0, 960.0, 2000.0] {
            assert!((m.tick_at(m.frame_at(t)) - t).abs() < 1e-6, "{t}");
        }
    }

    #[test]
    fn split_and_trim_follow_the_markers() {
        let mut p = Project::default();
        p.add_track(TrackKind::Audio, "A");
        let c = p.add_clip(PoolClip {
            name: "c".into(),
            path: None,
            sample_rate: 48_000,
            frames: 96_000,
        });
        let id = p.add_audio_event(0, 0, c).unwrap();
        p.set_musical(id, Some(120.0));
        // Die erste Viertel ist im Clip 30 000 Frames lang (gespielt etwas zu langsam).
        p.event_mut(id).unwrap().warp = vec![
            WarpPoint { frame: 0, tick: 0 },
            WarpPoint {
                frame: 30_000,
                tick: 960,
            },
        ];
        let right = p.split(id, 960).unwrap();
        assert_eq!(p.event(right).unwrap().offset, 30_000);
        // Eine Achtel weiter hinten anfangen: halbe Strecke bis zum Marker.
        let half = p.split(id, 480).unwrap();
        assert_eq!(p.event(half).unwrap().offset, 15_000);
        assert!(p.trim_start(right, 1200));
        // 240 Ticks hinter dem Marker gilt wieder das Tempo der Datei: 6000 Frames.
        assert_eq!(p.event(right).unwrap().offset, 36_000);
    }

    #[test]
    fn clean_sorts_and_drops_crossing_markers() {
        let mut w = vec![
            WarpPoint {
                frame: 50,
                tick: 50,
            },
            WarpPoint {
                frame: 10,
                tick: 10,
            },
            WarpPoint { frame: 30, tick: 5 },
            WarpPoint {
                frame: 10,
                tick: 20,
            },
        ];
        clean(&mut w);
        assert_eq!(
            w,
            vec![
                WarpPoint {
                    frame: 10,
                    tick: 10
                },
                WarpPoint {
                    frame: 50,
                    tick: 50
                }
            ]
        );
    }

    #[test]
    fn markers_snap_to_sixteenths_and_move_between_neighbours() {
        let mut p = Project::default();
        p.add_track(TrackKind::Audio, "A");
        let c = p.add_clip(PoolClip {
            name: "c".into(),
            path: None,
            sample_rate: 48_000,
            frames: 96_000,
        });
        let id = p.add_audio_event(0, 0, c).unwrap();
        assert_eq!(p.add_warp(id, 1000), None, "nur Audio, das dem Tempo folgt");
        p.set_musical(id, Some(120.0));
        // 25 000 Frames sind 1000 Ticks: nächste Sechzehntel ist 960. Der erste Marker setzt
        // einen Anker am Anfang des Events.
        assert_eq!(p.add_warp(id, 25_000), Some(1));
        assert_eq!(p.add_warp(id, 50_000), Some(2));
        assert_eq!(
            p.event(id).unwrap().warp,
            vec![
                WarpPoint { frame: 0, tick: 0 },
                WarpPoint {
                    frame: 25_000,
                    tick: 960
                },
                WarpPoint {
                    frame: 50_000,
                    tick: 1920
                },
            ]
        );
        // Gleiche Sechzehntel noch einmal: nichts.
        assert_eq!(p.add_warp(id, 24_500), None);
        assert!(p.move_warp(id, 1, 60_000));
        assert_eq!(
            p.event(id).unwrap().warp[1].frame,
            49_999,
            "vor dem Nachbarn"
        );
        assert!(p.remove_warp(id, 2));
        assert!(!p.remove_warp(id, 5));
        assert_eq!(p.event(id).unwrap().warp.len(), 2);
    }

    #[test]
    fn markers_snap_to_the_project_grid_for_events_off_the_clip_start() {
        let mut p = Project::default();
        p.add_track(TrackKind::Audio, "A");
        let c = p.add_clip(PoolClip {
            name: "c".into(),
            path: None,
            sample_rate: 48_000,
            frames: 96_000,
        });
        let id = p.add_audio_event(0, 3840, c).unwrap();
        p.set_musical(id, Some(100.0));
        // Das Event nutzt den Clip ab 5000 Frames; ein Marker 24 000 Frames dahinter liegt
        // nach Dateitempo 800 Ticks nach dem Event-Anfang und rückt auf 720.
        p.event_mut(id).unwrap().offset = 5000;
        assert_eq!(p.add_warp(id, 29_000), Some(1));
        let ev = p.event(id).unwrap();
        let m = ev.content_map(48_000).unwrap();
        let at = ev.start.0 as f64 + m.tick_at(29_000.0) - m.tick_at(5000.0);
        assert!((at - (3840.0 + 720.0)).abs() < 1.0, "{at}");
        // Der Anfang des Events bleibt, wo er war.
        assert_eq!(ev.warp[0].frame, 5000);
    }
}
