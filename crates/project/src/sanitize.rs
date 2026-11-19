//! Werte aus Projektdateien in gültige Bereiche bringen (AGENTS.md „Robustness“, Issue #49).
//!
//! Eine Projektdatei ist fremde Eingabe: Positionen nahe `u64::MAX` lassen Additionen
//! überlaufen, eine Lane-Nummer von vier Milliarden ließe [`crate::Track::lanes`] Speicher für
//! vier Milliarden Lanes anlegen, ein Taktnenner 0 teilt durch null. [`Project::sanitize`] zieht
//! solche Werte nach dem Laden an die Grenzen, die die App selbst nie überschreitet.

use crate::{MAX_DB, MIN_DB, Project, TICKS_PER_QUARTER};

/// Längste Position bzw. Länge in Ticks: 100 000 Takte zu 4/4.
pub const MAX_TICKS: u64 = TICKS_PER_QUARTER * 4 * 100_000;
/// Längster Clip in Frames: ein Tag bei 384 kHz.
pub const MAX_FRAMES: u64 = 384_000 * 60 * 60 * 24;
/// Höchste Lane- bzw. Take-Nummer.
pub const MAX_LANE: u32 = 255;
/// Tempo-Grenzen in BPM.
pub const BPM: (f64, f64) = (10.0, 999.0);
/// Höchstens so viele Tempowechsel.
pub const MAX_TEMPO_EVENTS: usize = 10_000;
/// Abtastraten-Grenzen eines Pool-Clips.
pub const SAMPLE_RATE: (u32, u32) = (1_000, 768_000);

fn finite_f32(v: f32, (lo, hi): (f32, f32), default: f32) -> f32 {
    if v.is_finite() {
        v.clamp(lo, hi)
    } else {
        default
    }
}

impl Project {
    /// Zieht alle Werte in gültige Bereiche und gibt zurück, wie viele geändert wurden.
    pub fn sanitize(&mut self) -> usize {
        let mut fixed = 0usize;
        let mut fix = |changed: bool| fixed += usize::from(changed);
        macro_rules! clamp {
            ($v:expr, $new:expr) => {{
                let new = $new;
                fix($v != new);
                $v = new;
            }};
        }

        fix(self.layout.sanitize());
        if self.mixer_views.len() > crate::MAX_MIXER_VIEWS {
            self.mixer_views.truncate(crate::MAX_MIXER_VIEWS);
            fix(true);
        }
        for v in &mut self.mixer_views {
            if v.name.chars().count() > 64 {
                v.name = v.name.chars().take(64).collect();
                fix(true);
            }
            let cap = 65_536;
            if v.hidden.len() > cap || v.narrow.len() > cap {
                v.hidden.truncate(cap);
                v.narrow.truncate(cap);
                fix(true);
            }
        }
        for t in &mut self.tracks {
            clamp!(t.midi_channel, t.midi_channel.clamp(1, 16));
            clamp!(t.program, t.program.map(|p| p.min(127)));
            if let Some(port) = &t.midi_port
                && port.chars().count() > 256
            {
                t.midi_port = Some(port.chars().take(256).collect());
                fix(true);
            }
        }
        if self.layouts.len() > crate::layout::MAX_LAYOUTS {
            self.layouts.truncate(crate::layout::MAX_LAYOUTS);
            fix(true);
        }
        for l in &mut self.layouts {
            fix(l.layout.sanitize());
            let name = crate::layout::clip_name(&l.name);
            fix(name != l.name);
            l.name = name;
        }

        let bpm = self.tempo.bpm;
        clamp!(
            self.tempo.bpm,
            if bpm.is_finite() {
                bpm.clamp(BPM.0, BPM.1)
            } else {
                120.0
            }
        );
        let sig = &mut self.tempo.signature;
        clamp!(sig.numerator, sig.numerator.clamp(1, 64));
        let d = sig.denominator;
        clamp!(
            sig.denominator,
            if d.is_power_of_two() && d <= 64 { d } else { 4 }
        );
        clamp!(
            self.master_db,
            finite_f32(self.master_db, (MIN_DB, MAX_DB), 0.0)
        );

        // Tempospur: gültige Werte, sortiert, jeder Tick einmal, begrenzte Anzahl.
        let before = self.tempo.events.clone();
        let ev = &mut self.tempo.events;
        ev.truncate(MAX_TEMPO_EVENTS);
        ev.retain(|e| e.tick > 0 && e.bpm.is_finite());
        for e in ev.iter_mut() {
            e.tick = e.tick.min(MAX_TICKS);
            e.bpm = e.bpm.clamp(BPM.0, BPM.1);
        }
        ev.sort_by_key(|e| e.tick);
        ev.dedup_by_key(|e| e.tick);
        fix(*ev != before);
        let before = self.tempo.signatures.clone();
        let sigs = &mut self.tempo.signatures;
        sigs.truncate(MAX_TEMPO_EVENTS);
        sigs.retain(|e| e.bar >= 2 && e.bar <= MAX_TICKS / crate::TICKS_PER_QUARTER);
        for e in sigs.iter_mut() {
            let sane = e.signature().sane();
            e.numerator = sane.numerator;
            e.denominator = sane.denominator;
        }
        sigs.sort_by_key(|e| e.bar);
        sigs.dedup_by_key(|e| e.bar);
        fix(*sigs != before);
        let io = std::mem::take(&mut self.io).sanitized();
        self.io = io;
        // Verweise auf Busse, die es nicht (mehr) gibt.
        for t in &mut self.tracks {
            if t.input_bus.is_some_and(|id| self.io.input(id).is_none()) {
                t.input_bus = None;
            }
            if t.out_bus
                .is_some_and(|id| id == crate::MAIN_OUT || self.io.output(id).is_none())
            {
                t.out_bus = None;
            }
        }
        let click = self.click.sanitized();

        fix(click != self.click);

        self.click = click;

        for c in &mut self.pool {
            clamp!(
                c.sample_rate,
                c.sample_rate.clamp(SAMPLE_RATE.0, SAMPLE_RATE.1)
            );
            clamp!(c.frames, c.frames.min(MAX_FRAMES));
        }

        for m in &mut self.markers {
            clamp!(m.start, m.start.min(MAX_TICKS));
            let start = m.start;
            clamp!(
                m.end,
                m.end.map(|e| e.min(MAX_TICKS)).filter(|&e| e > start)
            );
        }

        for t in &mut self.tracks {
            clamp!(t.volume_db, finite_f32(t.volume_db, (MIN_DB, MAX_DB), 0.0));
            clamp!(t.pan, finite_f32(t.pan, (-1.0, 1.0), 0.0));
            for band in &mut t.eq.bands {
                let clean = band.sanitized();
                fix(*band != clean);
                *band = clean;
            }
            for s in &mut t.sends {
                clamp!(s.level_db, finite_f32(s.level_db, (MIN_DB, MAX_DB), 0.0));
            }
            for slot in t.inserts.iter_mut().chain(t.instrument.as_mut()) {
                for (_, v) in &mut slot.params {
                    clamp!(*v, if v.is_finite() { *v } else { 0.0 });
                }
            }
            for lane in &mut t.automation {
                let range = lane.param.range();
                let before = lane.points.len();
                lane.points.retain(|p| p.value.is_finite());
                fix(lane.points.len() != before);
                for p in &mut lane.points {
                    clamp!(p.tick, p.tick.min(MAX_TICKS));
                    if let Some((lo, hi)) = range {
                        clamp!(p.value, p.value.clamp(lo, hi));
                    }
                }
            }
            for e in &mut t.events {
                clamp!(e.start.0, e.start.0.min(MAX_TICKS));
                clamp!(e.length, e.length.min(MAX_TICKS));
                clamp!(e.offset, e.offset.min(MAX_FRAMES));
                clamp!(e.fade_in, e.fade_in.min(e.length));
                let rest = e.length - e.fade_in;
                clamp!(e.fade_out, e.fade_out.min(rest));
                clamp!(
                    e.gain_db,
                    finite_f32(e.gain_db, (MIN_DB, crate::EVENT_MAX_DB), 0.0)
                );
                clamp!(e.lane, e.lane.map(|l| l.min(MAX_LANE)));
                clamp!(e.comp, e.comp.min(MAX_LANE));
                for n in &mut e.notes {
                    let max = MAX_TICKS as i64;
                    clamp!(n.start, n.start.clamp(-max, max));
                    clamp!(n.length, n.length.min(MAX_TICKS));
                    clamp!(n.pitch, n.pitch.min(127));
                    clamp!(n.velocity, n.velocity.clamp(1, 127));
                }
                if e.controllers.len() > crate::controllers::MAX_CTRL_POINTS {
                    e.controllers.truncate(crate::controllers::MAX_CTRL_POINTS);
                    fix(true);
                }
                for c in &mut e.controllers {
                    let max = MAX_TICKS as i64;
                    clamp!(c.start, c.start.clamp(-max, max));
                    clamp!(c.value, c.value.min(c.kind.max()));
                    if let crate::CtrlKind::Cc(n) = c.kind {
                        clamp!(c.kind, crate::CtrlKind::Cc(n.min(127)));
                    }
                }
            }
        }
        fixed += self.fix_ids();
        fixed
    }
}
