//! Tempospur in der Event-Anzeige (Issue #50): die Tempokurve mit ihren Punkten.
//!
//! Objektauswahl zieht einen Punkt (Takt nach Snap, Tempo nach Höhe; Alt: ohne Snap), Stift
//! setzt einen Punkt, Radiergummi löscht ihn. Rampen und genaue Werte im Inspector.

use egui::{Align2, FontId, Painter, Pos2, Stroke, pos2};
use studiocraft_project::{BPM_RANGE, TempoMap};

use super::{Drag, Geo};
use crate::{ProjectWindow, Tool};

/// Fangradius für Punkte in Pixeln.
const POINT_PX: f32 = 6.0;
/// Innenabstand oben und unten.
const PAD: f32 = 6.0;

/// Angezeigter Tempobereich: alle Punkte mit etwas Luft, mindestens 40 BPM hoch.
fn range(map: &TempoMap) -> (f64, f64) {
    let all = std::iter::once(map.bpm).chain(map.events.iter().map(|e| e.bpm));
    let (lo, hi) = all.fold((f64::MAX, f64::MIN), |(lo, hi), b| (lo.min(b), hi.max(b)));
    let mid = (lo + hi) / 2.0;
    let half = ((hi - lo) / 2.0 + 10.0).max(20.0);
    ((mid - half).max(BPM_RANGE.0), (mid + half).min(BPM_RANGE.1))
}

fn y_of(g: &Geo, row: usize, (lo, hi): (f64, f64), bpm: f64) -> f32 {
    let top = g.row_top(row) + PAD;
    let h = (g.row_height(row) - 2.0 * PAD).max(1.0);
    top + h * (1.0 - ((bpm - lo) / (hi - lo).max(1e-6)) as f32).clamp(0.0, 1.0)
}

fn bpm_of(g: &Geo, row: usize, (lo, hi): (f64, f64), y: f32) -> f64 {
    let top = g.row_top(row) + PAD;
    let h = (g.row_height(row) - 2.0 * PAD).max(1.0);
    let f = (1.0 - (y - top) / h).clamp(0.0, 1.0);
    (lo + f64::from(f) * (hi - lo)).clamp(BPM_RANGE.0, BPM_RANGE.1)
}

/// Punkt unter `p`: `Some(None)` = Anfangstempo (Tick 0), `Some(Some(i))` = Wechsel `i`.
pub(super) fn point_at(w: &ProjectWindow, g: &Geo, row: usize, p: Pos2) -> Option<Option<usize>> {
    let map = &w.project.tempo;
    let r = range(map);
    let near = |tick: u64, bpm: f64| {
        let q = pos2(g.x(tick as f64), y_of(g, row, r, bpm));
        q.distance(p) <= POINT_PX
    };
    if let Some(i) = map.events.iter().rposition(|e| near(e.tick, e.bpm)) {
        return Some(Some(i));
    }
    near(0, map.bpm).then_some(None)
}

/// Klick in der Tempospur (ohne Ziehen): Stift setzt, Radiergummi löscht.
pub(super) fn click(w: &mut ProjectWindow, g: &Geo, row: usize, p: Pos2) {
    w.selected_track = Some(row);
    let mut map = w.project.tempo.clone();
    match (w.tool, point_at(w, g, row, p)) {
        (Tool::Erase, Some(Some(i))) => {
            map.events.remove(i);
        }
        (Tool::Draw, None) => {
            let t = w.snap_ticks(g.ticks(p.x));
            let bpm = (bpm_of(g, row, range(&map), p.y) * 10.0).round() / 10.0;
            map.set_event(t, bpm, false);
            map.use_track = true;
        }
        _ => return,
    }
    w.project.set_tempo_map(map);
}

/// Ziehen eines Punkts beginnt (Objektauswahl).
pub(super) fn start_drag(w: &ProjectWindow, g: &Geo, row: usize, p: Pos2) -> Option<Drag> {
    if w.tool != Tool::Select {
        return None;
    }
    let point = point_at(w, g, row, p)?;
    Some(Drag::TempoPoint {
        row,
        index: point,
        range: range(&w.project.tempo),
    })
}

/// Während des Ziehens: neue Map aus dem Stand beim Greifen, damit sich beim Umrechnen der
/// Audio-Events keine Rundungsfehler aufsummieren.
pub(super) fn live_drag(
    w: &mut ProjectWindow,
    g: &Geo,
    (row, index, range): (usize, Option<usize>, (f64, f64)),
    p: Pos2,
    snapped: u64,
) {
    let Some(before) = w.drag_before.clone() else {
        return;
    };
    let mut map = before.tempo.clone();
    let bpm = (bpm_of(g, row, range, p.y) * 10.0).round() / 10.0;
    match index {
        None => map.bpm = bpm,
        Some(i) => {
            let Some(old) = map.events.get(i).copied() else {
                return;
            };
            map.events.remove(i);
            // Nicht auf den Anfang: dort gilt das Anfangstempo.
            map.set_event(snapped.max(1), bpm, old.ramp);
        }
    }
    let mut project = before;
    project.set_tempo_map(map);
    w.project = project;
}

/// Tempokurve mit Punkten und Werten.
pub(super) fn paint(w: &ProjectWindow, painter: &Painter, g: &Geo, row: usize) {
    let map = &w.project.tempo;
    let t = crate::theme::tokens(painter.ctx());
    let r = range(map);
    let color = if map.use_track { t.accent } else { t.text_dim };
    let stroke = Stroke::new(1.5, color);
    let right = g.lanes.right();
    let mut pts = vec![pos2(g.x(0.0), y_of(g, row, r, map.bpm))];
    let mut bpm = map.bpm;
    for e in &map.events {
        let x = g.x(e.tick as f64);
        if !e.ramp {
            pts.push(pos2(x, y_of(g, row, r, bpm)));
        }
        pts.push(pos2(x, y_of(g, row, r, e.bpm)));
        bpm = e.bpm;
    }
    pts.push(pos2(right.max(g.x(0.0)), y_of(g, row, r, bpm)));
    painter.add(egui::Shape::line(pts, stroke));
    let font = FontId::proportional(10.0);
    let points = std::iter::once((0u64, map.bpm)).chain(map.events.iter().map(|e| (e.tick, e.bpm)));
    for (tick, bpm) in points {
        let c = pos2(g.x(tick as f64), y_of(g, row, r, bpm));
        if c.x < g.lanes.left() - POINT_PX || c.x > right + POINT_PX {
            continue;
        }
        painter.circle_filled(c, 3.5, t.handle);
        painter.circle_stroke(c, 3.5, Stroke::new(1.0, color));
        painter.text(
            c + egui::vec2(5.0, -2.0),
            Align2::LEFT_BOTTOM,
            format!("{bpm:.1}"),
            font.clone(),
            t.text,
        );
    }
    if !map.use_track {
        painter.text(
            pos2(
                g.lanes.left() + 6.0,
                g.row_top(row) + g.row_height(row) - 3.0,
            ),
            Align2::LEFT_BOTTOM,
            "Festes Tempo – Tempospur im Inspector einschalten",
            font,
            t.text_dim,
        );
    }
}
