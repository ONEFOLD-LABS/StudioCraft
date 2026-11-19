//! Automation schreiben (Spec „Automation“ → Schreibmodi, Issue #34).
//!
//! Bewegt man bei laufender Wiedergabe ein Bedienelement einer Spur mit W (Fader, Pan, Mute,
//! Plugin-Parameter, im Mixer wie im Inspector), entsteht ein Schreibdurchgang für diesen
//! Parameter. Das Bedienelement gilt als berührt, solange die Maustaste gedrückt bleibt.
//!
//! - Touch: schreibt, solange berührt; beim Loslassen kehrt die Kurve kurz danach zur
//!   bisherigen zurück.
//! - Auto-Latch: schreibt ab der Berührung bis zum Stopp mit dem letzten Wert.
//!
//! Fehlt die Automationsspur, wird sie angelegt. Während des Schreibens klingt der Wert des
//! Bedienelements, nicht die Kurve. Beim Stopp werden die geschriebenen Punkte ausgedünnt; der
//! ganze Durchgang ist ein Rückgängig-Schritt.

use std::collections::HashMap;

use egui::Context;
use studiocraft_project::{AutoParam, AutomationLane, Project};

use crate::ProjectWindow;

/// Schreibmodus bei aktivem W.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum WriteMode {
    #[default]
    Touch,
    Latch,
}

impl WriteMode {
    pub const ALL: [WriteMode; 2] = [WriteMode::Touch, WriteMode::Latch];

    pub fn label(self) -> &'static str {
        match self {
            WriteMode::Touch => "Touch",
            WriteMode::Latch => "Auto-Latch",
        }
    }
}

/// Zeit, in der die Kurve nach dem Loslassen (Touch) zur bisherigen zurückkehrt: eine
/// Sechzehntel.
const RETURN_TICKS: u64 = studiocraft_project::TICKS_PER_QUARTER / 4;

/// Ein laufender Schreibdurchgang für einen Parameter.
#[derive(Debug, Clone)]
struct Pass {
    /// Die Kurve vor dem Durchgang (für die Rückkehr bei Touch).
    original: AutomationLane,
    start: u64,
    last: u64,
    value: f64,
    /// Bedienelement gehalten.
    touching: bool,
    /// Touch losgelassen: der Durchgang ist zu Ende.
    done: bool,
}

/// Zustand des Schreibens im Projektfenster.
#[derive(Debug, Clone)]
pub(crate) struct AutomationWriter {
    pub mode: WriteMode,
    /// Ausdünnen der geschriebenen Punkte, 0–100 %.
    pub reduction: u8,
    /// Werte im letzten Bild, um Bewegungen zu erkennen.
    last: HashMap<(usize, AutoParam), f64>,
    passes: HashMap<(usize, AutoParam), Pass>,
    /// Projekt vor dem ersten Durchgang (ein Rückgängig-Schritt für alles bis zum Stopp).
    before: Option<Project>,
}

impl Default for AutomationWriter {
    fn default() -> Self {
        Self {
            mode: WriteMode::Touch,
            reduction: 50,
            last: HashMap::new(),
            passes: HashMap::new(),
            before: None,
        }
    }
}

/// Werte aller beschreibbaren Parameter der Spuren mit W.
fn current_values(project: &Project) -> HashMap<(usize, AutoParam), f64> {
    let mut out = HashMap::new();
    for (i, t) in project.tracks.iter().enumerate() {
        if !t.write || !crate::mixer::has_channel(t.kind) {
            continue;
        }
        for p in [AutoParam::Volume, AutoParam::Pan, AutoParam::Mute] {
            if let Some(v) = p.static_value(t) {
                out.insert((i, p), v);
            }
        }
        let plugins = t.instrument.iter().map(|s| (u32::MAX, s)).chain(
            t.inserts
                .iter()
                .enumerate()
                .filter_map(|(k, s)| Some((u32::try_from(k).ok()?, s))),
        );
        for (slot, s) in plugins {
            for &(id, v) in &s.params {
                out.insert((i, AutoParam::Plugin { slot, id }), v);
            }
        }
    }
    out
}

impl ProjectWindow {
    /// Einmal pro Bild nach der Oberfläche: Bewegungen erkennen und in die Kurven schreiben.
    pub(crate) fn automation_write(&mut self, ctx: &Context, playing: bool) {
        let now = current_values(&self.project);
        if !playing {
            self.finish_passes(true);
            self.auto_writer.last = now;
            return;
        }
        let cursor = self.cursor;
        let held = ctx.input(|i| i.pointer.primary_down());
        let mut moved = Vec::new();
        for (key, &v) in &now {
            let changed = self
                .auto_writer
                .last
                .get(key)
                .is_none_or(|&old| (old - v).abs() > 1e-9);
            // Nur Spuren, die schon im letzten Bild W hatten (Einschalten von W ist keine Bewegung).
            if changed
                && self
                    .auto_writer
                    .last
                    .contains_key(&(key.0, AutoParam::Volume))
            {
                moved.push((*key, v, self.auto_writer.last.get(key).copied()));
            }
        }
        for (key, v, old) in moved {
            self.touch(key, v, old, cursor);
        }
        let mode = self.auto_writer.mode;
        let keys: Vec<_> = self.auto_writer.passes.keys().copied().collect();
        for key in keys {
            let Some(pass) = self.auto_writer.passes.get_mut(&key) else {
                continue;
            };
            if pass.done {
                continue;
            }
            if let Some(&v) = now.get(&key) {
                pass.value = v;
            }
            // Cycle-Sprung zurück: neu ansetzen.
            if cursor < pass.last {
                pass.last = cursor;
            }
            let released = pass.touching && !held;
            let (from, value) = (pass.last, pass.value);
            if pass.touching || mode == WriteMode::Latch {
                if let Some(lane) = self.lane_mut(key) {
                    lane.overwrite(from.saturating_add(1).min(cursor), cursor, value);
                }
                if let Some(p) = self.auto_writer.passes.get_mut(&key) {
                    p.last = cursor;
                }
            }
            if released && let Some(p) = self.auto_writer.passes.get_mut(&key) {
                p.touching = false;
                if mode == WriteMode::Touch {
                    // Zurück zur bisherigen Kurve (oder zum Wert beim Loslassen, wenn es keine gab).
                    let back = cursor.saturating_add(RETURN_TICKS);
                    let target = p.original.value_at(back).unwrap_or(value);
                    let original = p.original.clone();
                    p.done = true;
                    if let Some(lane) = self.lane_mut(key) {
                        lane.points.retain(|q| q.tick <= cursor || q.tick > back);
                        lane.set_point(back, target);
                        // Hinter der Rückkehr gilt wieder die alte Kurve.
                        for q in original.points.iter().filter(|q| q.tick > back) {
                            lane.set_point(q.tick, q.value);
                        }
                    }
                }
            }
        }
        self.auto_writer.last = now;
    }

    fn lane_mut(&mut self, (track, param): (usize, AutoParam)) -> Option<&mut AutomationLane> {
        let t = self.project.tracks.get_mut(track)?;
        Some(t.automation_lane_mut(param))
    }

    /// Ein Bedienelement wurde bewegt: Durchgang anlegen bzw. als berührt markieren.
    fn touch(&mut self, key: (usize, AutoParam), value: f64, old: Option<f64>, cursor: u64) {
        if let Some(p) = self.auto_writer.passes.get_mut(&key)
            && !p.done
        {
            p.touching = true;
            p.value = value;
            return;
        }
        if self.auto_writer.before.is_none() {
            self.auto_writer.before = Some(self.project.clone());
        }
        let Some(lane) = self.lane_mut(key) else {
            return;
        };
        let original = lane.clone();
        lane.writing = true;
        lane.visible = true;
        // Mute springt: der alte Wert gilt bis direkt vor dem Cursor.
        if key.1 == AutoParam::Mute
            && let Some(old) = original.value_at(cursor).or(old)
        {
            lane.set_point(cursor.saturating_sub(1), old);
        }
        lane.set_point(cursor, value);
        self.auto_writer.passes.insert(
            key,
            Pass {
                original,
                start: cursor,
                last: cursor,
                value,
                touching: true,
                done: false,
            },
        );
        // Engine neu versorgen: die Spur folgt jetzt dem Bedienelement.
        self.dirty = true;
    }

    /// Stopp: Latch schreibt bis zur Stoppposition, dann ausdünnen und einen Rückgängig-Schritt
    /// anlegen.
    fn finish_passes(&mut self, stopped: bool) {
        if self.auto_writer.passes.is_empty() {
            return;
        }
        let passes = std::mem::take(&mut self.auto_writer.passes);
        let reduction = f64::from(self.auto_writer.reduction.min(100)) / 100.0;
        for (key, pass) in passes {
            let range = {
                let Some(t) = self.project.tracks.get(key.0) else {
                    continue;
                };
                crate::timeline::param_range(self, t, key.1)
            };
            let end = pass.last;
            if let Some(lane) = self.lane_mut(key) {
                if stopped && !pass.done && pass.last > pass.start {
                    lane.set_point(end, pass.value);
                }
                lane.writing = false;
                if key.1 != AutoParam::Mute {
                    lane.reduce(pass.start, end, (range.1 - range.0) * reduction * 0.02);
                }
            }
        }
        if let Some(before) = self.auto_writer.before.take() {
            self.checkpoint(before);
        }
        self.dirty = true;
    }

    /// Ob gerade Automation geschrieben wird (für die Anzeige).
    pub(crate) fn writing_automation(&self) -> bool {
        !self.auto_writer.passes.is_empty()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use studiocraft_project::TrackKind;

    fn window() -> ProjectWindow {
        let mut p = Project::default();
        let t = p.add_track(TrackKind::Audio, "Gesang");
        p.tracks[t].write = true;
        ProjectWindow::new(p, None, String::new())
    }

    /// Ein Bild mit laufender Wiedergabe an `cursor`, Maustaste gedrückt oder nicht.
    fn frame(w: &mut ProjectWindow, cursor: u64, held: bool) {
        w.cursor = cursor;
        let ctx = Context::default();
        let mut input = egui::RawInput::default();
        if held {
            let pos = egui::pos2(1.0, 1.0);
            input.events.push(egui::Event::PointerMoved(pos));
            input.events.push(egui::Event::PointerButton {
                pos,
                button: egui::PointerButton::Primary,
                pressed: true,
                modifiers: egui::Modifiers::NONE,
            });
        }
        let mut out = ctx.run_ui(input, |ui| w.automation_write(ui.ctx(), true));
        out.textures_delta.clear();
    }

    fn points(w: &ProjectWindow, p: AutoParam) -> Vec<(u64, f64)> {
        w.project.tracks[0]
            .automation_lane(p)
            .map(|l| l.points.iter().map(|q| (q.tick, q.value)).collect())
            .unwrap_or_default()
    }

    #[test]
    fn touch_writes_while_held_and_returns_after_release() {
        let mut w = window();
        frame(&mut w, 0, false);
        w.project.tracks[0].volume_db = -6.0;
        frame(&mut w, 960, true);
        assert!(w.writing_automation());
        assert!(
            w.project.tracks[0]
                .automation_lane(AutoParam::Volume)
                .is_some_and(|l| l.writing)
        );
        w.project.tracks[0].volume_db = -12.0;
        frame(&mut w, 1920, true);
        frame(&mut w, 2880, false);
        // Released at 2880: the curve returns to the original (none: the last value) a 1/16 later.
        let pts = points(&w, AutoParam::Volume);
        assert_eq!(pts.first(), Some(&(960, -6.0)));
        assert!(pts.contains(&(2880, -12.0)), "{pts:?}");
        assert_eq!(pts.last().map(|p| p.0), Some(2880 + 240));
        // Stop: one undo step, lane plays again.
        frame_stopped(&mut w);
        assert!(!w.writing_automation());
        assert!(
            w.project.tracks[0]
                .automation_lane(AutoParam::Volume)
                .is_some_and(|l| !l.writing && l.visible)
        );
        w.undo();
        assert!(w.project.tracks[0].automation.is_empty());
    }

    fn frame_stopped(w: &mut ProjectWindow) {
        let ctx = Context::default();
        let mut out = ctx.run_ui(egui::RawInput::default(), |ui| {
            w.automation_write(ui.ctx(), false)
        });
        out.textures_delta.clear();
    }

    #[test]
    fn latch_keeps_writing_the_last_value_until_stop() {
        let mut w = window();
        w.auto_writer.mode = WriteMode::Latch;
        frame(&mut w, 0, false);
        w.project.tracks[0].pan = 0.5;
        frame(&mut w, 960, true);
        frame(&mut w, 1920, false);
        frame(&mut w, 3840, false);
        frame_stopped(&mut w);
        let pts = points(&w, AutoParam::Pan);
        assert_eq!(pts.first(), Some(&(960, 0.5)));
        assert_eq!(pts.last(), Some(&(3840, 0.5)));
        // Straight run thinned out to its ends.
        assert_eq!(pts.len(), 2, "{pts:?}");
    }

    #[test]
    fn nothing_is_written_without_w_or_while_stopped() {
        let mut w = window();
        frame_stopped(&mut w);
        w.project.tracks[0].volume_db = -3.0;
        frame_stopped(&mut w);
        assert!(w.project.tracks[0].automation.is_empty());
        w.project.tracks[0].write = false;
        frame(&mut w, 0, false);
        w.project.tracks[0].volume_db = -9.0;
        frame(&mut w, 960, true);
        assert!(w.project.tracks[0].automation.is_empty());
    }

    #[test]
    fn mute_writes_a_step() {
        let mut w = window();
        frame(&mut w, 0, false);
        w.project.tracks[0].mute = true;
        frame(&mut w, 960, true);
        let pts = points(&w, AutoParam::Mute);
        assert!(
            pts.contains(&(959, 0.0)) && pts.contains(&(960, 1.0)),
            "{pts:?}"
        );
    }
}
