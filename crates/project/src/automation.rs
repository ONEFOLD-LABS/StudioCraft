//! Automation (Spec „Automation“, Issue #25): Kurven für Parameter eines Kanals oder Plugins.
//!
//! Jede Kurve liegt auf einer Automationsspur unter ihrer Spur. Punkte stehen in Ticks und in
//! der natürlichen Einheit des Parameters: Lautstärke in dB, Pan −1…+1, Mute 0/1, Plugin-
//! Parameter im Wertebereich des Plugins. Zwischen zwei Punkten wird linear interpoliert; vor
//! dem ersten und nach dem letzten Punkt gilt dessen Wert.

use crate::{InsertSlot, MAX_DB, MIN_DB, Track};

/// Welcher Parameter automatisiert wird.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, serde::Serialize, serde::Deserialize)]
pub enum AutoParam {
    Volume,
    Pan,
    Mute,
    /// Pegel des Sends `index` (in dB, wie der Send-Regler).
    Send {
        index: u32,
    },
    /// Parameter `id` des Inserts `slot`; `slot == u32::MAX` ist das Instrument.
    Plugin {
        slot: u32,
        id: u32,
    },
}

impl AutoParam {
    /// Wertebereich der Kanal-Parameter (Plugins: unbekannt, `None`).
    pub fn range(self) -> Option<(f64, f64)> {
        match self {
            AutoParam::Volume => Some((f64::from(MIN_DB), f64::from(MAX_DB))),
            AutoParam::Pan => Some((-1.0, 1.0)),
            AutoParam::Mute => Some((0.0, 1.0)),
            AutoParam::Send { .. } => Some((f64::from(MIN_DB), f64::from(MAX_DB))),
            AutoParam::Plugin { .. } => None,
        }
    }

    /// Kurzer Name für die Automationsspur.
    pub fn label(self) -> String {
        match self {
            AutoParam::Volume => "Lautstärke".into(),
            AutoParam::Pan => "Pan".into(),
            AutoParam::Mute => "Mute".into(),
            AutoParam::Send { index } => format!("Send {}", u64::from(index) + 1),
            AutoParam::Plugin { slot: u32::MAX, id } => format!("Instrument · Parameter {id}"),
            AutoParam::Plugin { slot, id } => format!("Insert {} · Parameter {id}", slot + 1),
        }
    }

    /// Der statische Wert des Parameters auf `track` (ohne Automation).
    pub fn static_value(self, track: &Track) -> Option<f64> {
        match self {
            AutoParam::Volume => Some(f64::from(track.volume_db)),
            AutoParam::Pan => Some(f64::from(track.pan)),
            AutoParam::Mute => Some(if track.mute { 1.0 } else { 0.0 }),
            AutoParam::Send { index } => track
                .sends
                .get(index as usize)
                .map(|s| f64::from(s.level_db)),
            AutoParam::Plugin { slot, id } => {
                let ins = if slot == u32::MAX {
                    track.instrument.as_ref()
                } else {
                    track.inserts.get(slot as usize)
                }?;
                ins.params.iter().find(|(p, _)| *p == id).map(|(_, v)| *v)
            }
        }
    }
}

/// Ein Punkt einer Kurve.
#[derive(Debug, Clone, Copy, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct AutoPoint {
    pub tick: u64,
    pub value: f64,
}

/// Eine Automationsspur.
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct AutomationLane {
    pub param: AutoParam,
    /// Nach `tick` sortiert, jeder Tick höchstens einmal.
    pub points: Vec<AutoPoint>,
    /// Stumm: der Parameter steht auf seinem statischen Wert.
    #[serde(default)]
    pub muted: bool,
    /// Unter der Spur aufgeklappt.
    #[serde(default)]
    pub visible: bool,
    /// Wird gerade geschrieben (W): beim Abspielen gilt dann der Wert des Bedienelements,
    /// nicht die Kurve. Wird nicht gespeichert.
    #[serde(skip)]
    pub writing: bool,
}

impl AutomationLane {
    pub fn new(param: AutoParam) -> Self {
        Self {
            param,
            points: Vec::new(),
            muted: false,
            visible: true,
            writing: false,
        }
    }

    /// Wert bei `tick`; `None` ohne Punkte.
    pub fn value_at(&self, tick: u64) -> Option<f64> {
        let i = self.points.partition_point(|p| p.tick <= tick);
        match (i.checked_sub(1).map(|j| self.points[j]), self.points.get(i)) {
            (None, None) => None,
            (Some(a), None) => Some(a.value),
            (None, Some(b)) => Some(b.value),
            (Some(a), Some(b)) => {
                let span = (b.tick - a.tick) as f64;
                let f = if span > 0.0 {
                    (tick - a.tick) as f64 / span
                } else {
                    0.0
                };
                Some(a.value + (b.value - a.value) * f)
            }
        }
    }

    /// Wert in den Wertebereich des Parameters gezwungen; NaN wird zu `None`.
    fn clamp(&self, value: f64) -> Option<f64> {
        if !value.is_finite() {
            return None;
        }
        Some(match self.param.range() {
            Some((lo, hi)) => value.clamp(lo, hi),
            None => value,
        })
    }

    /// Setzt einen Punkt (ersetzt einen am selben Tick). Gibt seinen Index zurück.
    pub fn set_point(&mut self, tick: u64, value: f64) -> Option<usize> {
        let value = self.clamp(value)?;
        let i = self.points.partition_point(|p| p.tick < tick);
        match self.points.get_mut(i) {
            Some(p) if p.tick == tick => p.value = value,
            _ => self.points.insert(i, AutoPoint { tick, value }),
        }
        Some(i)
    }

    /// Verschiebt Punkt `i` nach (`tick`, `value`), höchstens bis an seine Nachbarn.
    /// Gibt den neuen Index zurück.
    pub fn move_point(&mut self, i: usize, tick: u64, value: f64) -> Option<usize> {
        let value = self.clamp(value)?;
        let lo = i.checked_sub(1).map_or(0, |j| self.points[j].tick + 1);
        let hi = self.points.get(i + 1).map_or(u64::MAX, |p| p.tick - 1);
        let p = self.points.get_mut(i)?;
        p.tick = tick.clamp(lo, hi.max(lo));
        p.value = value;
        Some(i)
    }

    pub fn remove_point(&mut self, i: usize) -> bool {
        if i < self.points.len() {
            self.points.remove(i);
            true
        } else {
            false
        }
    }

    /// Ob die Kurve beim Abspielen wirkt.
    pub fn active(&self) -> bool {
        !self.muted && !self.writing && !self.points.is_empty()
    }

    /// Ersetzt alle Punkte in `from..=to` durch einen Punkt bei `to` mit `value`
    /// (Schreiben: was der Cursor überstreicht, wird überschrieben).
    pub fn overwrite(&mut self, from: u64, to: u64, value: f64) {
        let (a, b) = (from.min(to), from.max(to));
        self.points.retain(|p| p.tick < a || p.tick > b);
        self.set_point(to, value);
    }

    /// Dünnt die Punkte in `from..=to` aus (Ramer-Douglas-Peucker): Punkte, die weniger als
    /// `tolerance` (in Einheiten des Parameters) von der Linie ihrer Nachbarn abweichen, fallen
    /// weg. Der erste und letzte Punkt des Bereichs bleiben.
    pub fn reduce(&mut self, from: u64, to: u64, tolerance: f64) {
        if tolerance.is_nan() || tolerance <= 0.0 {
            return;
        }
        let lo = self.points.partition_point(|p| p.tick < from);
        let hi = self.points.partition_point(|p| p.tick <= to);
        if hi.saturating_sub(lo) < 3 {
            return;
        }
        let seg = &self.points[lo..hi];
        let mut keep = vec![false; seg.len()];
        keep[0] = true;
        keep[seg.len() - 1] = true;
        let mut stack = vec![(0usize, seg.len() - 1)];
        while let Some((a, b)) = stack.pop() {
            if b <= a + 1 {
                continue;
            }
            let (pa, pb) = (seg[a], seg[b]);
            let span = (pb.tick - pa.tick) as f64;
            let mut worst = (0.0f64, a);
            for (i, p) in seg.iter().enumerate().take(b).skip(a + 1) {
                let f = if span > 0.0 {
                    (p.tick - pa.tick) as f64 / span
                } else {
                    0.0
                };
                let d = (p.value - (pa.value + (pb.value - pa.value) * f)).abs();
                if d > worst.0 {
                    worst = (d, i);
                }
            }
            if worst.0 > tolerance {
                keep[worst.1] = true;
                stack.push((a, worst.1));
                stack.push((worst.1, b));
            }
        }
        let mut i = 0;
        self.points.retain(|_| {
            let k = i < lo || i >= hi || keep[i - lo];
            i += 1;
            k
        });
    }
}

impl Track {
    /// Entfernt Send `k` samt seiner Automation; die Automation späterer Sends rückt nach.
    pub fn remove_send(&mut self, k: usize) {
        if k >= self.sends.len() {
            return;
        }
        self.sends.remove(k);
        self.automation
            .retain(|l| l.param != AutoParam::Send { index: k as u32 });
        for l in &mut self.automation {
            if let AutoParam::Send { index } = &mut l.param
                && *index as usize > k
            {
                *index -= 1;
            }
        }
    }

    /// Entfernt Insert `k` samt der Automation seiner Parameter; die Automation späterer
    /// Inserts rückt nach.
    pub fn remove_insert(&mut self, k: usize) {
        if k >= self.inserts.len() {
            return;
        }
        self.inserts.remove(k);
        let k = k as u32;
        self.automation
            .retain(|l| !matches!(l.param, AutoParam::Plugin { slot, .. } if slot == k));
        for l in &mut self.automation {
            if let AutoParam::Plugin { slot, .. } = &mut l.param
                && *slot != u32::MAX
                && *slot > k
            {
                *slot -= 1;
            }
        }
    }

    /// Setzt das Instrument. Ist es ein anderes Plugin (oder keins mehr), fällt die Automation
    /// der Parameter des alten weg.
    pub fn set_instrument(&mut self, slot: Option<InsertSlot>) {
        let old = self.instrument.as_ref().map(|i| i.plugin_id.as_str());
        let new = slot.as_ref().map(|i| i.plugin_id.as_str());
        if old != new {
            self.automation
                .retain(|l| !matches!(l.param, AutoParam::Plugin { slot: u32::MAX, .. }));
        }
        self.instrument = slot;
    }

    /// Die Automationsspur für `param`, falls vorhanden.
    pub fn automation_lane(&self, param: AutoParam) -> Option<&AutomationLane> {
        self.automation.iter().find(|l| l.param == param)
    }

    /// Die Automationsspur für `param`; wird angelegt, wenn es sie noch nicht gibt.
    pub fn automation_lane_mut(&mut self, param: AutoParam) -> &mut AutomationLane {
        let i = match self.automation.iter().position(|l| l.param == param) {
            Some(i) => i,
            None => {
                self.automation.push(AutomationLane::new(param));
                self.automation.len() - 1
            }
        };
        &mut self.automation[i]
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{Project, TrackKind};

    fn lane() -> AutomationLane {
        let mut l = AutomationLane::new(AutoParam::Volume);
        l.set_point(960, -12.0);
        l.set_point(0, 0.0);
        l
    }

    #[test]
    fn values_interpolate_and_hold_at_the_ends() {
        let l = lane();
        assert_eq!(l.value_at(0), Some(0.0));
        assert_eq!(l.value_at(480), Some(-6.0));
        assert_eq!(l.value_at(5000), Some(-12.0));
        assert_eq!(AutomationLane::new(AutoParam::Pan).value_at(0), None);
    }

    #[test]
    fn points_stay_sorted_unique_and_in_range() {
        let mut l = lane();
        assert_eq!(l.set_point(960, 99.0), Some(1));
        assert_eq!(l.points.len(), 2);
        assert_eq!(l.points[1].value, f64::from(MAX_DB));
        assert_eq!(l.set_point(480, f64::NAN), None);
        assert_eq!(l.set_point(480, -3.0), Some(1));
        let ticks: Vec<u64> = l.points.iter().map(|p| p.tick).collect();
        assert_eq!(ticks, [0, 480, 960]);
        // A point cannot pass its neighbours.
        l.move_point(1, 5000, -3.0);
        assert_eq!(l.points[1].tick, 959);
        assert!(l.remove_point(1));
        assert!(!l.remove_point(9));
    }

    #[test]
    fn overwrite_replaces_what_the_cursor_passed() {
        let mut l = lane();
        l.set_point(480, -3.0);
        l.overwrite(100, 600, -1.0);
        let ticks: Vec<u64> = l.points.iter().map(|p| p.tick).collect();
        assert_eq!(ticks, [0, 600, 960]);
        assert!(
            !AutomationLane {
                writing: true,
                ..lane()
            }
            .active()
        );
    }

    #[test]
    fn reduce_keeps_corners_and_drops_straight_runs() {
        let mut l = AutomationLane::new(AutoParam::Volume);
        // A ramp from 0 to -10 dB over ticks 0..100, then flat at -10 until 200.
        for k in 0..=20u64 {
            let t = k * 10;
            let v = if t <= 100 { -(t as f64) / 10.0 } else { -10.0 };
            l.set_point(t, v);
        }
        l.set_point(500, 3.0);
        l.reduce(0, 200, 0.05);
        let ticks: Vec<u64> = l.points.iter().map(|p| p.tick).collect();
        assert_eq!(ticks, [0, 100, 200, 500]);
        // Zero tolerance changes nothing.
        let before = l.points.clone();
        l.reduce(0, 500, 0.0);
        assert_eq!(l.points, before);
    }

    #[test]
    fn removing_a_send_takes_its_automation_along() {
        use crate::SendSlot;
        let mut p = Project::default();
        let t = p.add_track(TrackKind::Audio, "A");
        for level in [-6.0, -12.0, -18.0] {
            p.tracks[t].sends.push(SendSlot {
                target: 99,
                level_db: level,
                pre_fader: false,
                on: true,
            });
        }
        for index in 0..3 {
            p.tracks[t]
                .automation_lane_mut(AutoParam::Send { index })
                .set_point(0, -1.0);
        }
        assert_eq!(
            AutoParam::Send { index: 1 }.static_value(&p.tracks[t]),
            Some(-12.0)
        );
        p.tracks[t].remove_send(1);
        let params: Vec<AutoParam> = p.tracks[t].automation.iter().map(|l| l.param).collect();
        assert_eq!(
            params,
            [AutoParam::Send { index: 0 }, AutoParam::Send { index: 1 }]
        );
        assert_eq!(
            AutoParam::Send { index: 1 }.static_value(&p.tracks[t]),
            Some(-18.0)
        );
        assert_eq!(AutoParam::Send { index: 1 }.label(), "Send 2");
    }

    fn slot(id: &str) -> InsertSlot {
        InsertSlot {
            plugin_id: id.into(),
            name: id.into(),
            bypass: false,
            params: Vec::new(),
            state: None,
        }
    }

    #[test]
    fn removing_an_insert_takes_its_automation_along() {
        let mut p = Project::default();
        let t = p.add_track(TrackKind::Instrument, "Synth");
        let track = &mut p.tracks[t];
        track.inserts = vec![slot("eq"), slot("comp"), slot("hall")];
        track.instrument = Some(slot("synth"));
        for (s, id) in [(0, 1), (1, 2), (2, 3), (u32::MAX, 4)] {
            track
                .automation_lane_mut(AutoParam::Plugin { slot: s, id })
                .set_point(0, 0.5);
        }
        track
            .automation_lane_mut(AutoParam::Volume)
            .set_point(0, -3.0);
        track.remove_insert(1);
        let names: Vec<&str> = track.inserts.iter().map(|i| i.plugin_id.as_str()).collect();
        assert_eq!(names, ["eq", "hall"]);
        let params: Vec<AutoParam> = track.automation.iter().map(|l| l.param).collect();
        assert_eq!(
            params,
            [
                AutoParam::Plugin { slot: 0, id: 1 },
                AutoParam::Plugin { slot: 1, id: 3 },
                AutoParam::Plugin {
                    slot: u32::MAX,
                    id: 4
                },
                AutoParam::Volume,
            ],
            "the hall's curve moves to slot 1, the instrument's stays"
        );
        track.remove_insert(7);
        assert_eq!(track.inserts.len(), 2);
    }

    #[test]
    fn a_new_instrument_drops_the_old_ones_automation() {
        let mut p = Project::default();
        let t = p.add_track(TrackKind::Instrument, "Synth");
        let track = &mut p.tracks[t];
        track.instrument = Some(slot("synth"));
        track
            .automation_lane_mut(AutoParam::Plugin {
                slot: u32::MAX,
                id: 4,
            })
            .set_point(0, 0.5);
        track.automation_lane_mut(AutoParam::Plugin { slot: 0, id: 1 });
        // Same plugin again (e.g. state reload): automation stays.
        track.set_instrument(Some(slot("synth")));
        assert_eq!(track.automation.len(), 2);
        track.set_instrument(Some(slot("piano")));
        let params: Vec<AutoParam> = track.automation.iter().map(|l| l.param).collect();
        assert_eq!(params, [AutoParam::Plugin { slot: 0, id: 1 }]);
        track.set_instrument(None);
        assert!(track.instrument.is_none());
    }

    #[test]
    fn lanes_are_created_once_per_parameter_and_saved() {
        let mut p = Project::default();
        let t = p.add_track(TrackKind::Audio, "A");
        p.tracks[t]
            .automation_lane_mut(AutoParam::Pan)
            .set_point(0, 0.5);
        p.tracks[t]
            .automation_lane_mut(AutoParam::Pan)
            .set_point(10, -0.5);
        assert_eq!(p.tracks[t].automation.len(), 1);
        assert_eq!(AutoParam::Volume.static_value(&p.tracks[t]), Some(0.0));
        let json = serde_json::to_string(&p).expect("save");
        let back: Project = serde_json::from_str(&json).expect("load");
        assert_eq!(back.tracks[t].automation, p.tracks[t].automation);
    }
}
