//! Automation beim Abspielen (Spec „Automation“ → Engine-Hinweis, Issue #25).
//!
//! [`Song::build`](crate::Song::build) übersetzt die Kurven einer Spur mit R in Hüllkurven in
//! Frames. Der Audio-Thread wertet sie sample-genau aus: Lautstärke, Pan und Mute als Faktor
//! pro Frame, Plugin-Parameter als zeitgestempelte CLAP-Ereignisse im Block.

use studiocraft_project::{AutoParam, AutomationLane, TempoMap, Track, db_to_gain};

/// Abstand der Parameter-Ereignisse an Plugins, während sich eine Kurve ändert.
pub const PARAM_STEP_FRAMES: usize = 128;

/// Eine Kurve in Frames.
#[derive(Debug, Clone, PartialEq)]
pub struct Envelope {
    /// Nach Frame sortiert.
    points: Vec<(u64, f64)>,
}

impl Envelope {
    pub fn from_lane(lane: &AutomationLane, tempo: &TempoMap, rate: u32) -> Option<Self> {
        if !lane.active() {
            return None;
        }
        let mut points: Vec<(u64, f64)> = lane
            .points
            .iter()
            .filter(|p| p.value.is_finite())
            .map(|p| (tempo.frames(p.tick, rate), p.value))
            .collect();
        // Zwei Punkte können auf denselben Frame fallen; der spätere gewinnt.
        points.dedup_by(|b, a| {
            if a.0 == b.0 {
                a.1 = b.1;
                true
            } else {
                false
            }
        });
        (!points.is_empty()).then_some(Self { points })
    }

    /// Wert bei `frame` (linear zwischen den Punkten, an den Enden gehalten). Echtzeitsicher.
    pub fn value_at(&self, frame: u64) -> f64 {
        let i = self.points.partition_point(|p| p.0 <= frame);
        match (i.checked_sub(1).map(|j| self.points[j]), self.points.get(i)) {
            (Some(a), Some(b)) => {
                let f = (frame - a.0) as f64 / (b.0 - a.0) as f64;
                a.1 + (b.1 - a.1) * f
            }
            (Some(a), None) => a.1,
            (None, Some(b)) => b.1,
            (None, None) => 0.0,
        }
    }

    /// Ob die Kurve in `from..to` konstant ist.
    fn flat(&self, from: u64, to: u64) -> bool {
        let a = self.value_at(from);
        let i = self.points.partition_point(|p| p.0 <= from);
        self.points[i..]
            .iter()
            .take_while(|p| p.0 <= to)
            .all(|p| p.1 == a)
            && self.value_at(to) == a
    }
}

/// Automation einer Spur fürs Abspielen.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct TrackAuto {
    pub volume: Option<Envelope>,
    pub pan: Option<Envelope>,
    pub mute: Option<Envelope>,
    /// Statische Werte für Parameter ohne Kurve.
    pub volume_db: f32,
    pub pan_static: f32,
    /// Plugin-Parameter: Insert (`usize::MAX` = Instrument), Parameter-Id, Kurve.
    pub plugins: Vec<(usize, u32, Envelope)>,
    /// Send-Pegel in dB: Send im Projekt, Kurve.
    pub sends: Vec<(usize, Envelope)>,
}

impl TrackAuto {
    /// `None`, wenn die Spur nicht liest (R aus) oder keine wirksame Kurve hat.
    pub fn build(track: &Track, tempo: &TempoMap, rate: u32) -> Option<Box<Self>> {
        if !track.read {
            return None;
        }
        let mut auto = Self {
            volume_db: track.volume_db,
            pan_static: track.pan,
            ..Self::default()
        };
        for lane in &track.automation {
            let Some(env) = Envelope::from_lane(lane, tempo, rate) else {
                continue;
            };
            match lane.param {
                AutoParam::Volume => auto.volume = Some(env),
                AutoParam::Pan => auto.pan = Some(env),
                AutoParam::Mute => auto.mute = Some(env),
                AutoParam::Send { index } => auto.sends.push((index as usize, env)),
                AutoParam::Plugin { slot, id } => {
                    let slot = if slot == u32::MAX {
                        usize::MAX
                    } else {
                        slot as usize
                    };
                    auto.plugins.push((slot, id, env));
                }
            }
        }
        let any = auto.volume.is_some()
            || auto.pan.is_some()
            || auto.mute.is_some()
            || !auto.plugins.is_empty()
            || !auto.sends.is_empty();
        any.then(|| Box::new(auto))
    }

    /// Ob Fader, Pan oder Mute automatisiert sind.
    pub fn has_gain(&self) -> bool {
        self.volume.is_some() || self.pan.is_some() || self.mute.is_some()
    }

    /// Faktoren links und rechts für `frames` Frames ab `pos`. Echtzeitsicher.
    pub fn fill_gains(&self, pos: u64, left: &mut [f32], right: &mut [f32]) {
        for (i, (l, r)) in left.iter_mut().zip(right.iter_mut()).enumerate() {
            let f = pos + i as u64;
            let muted = self.mute.as_ref().is_some_and(|m| m.value_at(f) >= 0.5);
            if muted {
                *l = 0.0;
                *r = 0.0;
                continue;
            }
            let db = self
                .volume
                .as_ref()
                .map_or(self.volume_db, |v| v.value_at(f) as f32);
            let pan = self
                .pan
                .as_ref()
                .map_or(self.pan_static, |p| p.value_at(f) as f32)
                .clamp(-1.0, 1.0);
            let g = db_to_gain(db);
            *l = g * if pan > 0.0 { 1.0 - pan } else { 1.0 };
            *r = g * if pan < 0.0 { 1.0 + pan } else { 1.0 };
        }
    }

    /// Faktor des Sends `model` (Platz im Projekt) für jeden Frame ab `pos`; `false`, wenn er
    /// nicht automatisiert ist. Echtzeitsicher.
    pub fn fill_send_gain(&self, model: usize, pos: u64, out: &mut [f32]) -> bool {
        let Some((_, env)) = self.sends.iter().find(|(m, _)| *m == model) else {
            return false;
        };
        for (i, g) in out.iter_mut().enumerate() {
            *g = db_to_gain(env.value_at(pos + i as u64) as f32);
        }
        true
    }

    /// Parameter-Ereignisse für `frames` Frames ab `pos`: `emit(slot, zeit, id, wert)`.
    /// Ein Ereignis am Blockanfang, dazu alle [`PARAM_STEP_FRAMES`], solange sich die Kurve
    /// ändert. Echtzeitsicher.
    pub fn param_events(
        &self,
        pos: u64,
        frames: usize,
        mut emit: impl FnMut(usize, u32, u32, f64),
    ) {
        let end = pos + frames as u64;
        for (slot, id, env) in &self.plugins {
            emit(*slot, 0, *id, env.value_at(pos));
            if env.flat(pos, end) {
                continue;
            }
            for t in (PARAM_STEP_FRAMES..frames).step_by(PARAM_STEP_FRAMES) {
                emit(*slot, t as u32, *id, env.value_at(pos + t as u64));
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use studiocraft_project::{Project, TrackKind};

    fn track_with(param: AutoParam, points: &[(u64, f64)]) -> Track {
        let mut p = Project::default();
        let t = p.add_track(TrackKind::Audio, "A");
        let lane = p.tracks[t].automation_lane_mut(param);
        for &(tick, v) in points {
            lane.set_point(tick, v);
        }
        p.tracks[t].read = true;
        p.tracks.remove(t)
    }

    #[test]
    fn nothing_without_read_or_points() {
        let tempo = TempoMap::default();
        let mut t = track_with(AutoParam::Volume, &[(0, -6.0)]);
        assert!(TrackAuto::build(&t, &tempo, 48_000).is_some());
        t.read = false;
        assert!(TrackAuto::build(&t, &tempo, 48_000).is_none());
        let empty = track_with(AutoParam::Volume, &[]);
        assert!(TrackAuto::build(&empty, &tempo, 48_000).is_none());
        let mut muted = track_with(AutoParam::Volume, &[(0, -6.0)]);
        muted.automation[0].muted = true;
        assert!(TrackAuto::build(&muted, &tempo, 48_000).is_none());
    }

    #[test]
    fn gains_follow_the_curve_sample_by_sample() {
        // 120 bpm, 48 kHz: 960 ticks = 0.5 s = 24 000 frames.
        let t = track_with(AutoParam::Volume, &[(0, -60.0), (960, 0.0)]);
        let a = TrackAuto::build(&t, &TempoMap::default(), 48_000).expect("auto");
        let (mut l, mut r) = ([0.0f32; 4], [0.0f32; 4]);
        a.fill_gains(0, &mut l, &mut r);
        assert_eq!(l[0], 0.0, "-60 dB is silence");
        a.fill_gains(24_000, &mut l, &mut r);
        assert!((l[0] - 1.0).abs() < 1e-6 && (r[3] - 1.0).abs() < 1e-6);
        a.fill_gains(12_000, &mut l, &mut r);
        assert!((l[0] - db_to_gain(-30.0)).abs() < 1e-6);
        assert!(l[1] > l[0], "rises within the block");
    }

    #[test]
    fn pan_and_mute_shape_both_sides() {
        let mut t = track_with(AutoParam::Pan, &[(0, 1.0)]);
        t.automation_lane_mut(AutoParam::Mute).set_point(960, 1.0);
        t.automation_lane_mut(AutoParam::Mute).set_point(0, 0.0);
        let a = TrackAuto::build(&t, &TempoMap::default(), 48_000).expect("auto");
        let (mut l, mut r) = ([0.0f32; 2], [0.0f32; 2]);
        a.fill_gains(0, &mut l, &mut r);
        assert_eq!((l[0], r[0]), (0.0, 1.0));
        a.fill_gains(24_000, &mut l, &mut r);
        assert_eq!((l[0], r[0]), (0.0, 0.0));
    }

    #[test]
    fn send_levels_follow_their_curve() {
        let t = track_with(AutoParam::Send { index: 1 }, &[(0, -60.0), (960, 0.0)]);
        let a = TrackAuto::build(&t, &TempoMap::default(), 48_000).expect("auto");
        let mut g = [0.0f32; 2];
        assert!(!a.fill_send_gain(0, 0, &mut g), "send 0 is not automated");
        assert!(a.fill_send_gain(1, 24_000, &mut g));
        assert!((g[0] - 1.0).abs() < 1e-6);
        assert!(a.fill_send_gain(1, 0, &mut g));
        assert_eq!(g[0], 0.0);
    }

    #[test]
    fn plugin_events_are_time_stamped_only_while_the_curve_moves() {
        let mut t = track_with(
            AutoParam::Plugin { slot: 0, id: 7 },
            &[(0, 0.0), (960, 1.0)],
        );
        t.automation_lane_mut(AutoParam::Plugin {
            slot: u32::MAX,
            id: 3,
        })
        .set_point(0, 0.25);
        let a = TrackAuto::build(&t, &TempoMap::default(), 48_000).expect("auto");
        let mut got = Vec::new();
        a.param_events(0, 512, |slot, time, id, v| got.push((slot, time, id, v)));
        let ramp: Vec<u32> = got.iter().filter(|e| e.2 == 7).map(|e| e.1).collect();
        assert_eq!(ramp, [0, 128, 256, 384]);
        let flat: Vec<_> = got.iter().filter(|e| e.2 == 3).collect();
        assert_eq!(flat, [&(usize::MAX, 0, 3, 0.25)]);
        let at_256 = got
            .iter()
            .find(|e| e.2 == 7 && e.1 == 256)
            .map_or(0.0, |e| e.3);
        assert!((at_256 - 256.0 / 24_000.0).abs() < 1e-9);
    }
}
