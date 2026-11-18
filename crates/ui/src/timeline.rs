//! Spurliste, Lineal und Event-Anzeige: selbst gezeichnet, damit auch
//! hunderte Spuren flüssig bleiben (es wird nur gezeichnet, was sichtbar ist).

use egui::{
    Align2, Color32, FontId, Painter, Pos2, Rect, Sense, Stroke, StrokeKind, Ui, pos2, vec2,
};
use studiocraft_project::EventId;

use crate::{ProjectWindow, TPQ, Tool};

const HEADER_W: f32 = 220.0;
const RULER_H: f32 = 30.0;

#[derive(Debug, Clone, Copy)]
pub(crate) struct View {
    /// Pixel pro Viertelnote (horizontaler Zoom).
    pub px_per_quarter: f32,
    /// Linker Rand der Event-Anzeige in Ticks.
    pub scroll_ticks: f64,
    /// Vertikaler Versatz in Pixel.
    pub scroll_y: f32,
    pub row_h: f32,
}

impl Default for View {
    fn default() -> Self {
        Self {
            px_per_quarter: 24.0,
            scroll_ticks: 0.0,
            scroll_y: 0.0,
            row_h: 34.0,
        }
    }
}

impl View {
    /// Zoomt horizontal; `anchor_x` (Pixel ab linkem Rand der Event-Anzeige) bleibt stehen.
    pub fn zoom(&mut self, factor: f32, anchor_x: Option<f32>) {
        let x = anchor_x.unwrap_or(0.0);
        let anchor_ticks = self.scroll_ticks + f64::from(x / self.px_per_quarter) * TPQ;
        self.px_per_quarter = (self.px_per_quarter * factor).clamp(1.0, 2000.0);
        self.scroll_ticks = (anchor_ticks - f64::from(x / self.px_per_quarter) * TPQ).max(0.0);
    }
}

#[derive(Debug, Clone, Copy)]
pub(crate) enum Drag {
    /// Auswahl verschieben; `grab` = Abstand Mauszeiger zu Event-Start in Ticks.
    Move { id: EventId, grab: f64, row: usize },
    /// Neues Event aufziehen.
    Draw { row: usize, start: u64 },
    /// Cycle-Bereich im Lineal aufziehen.
    Ruler { start: u64 },
}

struct Geo {
    header: Rect,
    ruler: Rect,
    lanes: Rect,
    view: View,
    rows: usize,
}

impl Geo {
    fn x(&self, ticks: f64) -> f32 {
        self.lanes.left()
            + ((ticks - self.view.scroll_ticks) / TPQ) as f32 * self.view.px_per_quarter
    }
    fn ticks(&self, x: f32) -> f64 {
        self.view.scroll_ticks + f64::from((x - self.lanes.left()) / self.view.px_per_quarter) * TPQ
    }
    fn row_top(&self, row: usize) -> f32 {
        self.lanes.top() + row as f32 * self.view.row_h - self.view.scroll_y
    }
    fn row_at(&self, y: f32) -> Option<usize> {
        let r = ((y - self.lanes.top() + self.view.scroll_y) / self.view.row_h).floor();
        (r >= 0.0 && (r as usize) < self.rows).then_some(r as usize)
    }
    fn visible_rows(&self) -> std::ops::Range<usize> {
        let first = (self.view.scroll_y / self.view.row_h).floor().max(0.0) as usize;
        let count = (self.lanes.height() / self.view.row_h).ceil() as usize + 1;
        first.min(self.rows)..(first + count).min(self.rows)
    }
    /// Rechtecke der Knöpfe M, S, R im Spurkopf.
    fn header_buttons(&self, row: usize) -> [Rect; 3] {
        let top = self.row_top(row) + (self.view.row_h - 18.0) / 2.0;
        let right = self.header.right() - 6.0;
        std::array::from_fn(|i| {
            let x = right - (3 - i) as f32 * 22.0;
            Rect::from_min_size(pos2(x, top), vec2(20.0, 18.0))
        })
    }
}

pub(crate) fn show(w: &mut ProjectWindow, ui: &mut Ui) {
    let (resp, painter) = ui.allocate_painter(ui.available_size(), Sense::click_and_drag());
    let r = resp.rect;
    let header = Rect::from_min_max(
        pos2(r.left(), r.top() + RULER_H),
        pos2(r.left() + HEADER_W, r.bottom()),
    );
    let ruler = Rect::from_min_max(
        pos2(r.left() + HEADER_W, r.top()),
        pos2(r.right(), r.top() + RULER_H),
    );
    let lanes = Rect::from_min_max(pos2(r.left() + HEADER_W, r.top() + RULER_H), r.max);
    let rows = w.project.tracks.len();

    scroll_and_zoom(w, ui, &resp, lanes, rows);
    follow_cursor(w, lanes);
    let geo = Geo {
        header,
        ruler,
        lanes,
        view: w.view,
        rows,
    };

    interact(w, ui, &resp, &geo);

    let geo = Geo {
        view: w.view,
        ..geo
    };
    paint_lanes(w, &painter.with_clip_rect(lanes), &geo);
    paint_ruler(w, &painter.with_clip_rect(ruler), &geo);
    paint_headers(w, &painter.with_clip_rect(header), &geo);
    let corner = Rect::from_min_max(r.min, pos2(header.right(), ruler.bottom()));
    painter.rect_filled(corner, 0.0, ui.visuals().faint_bg_color);
    painter.text(
        corner.left_center() + vec2(8.0, 0.0),
        Align2::LEFT_CENTER,
        format!("{rows} Spuren"),
        FontId::proportional(12.0),
        ui.visuals().weak_text_color(),
    );
    paint_cursor(w, &painter.with_clip_rect(ruler.union(lanes)), &geo);
}

fn scroll_and_zoom(
    w: &mut ProjectWindow,
    ui: &Ui,
    resp: &egui::Response,
    lanes: Rect,
    rows: usize,
) {
    if resp.hovered() {
        let (delta, zoom, mods, hover) = ui.input(|i| {
            (
                i.smooth_scroll_delta,
                i.zoom_delta(),
                i.modifiers,
                i.pointer.hover_pos(),
            )
        });
        let anchor = hover.map(|p| p.x - lanes.left());
        if zoom != 1.0 {
            w.view.zoom(zoom, anchor);
        } else if mods.shift {
            w.view.scroll_ticks -= f64::from((delta.y + delta.x) / w.view.px_per_quarter) * TPQ;
        } else {
            w.view.scroll_y -= delta.y;
            w.view.scroll_ticks -= f64::from(delta.x / w.view.px_per_quarter) * TPQ;
        }
    }
    let max_y = (rows as f32 * w.view.row_h - lanes.height() + w.view.row_h).max(0.0);
    w.view.scroll_y = w.view.scroll_y.clamp(0.0, max_y);
    w.view.scroll_ticks = w.view.scroll_ticks.max(0.0);
}

/// Während der Wiedergabe blättert die Ansicht mit, wenn der Cursor rechts hinausläuft.
fn follow_cursor(w: &mut ProjectWindow, lanes: Rect) {
    if !w.is_playing() {
        return;
    }
    let visible = f64::from(lanes.width() / w.view.px_per_quarter) * TPQ;
    let c = w.cursor as f64;
    if c > w.view.scroll_ticks + visible * 0.95 || c < w.view.scroll_ticks {
        w.view.scroll_ticks = (c - visible * 0.05).max(0.0);
    }
}

fn interact(w: &mut ProjectWindow, ui: &Ui, resp: &egui::Response, g: &Geo) {
    let (mods, press, now) = ui.input(|i| {
        (
            i.modifiers,
            i.pointer.press_origin(),
            i.pointer.interact_pos(),
        )
    });

    if resp.drag_started()
        && let Some(p) = press
    {
        w.drag = start_drag(w, g, p);
    }
    if resp.dragged()
        && let (Some(Drag::Ruler { start }), Some(p)) = (w.drag, now)
    {
        let t = w.snap_ticks(g.ticks(p.x));
        w.left_locator = start.min(t);
        w.right_locator = start.max(t);
    }
    if resp.drag_stopped() {
        if let (Some(d), Some(p)) = (w.drag.take(), now) {
            finish_drag(w, g, d, p, mods);
        }
        return;
    }
    if resp.clicked()
        && let Some(p) = resp.interact_pointer_pos()
    {
        click(w, g, p, mods);
    }
}

fn start_drag(w: &mut ProjectWindow, g: &Geo, p: Pos2) -> Option<Drag> {
    if g.ruler.contains(p) {
        return Some(Drag::Ruler {
            start: w.snap_ticks(g.ticks(p.x)),
        });
    }
    if !g.lanes.contains(p) {
        return None;
    }
    let row = g.row_at(p.y)?;
    let ticks = g.ticks(p.x);
    match w.tool {
        Tool::Select => {
            let id = w.project.event_at(row, ticks.max(0.0) as u64)?;
            if !w.selection.contains(&id) {
                w.selection = vec![id];
            }
            w.selected_track = Some(row);
            let start = w.project.event(id)?.start.0 as f64;
            Some(Drag::Move {
                id,
                grab: ticks - start,
                row,
            })
        }
        Tool::Draw if w.project.tracks[row].kind.holds_events() => Some(Drag::Draw {
            row,
            start: w.snap_ticks(ticks),
        }),
        _ => None,
    }
}

/// Ziel einer Verschiebung: (Zeilenversatz, Tick-Versatz) gegenüber dem gegriffenen Event.
fn move_delta(
    w: &ProjectWindow,
    g: &Geo,
    id: EventId,
    grab: f64,
    row: usize,
    p: Pos2,
    no_snap: bool,
) -> Option<(isize, i64)> {
    let ev = w.project.event(id)?;
    let raw = g.ticks(p.x) - grab;
    let new_start = if no_snap {
        raw.max(0.0).round() as u64
    } else {
        w.snap_ticks(raw)
    };
    let target_row = g
        .row_at(p.y.clamp(g.lanes.top(), g.lanes.bottom() - 1.0))
        .unwrap_or(row);
    Some((
        target_row as isize - row as isize,
        new_start as i64 - ev.start.0 as i64,
    ))
}

fn finish_drag(w: &mut ProjectWindow, g: &Geo, d: Drag, p: Pos2, mods: egui::Modifiers) {
    match d {
        Drag::Ruler { .. } => w.send_locators(),
        Drag::Draw { row, start } => {
            let end = w.snap_ticks(g.ticks(p.x));
            let min = w.grid.ticks(&w.project);
            let (a, b) = (start.min(end), start.max(end));
            let len = (b - a).max(min);
            if let Some(id) = w.project.create_event(row, a, len) {
                w.selection = vec![id];
            }
        }
        Drag::Move { id, grab, row } => {
            let Some((drow, dt)) = move_delta(w, g, id, grab, row, p, mods.alt) else {
                return;
            };
            if drow == 0 && dt == 0 {
                return;
            }
            let copy = mods.command;
            let mut new_sel = Vec::new();
            for sel in w.selection.clone() {
                let Some((t, _)) = w.project.find(sel) else {
                    continue;
                };
                let Some(ev) = w.project.event(sel) else {
                    continue;
                };
                let target = (t as isize + drow).clamp(0, g.rows as isize - 1) as usize;
                let start = (ev.start.0 as i64 + dt).max(0) as u64;
                match w.project.move_event(sel, target, start, copy) {
                    Some(nid) => new_sel.push(nid),
                    None => new_sel.push(sel),
                }
            }
            w.selection = new_sel;
        }
    }
}

fn click(w: &mut ProjectWindow, g: &Geo, p: Pos2, mods: egui::Modifiers) {
    if g.ruler.contains(p) {
        let t = w.snap_ticks(g.ticks(p.x));
        if mods.command {
            w.left_locator = t;
        } else if mods.alt {
            w.right_locator = t;
        } else {
            w.locate(t);
            return;
        }
        if w.left_locator > w.right_locator {
            std::mem::swap(&mut w.left_locator, &mut w.right_locator);
        }
        w.send_locators();
        return;
    }
    if g.header.contains(p) {
        if let Some(row) = g.row_at(p.y) {
            let [m, s, r] = g.header_buttons(row);
            let track = &mut w.project.tracks[row];
            if m.contains(p) {
                track.mute = !track.mute;
            } else if s.contains(p) {
                track.solo = !track.solo;
            } else if r.contains(p) && track.kind.holds_events() {
                track.record = !track.record;
            } else {
                w.selected_track = Some(row);
            }
        }
        return;
    }
    if !g.lanes.contains(p) {
        return;
    }
    let Some(row) = g.row_at(p.y) else { return };
    let ticks = g.ticks(p.x).max(0.0);
    let hit = w.project.event_at(row, ticks as u64);
    w.selected_track = Some(row);
    match (w.tool, hit) {
        (Tool::Select, Some(id)) => {
            if mods.shift {
                if let Some(i) = w.selection.iter().position(|&s| s == id) {
                    w.selection.remove(i);
                } else {
                    w.selection.push(id);
                }
            } else {
                w.selection = vec![id];
            }
        }
        (Tool::Select, None) => w.selection.clear(),
        (Tool::Split, Some(id)) => {
            let at = if mods.alt {
                ticks as u64
            } else {
                w.snap_ticks(ticks)
            };
            w.project.split(id, at);
        }
        (Tool::Erase, Some(id)) => {
            w.project.delete(id);
            w.selection.retain(|&s| s != id);
        }
        (Tool::Mute, Some(id)) => {
            if let Some(ev) = w.project.event_mut(id) {
                ev.muted = !ev.muted;
            }
        }
        (Tool::Draw, None) => {
            let start = w.snap_ticks(ticks);
            let len = w.bar_ticks();
            if let Some(id) = w.project.create_event(row, start, len) {
                w.selection = vec![id];
            }
        }
        (Tool::Zoom, _) => {
            let f = if mods.alt { 1.0 / 1.5 } else { 1.5 };
            w.view.zoom(f, Some(p.x - g.lanes.left()));
        }
        (Tool::Play, _) => {
            w.locate(w.snap_ticks(ticks));
            if !w.is_playing() {
                w.toggle_play();
            }
        }
        _ => {}
    }
}

fn track_color(c: [u8; 3]) -> Color32 {
    Color32::from_rgb(c[0], c[1], c[2])
}

/// Abstand der Taktlinien, damit Beschriftungen nicht überlappen.
fn bar_step(w: &ProjectWindow) -> u64 {
    let bar_px = w.bar_ticks() as f32 / TPQ as f32 * w.view.px_per_quarter;
    let mut step = 1;
    while bar_px * step as f32 * 1.0 < 40.0 {
        step *= 2;
    }
    step
}

fn paint_grid(w: &ProjectWindow, painter: &Painter, g: &Geo, rect: Rect, labels: bool) {
    let visuals = painter.ctx().global_style().visuals.clone();
    let bar = w.bar_ticks();
    let beat = w.beat_ticks();
    let beat_px = beat as f32 / TPQ as f32 * w.view.px_per_quarter;
    let t0 = g.ticks(rect.left()).max(0.0) as u64;
    let t1 = g.ticks(rect.right()).max(0.0) as u64 + bar;
    let step = bar_step(w);
    let strong = Stroke::new(1.0, visuals.widgets.noninteractive.bg_stroke.color);
    let weak = Stroke::new(
        1.0,
        visuals
            .widgets
            .noninteractive
            .bg_stroke
            .color
            .gamma_multiply(0.45),
    );
    let mut b = t0 / bar;
    while b * bar <= t1 {
        let x = g.x((b * bar) as f64);
        if b % step == 0 {
            painter.vline(x, rect.y_range(), strong);
            if labels {
                painter.text(
                    pos2(x + 3.0, rect.bottom() - 4.0),
                    Align2::LEFT_BOTTOM,
                    (b + 1).to_string(),
                    FontId::proportional(11.0),
                    visuals.text_color(),
                );
            }
        }
        if beat_px >= 10.0 && !labels {
            for k in 1..(bar / beat) {
                painter.vline(g.x((b * bar + k * beat) as f64), rect.y_range(), weak);
            }
        }
        b += 1;
    }
}

fn paint_lanes(w: &ProjectWindow, painter: &Painter, g: &Geo) {
    let visuals = painter.ctx().global_style().visuals.clone();
    painter.rect_filled(g.lanes, 0.0, visuals.extreme_bg_color);
    for row in g.visible_rows() {
        let top = g.row_top(row);
        let rect = Rect::from_min_size(
            pos2(g.lanes.left(), top),
            vec2(g.lanes.width(), g.view.row_h),
        );
        if row % 2 == 1 {
            painter.rect_filled(rect, 0.0, visuals.faint_bg_color.gamma_multiply(0.5));
        }
        painter.hline(
            rect.x_range(),
            rect.bottom(),
            Stroke::new(1.0, visuals.faint_bg_color),
        );
    }
    paint_grid(w, painter, g, g.lanes, false);

    if w.cycle {
        let r = Rect::from_x_y_ranges(
            g.x(w.left_locator as f64)..=g.x(w.right_locator as f64),
            g.lanes.y_range(),
        );
        painter.rect_filled(r, 0.0, Color32::from_rgba_unmultiplied(120, 200, 120, 28));
    }

    let t0 = g.ticks(g.lanes.left()).max(0.0) as u64;
    let t1 = g.ticks(g.lanes.right()).max(0.0) as u64;
    for row in g.visible_rows() {
        let track = &w.project.tracks[row];
        let top = g.row_top(row) + 2.0;
        let h = g.view.row_h - 4.0;
        for ev in track
            .events
            .iter()
            .filter(|e| e.end() >= t0 && e.start.0 <= t1)
        {
            let rect = Rect::from_min_max(
                pos2(g.x(ev.start.0 as f64), top),
                pos2(g.x(ev.end() as f64), top + h),
            );
            let base = if ev.muted {
                Color32::GRAY
            } else {
                track_color(track.color)
            };
            painter.rect_filled(
                rect,
                3.0,
                base.gamma_multiply(if ev.muted { 0.35 } else { 0.75 }),
            );
            let selected = w.selection.contains(&ev.id);
            let stroke = if selected {
                Stroke::new(2.0, Color32::WHITE)
            } else {
                Stroke::new(1.0, base)
            };
            painter.rect_stroke(rect, 3.0, stroke, StrokeKind::Inside);
            let visible = rect.intersect(g.lanes);
            if visible.width() > 30.0 {
                // Name am sichtbaren linken Rand festhalten, auch wenn das Event links hinausragt.
                painter.with_clip_rect(visible.shrink(2.0)).text(
                    pos2(visible.left() + 5.0, rect.top() + 3.0),
                    Align2::LEFT_TOP,
                    &ev.name,
                    FontId::proportional(11.0),
                    Color32::from_gray(if ev.muted { 160 } else { 15 }),
                );
            }
        }
    }
    paint_drag_preview(w, painter, g);
}

fn paint_drag_preview(w: &ProjectWindow, painter: &Painter, g: &Geo) {
    let Some(p) = painter.ctx().input(|i| i.pointer.interact_pos()) else {
        return;
    };
    let alt = painter.ctx().input(|i| i.modifiers.alt);
    let ghost = Stroke::new(1.5, Color32::from_white_alpha(200));
    match w.drag {
        Some(Drag::Move { id, grab, row }) => {
            let Some((drow, dt)) = move_delta(w, g, id, grab, row, p, alt) else {
                return;
            };
            for &sel in &w.selection {
                let (Some((t, _)), Some(ev)) = (w.project.find(sel), w.project.event(sel)) else {
                    continue;
                };
                let target = (t as isize + drow).clamp(0, g.rows as isize - 1) as usize;
                let start = (ev.start.0 as i64 + dt).max(0) as f64;
                let top = g.row_top(target) + 2.0;
                let rect = Rect::from_min_max(
                    pos2(g.x(start), top),
                    pos2(g.x(start + ev.length as f64), top + g.view.row_h - 4.0),
                );
                painter.rect_stroke(rect, 3.0, ghost, StrokeKind::Inside);
            }
        }
        Some(Drag::Draw { row, start }) => {
            let end = w.snap_ticks(g.ticks(p.x));
            let top = g.row_top(row) + 2.0;
            let rect = Rect::from_x_y_ranges(
                g.x(start.min(end) as f64)..=g.x(start.max(end) as f64),
                top..=top + g.view.row_h - 4.0,
            );
            painter.rect_stroke(rect, 3.0, ghost, StrokeKind::Inside);
        }
        _ => {}
    }
}

fn paint_ruler(w: &ProjectWindow, painter: &Painter, g: &Geo) {
    let visuals = painter.ctx().global_style().visuals.clone();
    painter.rect_filled(g.ruler, 0.0, visuals.faint_bg_color);
    let band = Rect::from_x_y_ranges(
        g.x(w.left_locator as f64)..=g.x(w.right_locator as f64),
        g.ruler.top() + 2.0..=g.ruler.top() + 10.0,
    );
    let col = if w.cycle {
        Color32::from_rgb(90, 180, 90)
    } else {
        Color32::from_gray(110)
    };
    painter.rect_filled(band, 2.0, col);
    paint_grid(w, painter, g, g.ruler, true);
}

fn paint_headers(w: &ProjectWindow, painter: &Painter, g: &Geo) {
    let visuals = painter.ctx().global_style().visuals.clone();
    painter.rect_filled(g.header, 0.0, visuals.panel_fill);
    for row in g.visible_rows() {
        let track = &w.project.tracks[row];
        let top = g.row_top(row);
        let rect = Rect::from_min_size(
            pos2(g.header.left(), top),
            vec2(g.header.width(), g.view.row_h),
        );
        if w.selected_track == Some(row) {
            painter.rect_filled(rect, 0.0, visuals.selection.bg_fill.gamma_multiply(0.5));
        }
        painter.rect_filled(
            Rect::from_min_size(rect.min, vec2(5.0, rect.height())),
            0.0,
            track_color(track.color),
        );
        painter.text(
            rect.left_center() + vec2(12.0, -6.0),
            Align2::LEFT_CENTER,
            &track.name,
            FontId::proportional(13.0),
            visuals.strong_text_color(),
        );
        painter.text(
            rect.left_center() + vec2(12.0, 8.0),
            Align2::LEFT_CENTER,
            track.kind.label(),
            FontId::proportional(10.0),
            visuals.weak_text_color(),
        );
        let flags = [
            ("M", track.mute, Color32::from_rgb(220, 170, 40), true),
            ("S", track.solo, Color32::from_rgb(230, 210, 60), true),
            (
                "R",
                track.record,
                Color32::from_rgb(220, 60, 60),
                track.kind.holds_events(),
            ),
        ];
        for (b, (label, on, color, shown)) in g.header_buttons(row).into_iter().zip(flags) {
            if !shown {
                continue;
            }
            let fill = if on {
                color
            } else {
                visuals.widgets.inactive.bg_fill
            };
            painter.rect_filled(b, 3.0, fill);
            let text = if on {
                Color32::BLACK
            } else {
                visuals.text_color()
            };
            painter.text(
                b.center(),
                Align2::CENTER_CENTER,
                label,
                FontId::proportional(11.0),
                text,
            );
        }
        painter.hline(
            rect.x_range(),
            rect.bottom(),
            Stroke::new(1.0, visuals.faint_bg_color),
        );
    }
}

fn paint_cursor(w: &ProjectWindow, painter: &Painter, g: &Geo) {
    let x = g.x(w.cursor as f64);
    let y = g.ruler.top()..=g.lanes.bottom();
    painter.vline(x, y, Stroke::new(1.5, Color32::from_rgb(230, 60, 60)));
}
