//! Spurliste, Lineal und Event-Anzeige: selbst gezeichnet, damit auch
//! hunderte Spuren flüssig bleiben (es wird nur gezeichnet, was sichtbar ist).

use egui::{
    Align2, Color32, FontId, Painter, Pos2, Rect, Sense, Stroke, StrokeKind, Ui, pos2, vec2,
};
use studiocraft_project::{Event, EventId, Track, TrackKind};

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
            row_h: 52.0,
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
    let drop_target = file_drop(w, ui, &geo);

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
    if let Some((row, ticks)) = drop_target {
        paint_drop_target(&painter.with_clip_rect(lanes), &geo, row, ticks);
    }
}

/// Audiodateien per Drag & Drop: auf eine Audio-Spur an die Mausposition,
/// sonst auf neue Spuren. Gibt das Ziel zurück, solange Dateien darüber schweben.
fn file_drop(w: &mut ProjectWindow, ui: &Ui, g: &Geo) -> Option<(Option<usize>, u64)> {
    let (hovering, dropped, pos) = ui.input(|i| {
        let paths: Vec<std::path::PathBuf> = i
            .raw
            .dropped_files
            .iter()
            .map(|f| f.path().to_path_buf())
            .collect();
        (
            !i.raw.hovered_files.is_empty(),
            paths,
            i.pointer.latest_pos(),
        )
    });
    let target = pos.filter(|p| g.lanes.contains(*p)).map(|p| {
        let row = g
            .row_at(p.y)
            .filter(|&r| w.project.tracks[r].kind == TrackKind::Audio);
        (row, w.snap_ticks(g.ticks(p.x)))
    });
    if !dropped.is_empty() {
        let (row, at) = target.unwrap_or((None, 0));
        let mut errors = Vec::new();
        for (i, path) in dropped.iter().enumerate() {
            // Erste Datei auf die Zielspur, weitere auf eigene neue Spuren.
            let t = if i == 0 { row.map(|r| (r, at)) } else { None };
            let t = t.or(Some((usize::MAX, at)));
            if let Err(e) = w.import_audio(path, t) {
                errors.push(e);
            }
        }
        w.message = (!errors.is_empty()).then(|| errors.join("; "));
        return None;
    }
    if hovering { target } else { None }
}

fn paint_drop_target(painter: &Painter, g: &Geo, row: Option<usize>, ticks: u64) {
    let x = g.x(ticks as f64);
    let accent = painter.ctx().global_style().visuals.selection.stroke.color;
    match row {
        Some(r) => {
            let top = g.row_top(r);
            let rect = Rect::from_min_max(pos2(x, top), pos2(g.lanes.right(), top + g.view.row_h));
            painter.rect_stroke(rect, 2.0, Stroke::new(2.0, accent), StrokeKind::Inside);
        }
        None => {
            painter.vline(x, g.lanes.y_range(), Stroke::new(2.0, accent));
            painter.text(
                pos2(x + 6.0, g.lanes.top() + 6.0),
                Align2::LEFT_TOP,
                "Neue Audio-Spur",
                FontId::proportional(12.0),
                accent,
            );
        }
    }
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
            w.dirty = true;
        }
        return;
    }
    if resp.clicked()
        && let Some(p) = resp.interact_pointer_pos()
    {
        click(w, g, p, mods);
        w.dirty = true;
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
            paint_event(w, painter, g, track, ev, rect);
        }
    }
    paint_drag_preview(w, painter, g);
}

/// Ein Event: farbige Namensleiste oben, darunter der Inhalt (bei Audio die Wellenform).
fn paint_event(
    w: &ProjectWindow,
    painter: &Painter,
    g: &Geo,
    track: &Track,
    ev: &Event,
    rect: Rect,
) {
    const HEAD: f32 = 13.0;
    let color = if ev.muted {
        Color32::from_gray(120)
    } else {
        track_color(track.color)
    };
    let body_fill = if ev.muted {
        Color32::from_gray(45)
    } else {
        color.lerp_to_gamma(Color32::from_gray(18), 0.62)
    };
    painter.rect_filled(rect, 3.0, body_fill);
    let head = Rect::from_min_max(
        rect.min,
        pos2(rect.right(), (rect.top() + HEAD).min(rect.bottom())),
    );
    painter.rect_filled(
        head,
        egui::CornerRadius {
            nw: 3,
            ne: 3,
            sw: 0,
            se: 0,
        },
        color.gamma_multiply(if ev.muted { 0.6 } else { 1.0 }),
    );

    let visible = rect.intersect(g.lanes);
    let body = Rect::from_min_max(pos2(rect.left(), head.bottom()), rect.max);
    if let Some(clip) = ev.clip.and_then(|c| w.clip(c)) {
        let wave = if ev.muted {
            Color32::from_gray(110)
        } else {
            color.lerp_to_gamma(Color32::WHITE, 0.35)
        };
        paint_waveform(w, painter, g, ev, clip, body.intersect(g.lanes), wave);
    }

    let selected = w.selection.contains(&ev.id);
    let stroke = if selected {
        Stroke::new(2.0, Color32::WHITE)
    } else {
        Stroke::new(1.0, color.gamma_multiply(0.8))
    };
    painter.rect_stroke(rect, 3.0, stroke, StrokeKind::Inside);

    if visible.width() > 24.0 {
        // Name am sichtbaren linken Rand festhalten, auch wenn das Event links hinausragt.
        painter
            .with_clip_rect(head.intersect(visible).shrink2(egui::vec2(3.0, 0.0)))
            .text(
                pos2(visible.left() + 5.0, head.center().y),
                Align2::LEFT_CENTER,
                &ev.name,
                FontId::proportional(10.5),
                Color32::from_gray(if ev.muted { 200 } else { 10 }),
            );
    }
}

fn paint_waveform(
    w: &ProjectWindow,
    painter: &Painter,
    g: &Geo,
    ev: &Event,
    clip: &crate::LoadedClip,
    area: Rect,
    color: Color32,
) {
    if area.width() < 1.0 || area.height() < 4.0 {
        return;
    }
    let tempo = &w.project.tempo;
    // Frames des Clips pro Tick.
    let fpt = f64::from(w.sample_rate) * 60.0 / (tempo.bpm * TPQ);
    let mid = area.center().y;
    let half = area.height() * 0.5 - 1.0;
    let mut shapes = Vec::with_capacity(area.width() as usize + 1);
    let mut x = area.left().floor();
    while x < area.right() {
        let f0 = ev.offset as f64 + (g.ticks(x) - ev.start.0 as f64) * fpt;
        let f1 = ev.offset as f64 + (g.ticks(x + 1.0) - ev.start.0 as f64) * fpt;
        let (lo, hi) = clip.peaks.range(f0, f1.max(f0 + 1.0));
        let (y0, y1) = (
            mid - hi.clamp(-1.0, 1.0) * half,
            mid - lo.clamp(-1.0, 1.0) * half,
        );
        shapes.push(egui::Shape::line_segment(
            [pos2(x + 0.5, y0), pos2(x + 0.5, y1.max(y0 + 1.0))],
            Stroke::new(1.0, color),
        ));
        x += 1.0;
    }
    painter.extend(shapes);
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
