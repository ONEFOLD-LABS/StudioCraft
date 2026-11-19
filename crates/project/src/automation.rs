//! Automation (Spec „Automation“, Issue #34): Kurven für Parameter eines Kanals oder Plugins.
//!
//! Jede Kurve liegt auf einer Automationsspur unter ihrer Spur. Punkte stehen in Ticks und in
//! der natürlichen Einheit des Parameters: Lautstärke in dB, Pan −1…+1, Mute 0/1, Plugin-
//! Parameter im Wertebereich des Plugins. Zwischen zwei Punkten wird linear interpoliert; vor
//! dem ersten und nach dem letzten Punkt gilt dessen Wert.

use crate::{MAX_DB, MIN_DB, Track};

/// Welcher Parameter automatisiert wird.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, serde::Serialize, serde::Deserialize)]
pub enum AutoParam {
    Volume,
    Pan,
    Mute,
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
            AutoParam::Plugin { .. } => None,
        }
    }

    /// Kurzer Name für die Automationsspur.
    pub fn label(self) -> String {
        match self {
            AutoParam::Volume => "Lautstärke".into(),
            AutoParam::Pan => "Pan".into(),
            AutoParam::Mute => "Mute".into(),
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
}

impl AutomationLane {
    pub fn new(param: AutoParam) -> Self {
        Self {
            param,
            points: Vec::new(),
            muted: false,
            visible: true,
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
        !self.muted && !self.points.is_empty()
    }
}

impl Track {
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
