//! Automationsspuren in der Event-Anzeige (Spec „Automation“ → Automationsspuren und
//! Kurvenbearbeitung, Issue #34).
//!
//! Sichtbare Automationsspuren liegen unter ihrer Spur (unter den Lanes), je eine Zeile hoch.
//! Der Pfeil im Spurkopf klappt sie auf und zu. Objektauswahl: Klick setzt einen Punkt, Ziehen
//! bewegt ihn (Alt: ohne Snap). Radiergummi löscht einen Punkt, Zeichnen zeichnet frei.

use egui::{Align2, Color32, FontId, Painter, Pos2, Rect, Stroke, StrokeKind, pos2, vec2};
use studiocraft_project::{AutoParam, Track};

use super::{Drag, Geo, track_color};
use crate::{ProjectWindow, Tool};

/// Fangradius für Punkte in Pixeln.
const POINT_PX: f32 = 6.0;
/// Innenabstand oben und unten in einer Automationsspur.
const PAD: f32 = 5.0;

/// Indizes der sichtbaren Automationsspuren einer Spur, in Anzeigereihenfolge.
pub(super) fn visible(track: &Track) -> impl Iterator<Item = usize> + '_ {
    track
        .automation
        .iter()
        .enumerate()
        .filter(|(_, l)| l.visible)
        .map(|(i, _)| i)
}

/// Höhe der Lanes einer Spur (ohne Automation) in Zeilen.
pub(super) fn take_lanes(track: &Track) -> usize {
    if track.show_lanes { track.lanes().1 } else { 1 }
}

/// Zeilen, die eine Spur insgesamt belegt.
pub(super) fn rows_of(track: &Track) -> usize {
    take_lanes(track) + visible(track).count()
}

/// Rechteck der Automationsspur `lane` (Index in `track.automation`) von Spur `row`, über die
/// Breite von `x`.
fn lane_rect(w: &ProjectWindow, g: &Geo, row: usize, lane: usize, x: egui::Rangef) -> Option<Rect> {
    let track = w.project.tracks.get(row)?;
    let k = visible(track).position(|i| i == lane)?;
    let top = g.row_top(row) + (take_lanes(track) + k) as f32 * g.view.row_h;
    Some(Rect::from_x_y_ranges(x, top..=top + g.view.row_h))
}

/// Die Automationsspur unter `y` in Spur `row`.
pub(super) fn lane_at(w: &ProjectWindow, g: &Geo, row: usize, y: f32) -> Option<usize> {
    let track = w.project.tracks.get(row)?;
    let k = ((y - g.row_top(row)) / g.view.row_h).floor() as isize - take_lanes(track) as isize;
    usize::try_from(k).ok().and_then(|k| visible(track).nth(k))
}

/// Wertebereich eines Parameters; Plugins aus ihrer Parameterbeschreibung, sonst 0…1.
pub(crate) fn range(w: &ProjectWindow, track: &Track, param: AutoParam) -> (f64, f64) {
    if let Some(r) = param.range() {
        return r;
    }
    let AutoParam::Plugin { slot, id } = param else {
        return (0.0, 1.0);
    };
    let ins = if slot == u32::MAX {
        track.instrument.as_ref()
    } else {
        track.inserts.get(slot as usize)
    };
    ins.and_then(|i| w.plugins.params.get(&i.plugin_id))
        .and_then(|ps| ps.iter().find(|p| p.id == id))
        .filter(|p| p.max > p.min)
        .map_or((0.0, 1.0), |p| (p.min, p.max))
}

fn value_y(rect: Rect, (lo, hi): (f64, f64), v: f64) -> f32 {
    let f = ((v - lo) / (hi - lo)).clamp(0.0, 1.0) as f32;
    rect.bottom() - PAD - f * (rect.height() - 2.0 * PAD)
}

fn y_value(rect: Rect, (lo, hi): (f64, f64), y: f32) -> f64 {
    let f = f64::from((rect.bottom() - PAD - y) / (rect.height() - 2.0 * PAD)).clamp(0.0, 1.0);
    lo + f * (hi - lo)
}

/// Punkt der Kurve nahe `p`.
fn point_at(w: &ProjectWindow, g: &Geo, row: usize, lane: usize, p: Pos2) -> Option<usize> {
    let track = w.project.tracks.get(row)?;
    let l = track.automation.get(lane)?;
    let rect = lane_rect(w, g, row, lane, g.lanes.x_range())?;
    let r = range(w, track, l.param);
    l.points.iter().position(|pt| {
        let at = pos2(g.x(pt.tick as f64), value_y(rect, r, pt.value));
        at.distance(p) <= POINT_PX
    })
}

/// Tick unter `x`, mit Snap außer bei Alt.
fn tick_at(w: &ProjectWindow, g: &Geo, x: f32, free: bool) -> u64 {
    let t = g.ticks(x).max(0.0);
    if free {
        t.round() as u64
    } else {
        w.snap_ticks(t)
    }
}

/// Mausdruck in einer Automationsspur: Punkt greifen oder setzen und greifen (Objektauswahl),
/// frei zeichnen (Zeichnen). `None` außerhalb der Automationsspuren.
pub(super) fn start_drag(
    w: &mut ProjectWindow,
    g: &Geo,
    p: Pos2,
    alt: bool,
) -> Option<Option<Drag>> {
    let row = g.row_at(p.y)?;
    let lane = lane_at(w, g, row, p.y)?;
    let drag = match w.tool {
        Tool::Select => {
            let idx = match point_at(w, g, row, lane, p) {
                Some(i) => Some(i),
                None => add_point(w, g, row, lane, p, alt),
            };
            idx.map(|idx| Drag::AutoPoint { row, lane, idx })
        }
        Tool::Draw => {
            let tick = tick_at(w, g, p.x, alt);
            add_point(w, g, row, lane, p, alt);
            Some(Drag::AutoDraw {
                row,
                lane,
                last: tick,
            })
        }
        _ => None,
    };
    Some(drag)
}

fn add_point(
    w: &mut ProjectWindow,
    g: &Geo,
    row: usize,
    lane: usize,
    p: Pos2,
    alt: bool,
) -> Option<usize> {
    let rect = lane_rect(w, g, row, lane, g.lanes.x_range())?;
    let tick = tick_at(w, g, p.x, alt);
    let track = w.project.tracks.get(row)?;
    let r = range(w, track, track.automation.get(lane)?.param);
    let v = y_value(rect, r, p.y);
    w.project.tracks[row]
        .automation
        .get_mut(lane)?
        .set_point(tick, v)
}

/// Während des Ziehens: Punkt bewegen oder weiterzeichnen.
pub(super) fn live_drag(w: &mut ProjectWindow, g: &Geo, d: Drag, p: Pos2, alt: bool) -> Drag {
    match d {
        Drag::AutoPoint { row, lane, idx } => {
            let tick = tick_at(w, g, p.x, alt);
            if let (Some(rect), Some(track)) = (
                lane_rect(w, g, row, lane, g.lanes.x_range()),
                w.project.tracks.get(row),
            ) && let Some(l) = track.automation.get(lane)
            {
                let v = y_value(rect, range(w, track, l.param), p.y);
                if let Some(l) = w.project.tracks[row].automation.get_mut(lane) {
                    l.move_point(idx, tick, v);
                }
            }
            d
        }
        Drag::AutoDraw { row, lane, last } => {
            let tick = tick_at(w, g, p.x, alt);
            // Freihand: Punkte zwischen dem letzten und dem neuen Tick werden ersetzt.
            if let Some(l) = w
                .project
                .tracks
                .get_mut(row)
                .and_then(|t| t.automation.get_mut(lane))
            {
                let (a, b) = (last.min(tick), last.max(tick));
                l.points.retain(|pt| pt.tick <= a || pt.tick >= b);
            }
            add_point(w, g, row, lane, p, alt);
            Drag::AutoDraw {
                row,
                lane,
                last: tick,
            }
        }
        other => other,
    }
}

/// Klick in eine Automationsspur. `true`, wenn der Klick dort war (dann nichts anderes tun).
pub(super) fn click(w: &mut ProjectWindow, g: &Geo, p: Pos2, alt: bool) -> bool {
    let Some(row) = g.row_at(p.y) else {
        return false;
    };
    let Some(lane) = lane_at(w, g, row, p.y) else {
        return false;
    };
    w.selected_track = Some(row);
    match w.tool {
        Tool::Erase => {
            if let Some(i) = point_at(w, g, row, lane, p)
                && let Some(l) = w.project.tracks[row].automation.get_mut(lane)
            {
                l.remove_point(i);
            }
        }
        Tool::Select | Tool::Draw if point_at(w, g, row, lane, p).is_none() => {
            add_point(w, g, row, lane, p, alt);
        }
        _ => {}
    }
    true
}

/// Zeichnet Hintergrund, Kurve und Punkte der sichtbaren Automationsspuren.
pub(super) fn paint(w: &ProjectWindow, painter: &Painter, g: &Geo) {
    let t = crate::theme::tokens(painter.ctx());
    let t0 = g.ticks(g.lanes.left()).max(0.0);
    let t1 = g.ticks(g.lanes.right()).max(0.0);
    for row in g.visible_rows() {
        let track = &w.project.tracks[row];
        let color = track_color(track.color);
        for lane in visible(track) {
            let Some(rect) = lane_rect(w, g, row, lane, g.lanes.x_range()) else {
                continue;
            };
            let l = &track.automation[lane];
            painter.rect_filled(rect, 0.0, t.lane_alt);
            painter.hline(rect.x_range(), rect.top(), Stroke::new(1.0, t.lane_bg));
            let r = range(w, track, l.param);
            let dim = if l.muted || !track.read { 0.45 } else { 1.0 };
            let stroke = Stroke::new(1.5, color.gamma_multiply(dim));
            let Some(first) = l.points.first() else {
                // Ohne Punkte: statischer Wert als dünne Linie.
                if let Some(v) = l.param.static_value(track) {
                    let y = value_y(rect, r, v);
                    painter.hline(
                        rect.x_range(),
                        y,
                        Stroke::new(1.0, color.gamma_multiply(0.35)),
                    );
                }
                continue;
            };
            let mut line = vec![pos2(rect.left(), value_y(rect, r, first.value))];
            let from = l
                .points
                .partition_point(|p| (p.tick as f64) < t0)
                .saturating_sub(1);
            let to = l
                .points
                .partition_point(|p| (p.tick as f64) <= t1)
                .saturating_add(1);
            let shown = &l.points[from..to.min(l.points.len())];
            for p in shown {
                line.push(pos2(g.x(p.tick as f64), value_y(rect, r, p.value)));
            }
            if let Some(last) = l.points.last() {
                line.push(pos2(rect.right(), value_y(rect, r, last.value)));
            }
            painter.with_clip_rect(rect).line(line, stroke);
            for p in shown {
                let c = pos2(g.x(p.tick as f64), value_y(rect, r, p.value));
                let sq = Rect::from_center_size(c, vec2(6.0, 6.0));
                painter.rect_filled(sq, 1.0, color.gamma_multiply(dim));
                painter.rect_stroke(
                    sq,
                    1.0,
                    Stroke::new(1.0, Color32::BLACK),
                    StrokeKind::Outside,
                );
            }
        }
    }
}

/// Schalter im Kopf einer Automationsspur: Mute und Entfernen.
fn header_buttons(rect: Rect) -> [Rect; 2] {
    let top = rect.top() + (rect.height() - 18.0) / 2.0;
    std::array::from_fn(|i| {
        let x = rect.right() - 6.0 - (2 - i) as f32 * 21.0;
        Rect::from_min_size(pos2(x, top), vec2(19.0, 18.0))
    })
}

/// Pfeil im Spurkopf, der die Automation auf- und zuklappt.
pub(super) fn arrow_rect(g: &Geo, row: usize) -> Rect {
    let top = g.row_top(row) + (g.view.row_h - 14.0) / 2.0;
    let left = g.header.left() + 8.0 + super::indent(g, row);
    Rect::from_min_size(pos2(left, top), vec2(12.0, 14.0))
}

/// Klick auf den Pfeil: alle Automationsspuren der Spur zeigen bzw. verbergen. Ohne
/// Automationsspur wird eine für die Lautstärke angelegt.
pub(super) fn toggle(track: &mut Track) {
    if !crate::mixer::has_channel(track.kind) {
        return;
    }
    if track.automation.is_empty() {
        track.automation_lane_mut(AutoParam::Volume);
        return;
    }
    let show = !track.automation.iter().any(|l| l.visible);
    for l in &mut track.automation {
        l.visible = show;
    }
}

/// Klick in den Kopf einer Automationsspur. `true`, wenn er dort war.
pub(super) fn header_click(w: &mut ProjectWindow, g: &Geo, p: Pos2) -> bool {
    let Some(row) = g.row_at(p.y) else {
        return false;
    };
    let Some(lane) = lane_at(w, g, row, p.y) else {
        return false;
    };
    let Some(rect) = lane_rect(w, g, row, lane, g.header.x_range()) else {
        return false;
    };
    let [mute, remove] = header_buttons(rect);
    let track = &mut w.project.tracks[row];
    if mute.contains(p) {
        if let Some(l) = track.automation.get_mut(lane) {
            l.muted = !l.muted;
        }
    } else if remove.contains(p) {
        track.automation.remove(lane);
    } else {
        w.selected_track = Some(row);
    }
    true
}

/// Zeichnet Pfeil und die Köpfe der Automationsspuren.
pub(super) fn paint_headers(w: &ProjectWindow, painter: &Painter, g: &Geo, row: usize) {
    let t = crate::theme::tokens(painter.ctx());
    let visuals = painter.ctx().global_style().visuals.clone();
    let track = &w.project.tracks[row];
    let folder = track.kind == studiocraft_project::TrackKind::Folder;
    if crate::mixer::has_channel(track.kind) || folder {
        let a = arrow_rect(g, row);
        // Ordner: Pfeil klappt die enthaltenen Spuren auf und zu; sonst die Automation.
        let open = if folder {
            !track.folded
        } else {
            track.automation.iter().any(|l| l.visible)
        };
        let pts = if open {
            vec![
                a.left_top() + vec2(1.0, 4.0),
                a.right_top() + vec2(-1.0, 4.0),
                a.center_bottom() - vec2(0.0, 3.0),
            ]
        } else {
            vec![
                a.left_top() + vec2(3.0, 1.0),
                a.right_center() - vec2(1.0, 0.0),
                a.left_bottom() + vec2(3.0, -1.0),
            ]
        };
        let c = if !folder && track.automation.is_empty() {
            visuals.weak_text_color()
        } else {
            t.text
        };
        painter.add(egui::Shape::convex_polygon(pts, c, Stroke::NONE));
    }
    for lane in visible(track) {
        let Some(rect) = lane_rect(w, g, row, lane, g.header.x_range()) else {
            continue;
        };
        let l = &track.automation[lane];
        painter.rect_filled(rect, 0.0, t.lane_alt);
        painter.rect_filled(
            Rect::from_min_size(rect.min, vec2(5.0, rect.height())),
            0.0,
            track_color(track.color).gamma_multiply(0.6),
        );
        let [mute, remove] = header_buttons(rect);
        painter
            .with_clip_rect(Rect::from_min_max(
                rect.min,
                pos2(mute.left() - 4.0, rect.max.y),
            ))
            .text(
                rect.left_center() + vec2(24.0, 0.0),
                Align2::LEFT_CENTER,
                w.param_name(row, l.param),
                FontId::proportional(12.0),
                t.text,
            );
        for (b, label, on, col) in [(mute, "M", l.muted, t.mute), (remove, "×", false, t.text)] {
            painter.rect_filled(
                b,
                3.0,
                if on {
                    col
                } else {
                    visuals.widgets.inactive.bg_fill
                },
            );
            painter.text(
                b.center(),
                Align2::CENTER_CENTER,
                label,
                FontId::proportional(11.0),
                if on { Color32::BLACK } else { t.text },
            );
        }
        painter.hline(
            rect.x_range(),
            rect.bottom(),
            Stroke::new(1.0, visuals.faint_bg_color),
        );
    }
}
