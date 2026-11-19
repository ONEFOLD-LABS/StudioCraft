//! Spurliste, Lineal und Event-Anzeige: selbst gezeichnet, damit auch
//! hunderte Spuren flüssig bleiben (es wird nur gezeichnet, was sichtbar ist).

use egui::{
    Align2, Color32, FontId, Painter, Pos2, Rect, Sense, Stroke, StrokeKind, Ui, pos2, vec2,
};
use studiocraft_project::{Event, EventId, TimeBase, Track, TrackKind};

use crate::icons::Icon;
use crate::{ProjectWindow, TPQ, Tool};

mod automation;
pub(crate) use automation::range as param_range;

const HEADER_W: f32 = 290.0;
/// Anzahl der Schalter im Spurkopf.
const HEADER_BUTTONS: usize = 8;
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
    /// Punkt `idx` der Automationsspur `lane` (Index in `Track::automation`) ziehen.
    AutoPoint {
        row: usize,
        lane: usize,
        idx: usize,
    },
    /// Comping: Abschnitt eines Takes ab `start` aufziehen.
    Comp {
        row: usize,
        id: EventId,
        start: u64,
    },
    /// Automation frei zeichnen; `last` = zuletzt gezeichneter Tick.
    AutoDraw {
        row: usize,
        lane: usize,
        last: u64,
    },
    /// Auswahl verschieben; `grab` = Abstand Mauszeiger zu Event-Start in Ticks.
    Move {
        id: EventId,
        grab: f64,
        row: usize,
    },
    /// Neues Event aufziehen.
    Draw {
        row: usize,
        start: u64,
    },
    /// Cycle-Bereich im Lineal aufziehen.
    Ruler {
        start: u64,
    },
    /// Bereichsauswahl aufziehen (Werkzeug 2).
    Range {
        row: usize,
        start: u64,
    },
    /// Länge am linken bzw. rechten Rand ändern (untere Hälfte des Rands).
    TrimStart {
        id: EventId,
    },
    TrimEnd {
        id: EventId,
    },
    /// Fade-Griffe (obere Hälfte des Rands bzw. am Griff).
    FadeIn {
        id: EventId,
    },
    FadeOut {
        id: EventId,
    },
}

/// Fangbereich um Ränder und Fade-Griffe in Pixel.
const EDGE: f32 = 7.0;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Zone {
    Body,
    TrimStart,
    TrimEnd,
    FadeIn,
    FadeOut,
}

/// Smart-Bereiche eines Events unter dem Mauszeiger (Spec „Werkzeuge“):
/// obere Hälfte am Rand bzw. am Griff = Fade, untere Hälfte am Rand = Länge.
fn smart_zone(w: &ProjectWindow, g: &Geo, p: Pos2) -> Option<(EventId, usize, Zone)> {
    if !g.lanes.contains(p) {
        return None;
    }
    let row = g.row_at(p.y)?;
    let px = |dx: f32| f64::from(dx / g.view.px_per_quarter) * TPQ;
    // Etwas Toleranz, damit man den Rand auch knapp außerhalb trifft.
    let t = g.ticks(p.x);
    let id = hit_event(w, g, row, p.y, t.max(0.0) as u64)
        .or_else(|| hit_event(w, g, row, p.y, (t - px(EDGE)).max(0.0) as u64))
        .or_else(|| hit_event(w, g, row, p.y, (t + px(EDGE)).max(0.0) as u64))?;
    let ev = w.project.event(id)?;
    let lane_top = g.row_top(row) + lane_of(&w.project.tracks[row], id) as f32 * g.view.row_h;
    let x0 = g.x(ev.start.0 as f64);
    let x1 = g.x(ev.end() as f64);
    let upper = p.y < lane_top + g.view.row_h * 0.5;
    let fin = g.x((ev.start.0 + ev.fade_in) as f64);
    let fout = g.x((ev.end() - ev.fade_out) as f64);
    let zone = if upper && (p.x - fin).abs() <= EDGE && (p.x - fin).abs() <= (p.x - fout).abs() {
        Zone::FadeIn
    } else if upper && (p.x - fout).abs() <= EDGE {
        Zone::FadeOut
    } else if !upper && (p.x - x0).abs() <= EDGE {
        Zone::TrimStart
    } else if !upper && (x1 - p.x).abs() <= EDGE {
        Zone::TrimEnd
    } else if p.x >= x0 && p.x <= x1 {
        Zone::Body
    } else {
        return None;
    };
    Some((id, row, zone))
}

struct Geo {
    header: Rect,
    ruler: Rect,
    lanes: Rect,
    view: View,
    rows: usize,
    /// Oberkante jeder Spur ab Inhaltsanfang, dazu die Gesamthöhe (Länge `rows + 1`).
    /// Spuren mit aufgeklappten Lanes sind so hoch wie alle ihre Lanes.
    tops: Vec<f32>,
}

/// Oberkanten aller Spuren und Gesamthöhe (siehe [`Geo::tops`]).
fn row_tops(w: &ProjectWindow) -> Vec<f32> {
    let mut tops = Vec::with_capacity(w.project.tracks.len() + 1);
    let mut y = 0.0;
    tops.push(y);
    for t in &w.project.tracks {
        let lanes = automation::rows_of(t);
        y += w.view.row_h * lanes as f32;
        tops.push(y);
    }
    tops
}

/// Lane eines Events in seiner Spur (0 ohne aufgeklappte Lanes).
fn lane_of(track: &Track, id: EventId) -> usize {
    if !track.show_lanes {
        return 0;
    }
    let (lanes, _) = track.lanes();
    track
        .events
        .iter()
        .position(|e| e.id == id)
        .and_then(|i| lanes.get(i).copied())
        .unwrap_or(0)
}

/// Das Event unter (`y`, `ticks`) in Spur `row`; bei aufgeklappten Lanes nur in der Lane
/// unter `y`, sonst das oberste (klingende).
fn hit_event(w: &ProjectWindow, g: &Geo, row: usize, y: f32, ticks: u64) -> Option<EventId> {
    let track = w.project.tracks.get(row)?;
    let lane = ((y - g.row_top(row)) / g.view.row_h).floor().max(0.0) as usize;
    // Darunter liegen die Automationsspuren.
    if lane >= automation::take_lanes(track) {
        return None;
    }
    if !track.show_lanes {
        return w.project.event_at(row, ticks);
    }
    let (lanes, _) = track.lanes();
    track
        .events
        .iter()
        .zip(lanes)
        .rev()
        .find(|(e, l)| *l == lane && e.contains(ticks))
        .map(|(e, _)| e.id)
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
        let y = self.tops.get(row).copied().unwrap_or(0.0);
        self.lanes.top() + y - self.view.scroll_y
    }
    fn row_height(&self, row: usize) -> f32 {
        match (self.tops.get(row), self.tops.get(row + 1)) {
            (Some(a), Some(b)) => b - a,
            _ => self.view.row_h,
        }
    }
    /// Spur, deren Oberkante als letzte bei oder über `content_y` liegt.
    fn row_index(&self, content_y: f32) -> usize {
        self.tops
            .partition_point(|&t| t <= content_y)
            .saturating_sub(1)
    }
    fn row_at(&self, y: f32) -> Option<usize> {
        let c = y - self.lanes.top() + self.view.scroll_y;
        let total = self.tops.last().copied().unwrap_or(0.0);
        (c >= 0.0 && c < total).then(|| self.row_index(c))
    }
    fn visible_rows(&self) -> std::ops::Range<usize> {
        let first = self.row_index(self.view.scroll_y.max(0.0));
        let last = self.row_index(self.view.scroll_y + self.lanes.height()) + 1;
        first.min(self.rows)..last.min(self.rows)
    }
    /// Rechtecke der Knöpfe M, S, R im Spurkopf.
    /// Schalter im Spurkopf, von links: Mute, Solo, Aufnahme bereit, Monitor,
    /// Automation lesen, Automation schreiben, Zeitbasis, Lanes
    /// (Spec „Spurkopf in der Spurliste“).
    fn header_buttons(&self, row: usize) -> [Rect; HEADER_BUTTONS] {
        let top = self.row_top(row) + (self.view.row_h - 18.0) / 2.0;
        let right = self.header.right() - 6.0;
        std::array::from_fn(|i| {
            let x = right - (HEADER_BUTTONS - i) as f32 * 21.0;
            Rect::from_min_size(pos2(x, top), vec2(19.0, 18.0))
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
    w.lanes_rect = lanes;

    let tops = row_tops(w);
    scroll_and_zoom(w, ui, &resp, lanes, tops.last().copied().unwrap_or(0.0));
    follow_cursor(w, lanes);
    let geo = Geo {
        header,
        ruler,
        lanes,
        view: w.view,
        rows,
        tops,
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
        let before = w.project.clone();
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
        w.checkpoint(before);
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
    content_h: f32,
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
    let max_y = (content_h - lanes.height() + w.view.row_h).max(0.0);
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
    if resp.clicked() || resp.drag_started() {
        w.key_editor.focused = false;
    }
    // Doppelklick auf einen Part öffnet den Key-Editor.
    if resp.double_clicked()
        && let Some(p) = resp.interact_pointer_pos()
        && g.lanes.contains(p)
        && let Some(row) = g.row_at(p.y)
        && matches!(
            w.project.tracks[row].kind,
            TrackKind::Instrument | TrackKind::Midi
        )
        && let Some(id) = hit_event(w, g, row, p.y, g.ticks(p.x).max(0.0) as u64)
    {
        w.open_key_editor(id);
        return;
    }

    if resp.drag_started()
        && let Some(p) = press
    {
        w.drag_before = Some(w.project.clone());
        w.drag = start_drag(w, g, p, mods.alt);
    }
    if resp.dragged()
        && let (Some(d), Some(p)) = (w.drag, now)
    {
        live_drag(w, g, d, p, mods);
    }
    if resp.drag_stopped() {
        if let (Some(d), Some(p)) = (w.drag.take(), now) {
            finish_drag(w, g, d, p, mods);
        }
        if let Some(before) = w.drag_before.take() {
            w.checkpoint(before);
        }
        return;
    }
    if resp.clicked()
        && let Some(p) = resp.interact_pointer_pos()
    {
        let before = w.project.clone();
        click(w, g, p, mods);
        w.checkpoint(before);
    }

    // Mauszeiger zeigt den Smart-Bereich an.
    if w.tool == Tool::Select
        && let Some(p) = resp.hover_pos()
    {
        let zone = match w.drag {
            Some(Drag::TrimStart { .. } | Drag::TrimEnd { .. }) => Some(Zone::TrimStart),
            Some(Drag::FadeIn { .. } | Drag::FadeOut { .. }) => Some(Zone::FadeIn),
            Some(_) => None,
            None => smart_zone(w, g, p).map(|(_, _, z)| z),
        };
        match zone {
            Some(Zone::TrimStart | Zone::TrimEnd) => {
                ui.ctx().set_cursor_icon(egui::CursorIcon::ResizeHorizontal)
            }
            Some(Zone::FadeIn | Zone::FadeOut) => ui.ctx().set_cursor_icon(egui::CursorIcon::Grab),
            _ => {}
        }
    }
}

/// Änderungen, die schon während des Ziehens sichtbar und hörbar sind.
fn live_drag(w: &mut ProjectWindow, g: &Geo, d: Drag, p: Pos2, mods: egui::Modifiers) {
    let raw = g.ticks(p.x).max(0.0);
    let snapped = if mods.alt {
        raw.round() as u64
    } else {
        w.snap_ticks(raw)
    };
    match d {
        Drag::Ruler { start } => {
            let t = w.snap_ticks(raw);
            w.left_locator = start.min(t);
            w.right_locator = start.max(t);
            return;
        }
        Drag::TrimStart { id } => {
            w.project.trim_start(id, snapped);
        }
        Drag::TrimEnd { id } => {
            w.project.trim_end(id, snapped);
        }
        Drag::FadeIn { id } => {
            if let Some(ev) = w.project.event(id) {
                let len = (raw as u64).saturating_sub(ev.start.0);
                let out = ev.fade_out;
                w.project.set_fades(id, len, out);
            }
        }
        Drag::FadeOut { id } => {
            if let Some(ev) = w.project.event(id) {
                let len = ev.end().saturating_sub(raw as u64);
                let fin = ev.fade_in;
                w.project.set_fades(id, fin, len.min(ev.length - fin));
            }
        }
        Drag::Range { row, start } => {
            let r = g
                .row_at(p.y.clamp(g.lanes.top(), g.lanes.bottom() - 1.0))
                .unwrap_or(row);
            w.range = Some((
                row.min(r),
                row.max(r),
                start.min(snapped),
                start.max(snapped),
            ));
            return;
        }
        Drag::AutoPoint { .. } | Drag::AutoDraw { .. } => {
            w.drag = Some(automation::live_drag(w, g, d, p, mods.alt));
            return;
        }
        Drag::Move { .. } | Drag::Draw { .. } | Drag::Comp { .. } => return,
    }
    w.dirty = true;
}

fn start_drag(w: &mut ProjectWindow, g: &Geo, p: Pos2, alt: bool) -> Option<Drag> {
    if g.ruler.contains(p) {
        return Some(Drag::Ruler {
            start: w.snap_ticks(g.ticks(p.x)),
        });
    }
    if !g.lanes.contains(p) {
        return None;
    }
    if let Some(d) = automation::start_drag(w, g, p, alt) {
        return d;
    }
    let row = g.row_at(p.y)?;
    let ticks = g.ticks(p.x);
    match w.tool {
        Tool::Select => {
            let (id, row, zone) = smart_zone(w, g, p)?;
            if !w.selection.contains(&id) {
                w.selection = vec![id];
            }
            w.selected_track = Some(row);
            let start = w.project.event(id)?.start.0 as f64;
            Some(match zone {
                Zone::Body => Drag::Move {
                    id,
                    grab: ticks - start,
                    row,
                },
                Zone::TrimStart => Drag::TrimStart { id },
                Zone::TrimEnd => Drag::TrimEnd { id },
                Zone::FadeIn => Drag::FadeIn { id },
                Zone::FadeOut => Drag::FadeOut { id },
            })
        }
        Tool::Range => {
            let t = w.snap_ticks(ticks);
            w.range = Some((row, row, t, t));
            Some(Drag::Range { row, start: t })
        }
        Tool::Draw if w.project.tracks[row].kind.holds_events() => Some(Drag::Draw {
            row,
            start: w.snap_ticks(ticks),
        }),
        Tool::Comp => {
            let id = hit_event(w, g, row, p.y, ticks.max(0.0) as u64)?;
            w.selected_track = Some(row);
            Some(Drag::Comp {
                row,
                id,
                start: w.snap_ticks(ticks),
            })
        }
        _ => None,
    }
}

/// Comping: Abschnitt `a..b` (Klick: das ganze Take) hörbar machen und auswählen.
fn comp(w: &mut ProjectWindow, id: EventId, a: u64, b: u64) {
    let (a, b) = if a == b {
        match w.project.event(id) {
            Some(e) => (e.start.0, e.end()),
            None => return,
        }
    } else {
        (a.min(b), a.max(b))
    };
    if let Some(mid) = w.project.comp(id, a, b) {
        w.selection = vec![mid];
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
        w.snap_move(raw, ev.length, ev.start.0)
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
        // Schon während des Ziehens angewendet.
        Drag::Range { .. } => {
            if w.range.is_some_and(|(_, _, a, b)| b <= a) {
                w.range = None;
            }
        }
        Drag::TrimStart { .. }
        | Drag::TrimEnd { .. }
        | Drag::FadeIn { .. }
        | Drag::FadeOut { .. }
        | Drag::AutoPoint { .. }
        | Drag::AutoDraw { .. } => {}
        Drag::Comp { id, start, .. } => {
            let end = w.snap_ticks(g.ticks(p.x));
            comp(w, id, start, end);
        }
        Drag::Draw { row, start } => {
            let end = w.snap_ticks(g.ticks(p.x));
            let min = w.grid_ticks();
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
            // Snap-Modus „Schieben“: ein Event innerhalb seiner Spur, die Nachbarn rücken nach.
            if w.snap
                && w.snap_mode == crate::tools::SnapMode::Shuffle
                && !mods.alt
                && !copy
                && drow == 0
                && let [only] = w.selection[..]
                && let Some(ev) = w.project.event(only)
            {
                let start = (ev.start.0 as i64 + dt).max(0) as u64;
                w.project.shuffle_event(only, start);
                return;
            }
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
        if automation::header_click(w, g, p) {
            return;
        }
        // Farbleiste ganz links: Palette der Spurfarbe.
        if let Some(row) = g.row_at(p.y)
            && p.x < g.header.left() + 7.0
        {
            w.open_color_popup(crate::colors::Target::Track(row), p);
            return;
        }
        if let Some(row) = g.row_at(p.y)
            && automation::arrow_rect(g, row).expand(3.0).contains(p)
        {
            automation::toggle(&mut w.project.tracks[row]);
            return;
        }
        if let Some(row) = g.row_at(p.y) {
            let [m, s, r, mon, read, write, base, lanes] = g.header_buttons(row);
            let track = &mut w.project.tracks[row];
            let kind = track.kind;
            if m.contains(p) {
                track.mute = !track.mute;
            } else if s.contains(p) {
                track.solo = !track.solo;
            } else if r.contains(p) && kind.holds_events() {
                track.record = !track.record;
            } else if mon.contains(p) && kind == TrackKind::Audio {
                let on = !track.monitor;
                w.set_monitor(row, on);
            } else if read.contains(p) && crate::mixer::has_channel(kind) {
                track.read = !track.read;
            } else if write.contains(p) && crate::mixer::has_channel(kind) {
                track.write = !track.write;
            } else if base.contains(p) && kind.holds_events() {
                track.time_base = Some(match track.time_base() {
                    TimeBase::Musical => TimeBase::Linear,
                    TimeBase::Linear => TimeBase::Musical,
                });
            } else if lanes.contains(p) && kind.holds_events() {
                track.show_lanes = !track.show_lanes;
            } else {
                w.selected_track = Some(row);
            }
        }
        return;
    }
    if !g.lanes.contains(p) {
        return;
    }
    if automation::click(w, g, p, mods.alt) {
        return;
    }
    let Some(row) = g.row_at(p.y) else { return };
    let ticks = g.ticks(p.x).max(0.0);
    let hit = hit_event(w, g, row, p.y, ticks as u64);
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
        (Tool::Select, None) => {
            w.selection.clear();
            w.range = None;
        }
        (Tool::Range, _) => w.range = None,
        (Tool::Comp, Some(id)) => comp(w, id, 0, 0),
        (Tool::Glue, Some(id)) => {
            // Alt: alle folgenden Events dieser Spur ankleben.
            let mut result = w.project.glue(id);
            while mods.alt && result.is_ok() {
                result = w.project.glue(id);
            }
            match result {
                Ok(_) => w.selection = vec![id],
                Err(e) if !mods.alt => w.message = Some(e.to_owned()),
                Err(_) => {}
            }
        }
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
    // Feines Snap-Raster (z. B. 1/16, Frames, „An Zoom anpassen“) zwischen den Zählzeiten.
    if !labels {
        let fine = Stroke::new(1.0, weak.color.gamma_multiply(0.75));
        let lines = w.fine_grid_lines(t0 as f64, t1 as f64, w.view.px_per_quarter, beat_px >= 10.0);
        for t in lines {
            painter.vline(g.x(t), rect.y_range(), fine);
        }
    }
}

fn paint_lanes(w: &ProjectWindow, painter: &Painter, g: &Geo) {
    let visuals = painter.ctx().global_style().visuals.clone();
    painter.rect_filled(g.lanes, 0.0, visuals.extreme_bg_color);
    for row in g.visible_rows() {
        let top = g.row_top(row);
        let rect = Rect::from_min_size(
            pos2(g.lanes.left(), top),
            vec2(g.lanes.width(), g.row_height(row)),
        );
        if row % 2 == 1 {
            painter.rect_filled(rect, 0.0, visuals.faint_bg_color.gamma_multiply(0.5));
        }
        // Aufgeklappte Lanes: dünne Linien zwischen den Lanes.
        let lanes = (g.row_height(row) / g.view.row_h).round() as usize;
        for k in 1..lanes {
            painter.hline(
                rect.x_range(),
                top + k as f32 * g.view.row_h,
                Stroke::new(1.0, visuals.faint_bg_color.gamma_multiply(0.6)),
            );
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
        painter.rect_filled(
            r,
            0.0,
            crate::theme::tokens(painter.ctx())
                .cycle_fill
                .gamma_multiply(0.5),
        );
    }

    let t0 = g.ticks(g.lanes.left()).max(0.0) as u64;
    let t1 = g.ticks(g.lanes.right()).max(0.0) as u64;
    for row in g.visible_rows() {
        let track = &w.project.tracks[row];
        let h = g.view.row_h - 4.0;
        let lanes = track.show_lanes.then(|| track.lanes().0);
        // Aufgeklappt: nicht hörbare Abschnitte abdunkeln (Comping).
        let audible = track.show_lanes.then(|| track.audible_parts());
        // Zugeklappt: das klingende Event (höchster Comp-Rang) oben zeichnen.
        let mut order: Vec<usize> = (0..track.events.len()).collect();
        if track.events.iter().any(|e| e.comp > 0) {
            order.sort_by_key(|&i| (track.events[i].comp, i));
        }
        for i in order {
            let ev = &track.events[i];
            if ev.end() < t0 || ev.start.0 > t1 {
                continue;
            }
            let lane = lanes.as_ref().and_then(|l| l.get(i).copied()).unwrap_or(0);
            let top = g.row_top(row) + lane as f32 * g.view.row_h + 2.0;
            let rect = Rect::from_min_max(
                pos2(g.x(ev.start.0 as f64), top),
                pos2(g.x(ev.end() as f64), top + h),
            );
            paint_event(w, painter, g, track, ev, rect);
            if let Some(parts) = audible.as_ref().and_then(|a| a.get(i)) {
                let mut from = ev.start.0;
                let shade = Color32::from_black_alpha(120);
                for &(a, b) in parts.iter().chain(std::iter::once(&(ev.end(), ev.end()))) {
                    if a > from {
                        let r =
                            Rect::from_x_y_ranges(g.x(from as f64)..=g.x(a as f64), rect.y_range());
                        painter.rect_filled(r.intersect(rect), 3.0, shade);
                    }
                    from = from.max(b);
                }
            }
        }
    }
    automation::paint(w, painter, g);
    paint_recording(w, painter, g);
    if let Some((r0, r1, a, b)) = w.range {
        let rect = Rect::from_min_max(
            pos2(g.x(a as f64), g.row_top(r0)),
            pos2(
                g.x(b as f64).max(g.x(a as f64) + 1.0),
                g.row_top(r1) + g.row_height(r1),
            ),
        );
        painter.rect_filled(rect, 0.0, crate::theme::tokens(painter.ctx()).selection);
        painter.rect_stroke(
            rect,
            0.0,
            Stroke::new(1.0, crate::theme::tokens(painter.ctx()).accent),
            StrokeKind::Inside,
        );
    }
    paint_drag_preview(w, painter, g);
}

/// Während der Aufnahme wächst auf jeder aufnahmebereiten Spur ein roter Bereich mit.
fn paint_recording(w: &ProjectWindow, painter: &Painter, g: &Geo) {
    let Some(t) = w.transport.as_ref().filter(|_| w.recording) else {
        return;
    };
    let start = t.position(t.status.rec_start()).0;
    let from = if w.cursor >= start {
        start
    } else {
        w.left_locator
    };
    for row in g.visible_rows() {
        let tr = &w.project.tracks[row];
        if !(tr.record && tr.kind == TrackKind::Audio) {
            continue;
        }
        let top = g.row_top(row) + 2.0;
        let rect = Rect::from_min_max(
            pos2(g.x(from as f64), top),
            pos2(
                g.x(w.cursor as f64).max(g.x(from as f64) + 1.0),
                top + g.view.row_h - 4.0,
            ),
        );
        painter.rect_filled(
            rect,
            3.0,
            crate::theme::tokens(painter.ctx()).rec.gamma_multiply(0.55),
        );
        painter.rect_stroke(
            rect,
            3.0,
            Stroke::new(1.0, crate::theme::tokens(painter.ctx()).rec),
            StrokeKind::Inside,
        );
    }
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
        crate::theme::tokens(painter.ctx()).event_muted
    } else {
        track_color(ev.color_on(track))
    };
    let body_fill = if ev.muted {
        crate::theme::tokens(painter.ctx())
            .event_muted
            .lerp_to_gamma(crate::theme::tokens(painter.ctx()).event_shade, 0.6)
    } else {
        color.lerp_to_gamma(crate::theme::tokens(painter.ctx()).event_shade, 0.62)
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
    if !ev.notes.is_empty() {
        paint_notes(
            painter,
            g,
            ev,
            body.intersect(g.lanes),
            color.lerp_to_gamma(Color32::WHITE, 0.45),
        );
    }
    if let Some(clip) = ev.clip.and_then(|c| w.clip(c)) {
        let wave = if ev.muted {
            crate::theme::tokens(painter.ctx())
                .event_text_muted
                .gamma_multiply(0.7)
        } else {
            color.lerp_to_gamma(Color32::WHITE, 0.35)
        };
        paint_waveform(w, painter, g, ev, clip, body.intersect(g.lanes), wave);
    }
    let selected = w.selection.contains(&ev.id);
    paint_fades(painter, g, ev, body, selected);

    let stroke = if selected {
        Stroke::new(2.0, crate::theme::tokens(painter.ctx()).event_selected)
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
                if ev.muted {
                    crate::theme::tokens(painter.ctx()).event_text_muted
                } else {
                    crate::theme::tokens(painter.ctx()).event_text
                },
            );
    }
}

/// Mini-Noten in einem MIDI-Part: Tonhöhenbereich der Noten auf die Höhe verteilt.
fn paint_notes(painter: &Painter, g: &Geo, ev: &Event, area: Rect, color: Color32) {
    if area.height() < 4.0 || area.width() < 1.0 {
        return;
    }
    let lo = ev.notes.iter().map(|n| n.pitch).min().unwrap_or(60).min(84) as f32;
    let hi = ev
        .notes
        .iter()
        .map(|n| n.pitch)
        .max()
        .unwrap_or(60)
        .max(lo as u8 + 12) as f32;
    let step = (area.height() - 2.0) / (hi - lo + 1.0);
    let len = ev.length as i64;
    for n in ev.notes.iter().filter(|n| n.start < len && n.end() > 0) {
        let a = ev.start.0 as f64 + n.start.max(0) as f64;
        let b = ev.start.0 as f64 + n.end().min(len) as f64;
        let y = area.bottom() - 1.0 - (f32::from(n.pitch) - lo + 1.0) * step;
        let r = Rect::from_min_max(
            pos2(g.x(a), y),
            pos2(g.x(b).max(g.x(a) + 2.0), y + step.clamp(1.5, 4.0)),
        );
        painter.rect_filled(r.intersect(area), 0.5, color);
    }
}

/// Linearer Fade-Faktor an Tick `k` relativ zum Event-Anfang (wie in der Engine).
fn fade_gain(ev: &Event, k: f64) -> f32 {
    let mut g = 1.0f32;
    if (ev.fade_in as f64) > 0.0 && k < ev.fade_in as f64 {
        g = (k / ev.fade_in as f64).max(0.0) as f32;
    }
    let rest = ev.length as f64 - k;
    if (ev.fade_out as f64) > 0.0 && rest < ev.fade_out as f64 {
        g *= (rest / ev.fade_out as f64).max(0.0) as f32;
    }
    g
}

/// Fade-Bereiche abdunkeln, Kurve zeichnen, Griffe am ausgewählten Event.
fn paint_fades(painter: &Painter, g: &Geo, ev: &Event, body: Rect, selected: bool) {
    let shade = Color32::from_black_alpha(120);
    let line = Stroke::new(1.2, Color32::from_white_alpha(200));
    let (top, bottom) = (body.top(), body.bottom());
    let fin = g.x((ev.start.0 + ev.fade_in) as f64);
    let fout = g.x((ev.end() - ev.fade_out) as f64);
    if ev.fade_in > 0 {
        let pts = vec![
            pos2(body.left(), top),
            pos2(fin, top),
            pos2(body.left(), bottom),
        ];
        painter.add(egui::Shape::convex_polygon(pts, shade, Stroke::NONE));
        painter.line_segment([pos2(body.left(), bottom), pos2(fin, top)], line);
    }
    if ev.fade_out > 0 {
        let pts = vec![
            pos2(fout, top),
            pos2(body.right(), top),
            pos2(body.right(), bottom),
        ];
        painter.add(egui::Shape::convex_polygon(pts, shade, Stroke::NONE));
        painter.line_segment([pos2(fout, top), pos2(body.right(), bottom)], line);
    }
    if selected {
        for x in [fin, fout] {
            let handle = Rect::from_center_size(pos2(x, top + 1.0), egui::vec2(7.0, 7.0));
            painter.rect_filled(handle, 1.0, crate::theme::tokens(painter.ctx()).handle);
            painter.rect_stroke(
                handle,
                1.0,
                Stroke::new(1.0, crate::theme::tokens(painter.ctx()).border),
                StrokeKind::Inside,
            );
        }
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
        let fade = fade_gain(ev, g.ticks(x + 0.5) - ev.start.0 as f64);
        let (lo, hi) = (lo * fade, hi * fade);
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
        Some(Drag::Comp { row, id, start }) => {
            let end = w.snap_ticks(g.ticks(p.x));
            let lane = w.project.tracks.get(row).map_or(0, |t| lane_of(t, id));
            let top = g.row_top(row) + lane as f32 * g.view.row_h + 2.0;
            let rect = Rect::from_x_y_ranges(
                g.x(start.min(end) as f64)..=g.x(start.max(end) as f64),
                top..=top + g.view.row_h - 4.0,
            );
            let accent = crate::theme::tokens(painter.ctx()).accent;
            painter.rect_stroke(rect, 3.0, Stroke::new(2.0, accent), StrokeKind::Inside);
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
        crate::theme::tokens(painter.ctx()).accent
    } else {
        crate::theme::tokens(painter.ctx()).ruler_tick
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
            vec2(g.header.width(), g.row_height(row)),
        );
        // Name und Schalter stehen in der obersten Lane.
        let head = Rect::from_min_size(rect.min, vec2(rect.width(), g.view.row_h));
        // Platz für den Automationspfeil vor dem Namen.
        let name_x = if crate::mixer::has_channel(track.kind) {
            24.0
        } else {
            12.0
        };
        if w.selected_track == Some(row) {
            // Gewählte Spur: heller hinterlegt, nicht vollflächig in Akzentfarbe (ui-parity.md).
            painter.rect_filled(rect, 0.0, crate::theme::tokens(painter.ctx()).row_selected);
        }
        painter.rect_filled(
            Rect::from_min_size(rect.min, vec2(5.0, rect.height())),
            0.0,
            track_color(track.color),
        );
        // Name nur bis vor die Knöpfe, damit lange Namen nicht darüber laufen.
        let name_clip = Rect::from_min_max(
            rect.min,
            pos2(g.header_buttons(row)[0].left() - 4.0, rect.max.y),
        );
        painter.with_clip_rect(name_clip.intersect(g.header)).text(
            head.left_center() + vec2(name_x, -6.0),
            Align2::LEFT_CENTER,
            &track.name,
            FontId::proportional(13.0),
            crate::theme::tokens(painter.ctx()).text,
        );
        painter.text(
            head.left_center() + vec2(name_x, 8.0),
            Align2::LEFT_CENTER,
            track.kind.label(),
            FontId::proportional(10.0),
            visuals.weak_text_color(),
        );
        let t = crate::theme::tokens(painter.ctx());
        let channel = crate::mixer::has_channel(track.kind);
        let flags = [
            (Flag::Text("M"), track.mute, t.mute, true),
            (Flag::Text("S"), track.solo, t.solo, true),
            (
                Flag::Icon(Icon::Record),
                track.record,
                t.rec,
                track.kind.holds_events(),
            ),
            (
                Flag::Icon(Icon::Play),
                track.monitor,
                t.monitor,
                track.kind == TrackKind::Audio,
            ),
            (Flag::Text("R"), track.read, t.monitor, channel),
            (Flag::Text("W"), track.write, t.rec, channel),
            (
                Flag::Icon(match track.time_base() {
                    TimeBase::Musical => Icon::TimeMusical,
                    TimeBase::Linear => Icon::TimeLinear,
                }),
                false,
                t.accent,
                track.kind.holds_events(),
            ),
            (
                Flag::Icon(Icon::Lanes),
                track.show_lanes,
                t.accent,
                track.kind.holds_events(),
            ),
        ];
        for (b, (flag, on, color, shown)) in g.header_buttons(row).into_iter().zip(flags) {
            if !shown {
                continue;
            }
            let fill = if on {
                color
            } else {
                visuals.widgets.inactive.bg_fill
            };
            painter.rect_filled(b, 3.0, fill);
            match flag {
                Flag::Icon(icon) => {
                    // Aufnahme bereit (Punkt) und Monitor (Lautsprecher) als Symbol.
                    let fg = if on {
                        Color32::WHITE
                    } else if icon == Icon::Record {
                        color
                    } else {
                        visuals.text_color()
                    };
                    crate::icons::paint(painter, b.shrink(4.0), icon, fg);
                }
                Flag::Text(label) => {
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
            }
        }
        automation::paint_headers(w, painter, g, row);
        painter.hline(
            rect.x_range(),
            rect.bottom(),
            Stroke::new(1.0, visuals.faint_bg_color),
        );
    }
}

/// Beschriftung eines Spurkopf-Schalters.
enum Flag {
    Text(&'static str),
    Icon(Icon),
}

fn paint_cursor(w: &ProjectWindow, painter: &Painter, g: &Geo) {
    let x = g.x(w.cursor as f64);
    let y = g.ruler.top()..=g.lanes.bottom();
    painter.vline(
        x,
        y,
        Stroke::new(1.5, crate::theme::tokens(painter.ctx()).playhead),
    );
}
