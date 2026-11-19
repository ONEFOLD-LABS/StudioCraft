//! StudioCraft's own toolbar symbols, drawn with egui shapes (no image files, no foreign icons).
//!
//! Every symbol is laid out on a 16 × 16 grid and scaled into the button, so it stays sharp at
//! any UI scale and takes its colour from the theme. [`icon_button`] wraps a symbol in a square,
//! accessible button (`docs/ui-parity.md`, section „Werkzeugzeile“).

use egui::{
    Color32, CornerRadius, Painter, Pos2, Rect, Response, Role, Sense, Shape, Stroke, StrokeKind,
    Ui, Vec2, WidgetInfo, vec2,
};

/// One symbol.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Icon {
    // tools 1–0
    Select,
    Range,
    Split,
    Glue,
    Erase,
    Zoom,
    Mute,
    Draw,
    Play,
    TimeWarp,
    // snap
    Snap,
    /// Quantisierungs-Panel: Schieberegler.
    QuantizePanel,
    // track header
    /// Zeitbasis musikalisch: Achtelnote.
    TimeMusical,
    /// Zeitbasis linear: Uhr.
    TimeLinear,
    /// Lanes ein/aus: drei gestapelte Balken.
    Lanes,
    // zones
    ZoneLeft,
    ZoneLower,
    ZoneRight,
    // transport
    ToStart,
    Rewind,
    Forward,
    Stop,
    Start,
    Record,
    Cycle,
    Metronome,
    PunchIn,
    PunchOut,
}

impl Icon {
    pub const ALL: [Icon; 28] = [
        Icon::Select,
        Icon::Range,
        Icon::Split,
        Icon::Glue,
        Icon::Erase,
        Icon::Zoom,
        Icon::Mute,
        Icon::Draw,
        Icon::Play,
        Icon::TimeWarp,
        Icon::Snap,
        Icon::QuantizePanel,
        Icon::TimeMusical,
        Icon::TimeLinear,
        Icon::Lanes,
        Icon::ZoneLeft,
        Icon::ZoneLower,
        Icon::ZoneRight,
        Icon::ToStart,
        Icon::Rewind,
        Icon::Forward,
        Icon::Stop,
        Icon::Start,
        Icon::Record,
        Icon::Cycle,
        Icon::Metronome,
        Icon::PunchIn,
        Icon::PunchOut,
    ];
}

/// Maps the 16 × 16 design grid into `rect`.
struct Grid {
    origin: Pos2,
    unit: f32,
}

impl Grid {
    fn new(rect: Rect) -> Self {
        let side = rect.width().min(rect.height());
        let unit = side / 16.0;
        let origin = rect.center() - Vec2::splat(side / 2.0);
        Self { origin, unit }
    }

    fn p(&self, x: f32, y: f32) -> Pos2 {
        self.origin + vec2(x, y) * self.unit
    }

    fn r(&self, x0: f32, y0: f32, x1: f32, y1: f32) -> Rect {
        Rect::from_two_pos(self.p(x0, y0), self.p(x1, y1))
    }

    fn len(&self, v: f32) -> f32 {
        v * self.unit
    }
}

/// Draws `icon` into `rect` in `color`.
pub fn paint(painter: &Painter, rect: Rect, icon: Icon, color: Color32) {
    painter.extend(shapes(rect, icon, color));
}

/// The shapes of `icon` in `rect`.
pub fn shapes(rect: Rect, icon: Icon, color: Color32) -> Vec<Shape> {
    let mut out = Vec::new();
    let g = Grid::new(rect);
    let w = g.len(1.4).max(1.0);
    let s = Stroke::new(w, color);
    let fill = |pts: Vec<Pos2>| Shape::convex_polygon(pts, color, Stroke::NONE);
    let line = |pts: Vec<Pos2>| Shape::line(pts, s);
    match icon {
        Icon::Select => {
            out.push(fill(vec![
                g.p(4.0, 2.0),
                g.p(12.0, 9.5),
                g.p(8.4, 10.0),
                g.p(4.0, 14.0),
            ]));
            out.push(fill(vec![
                g.p(7.4, 9.6),
                g.p(9.2, 9.0),
                g.p(11.4, 13.6),
                g.p(9.6, 14.4),
            ]));
        }
        Icon::Range => {
            let dash = Stroke::new(g.len(1.0).max(1.0), color);
            out.extend(Shape::dashed_line(
                &[
                    g.p(2.0, 3.0),
                    g.p(14.0, 3.0),
                    g.p(14.0, 13.0),
                    g.p(2.0, 13.0),
                    g.p(2.0, 3.0),
                ],
                dash,
                g.len(2.0),
                g.len(1.5),
            ));
            out.push(Shape::rect_filled(
                g.r(5.0, 6.0, 11.0, 10.0),
                0.0,
                color.gamma_multiply(0.6),
            ));
        }
        Icon::Split => {
            // two finger loops and crossed blades
            out.push(Shape::circle_stroke(g.p(4.5, 12.0), g.len(2.2), s));
            out.push(Shape::circle_stroke(g.p(11.5, 12.0), g.len(2.2), s));
            out.push(line(vec![g.p(6.0, 10.4), g.p(12.0, 2.0)]));
            out.push(line(vec![g.p(10.0, 10.4), g.p(4.0, 2.0)]));
        }
        Icon::Glue => {
            // tube with a drop
            out.push(fill(vec![
                g.p(3.0, 4.0),
                g.p(10.0, 4.0),
                g.p(10.0, 9.0),
                g.p(3.0, 9.0),
            ]));
            out.push(fill(vec![
                g.p(10.0, 5.0),
                g.p(13.0, 6.0),
                g.p(13.0, 7.0),
                g.p(10.0, 8.0),
            ]));
            out.push(Shape::circle_filled(g.p(13.0, 12.0), g.len(1.6), color));
            out.push(fill(vec![g.p(12.0, 11.0), g.p(13.0, 8.5), g.p(14.0, 11.0)]));
        }
        Icon::Erase => {
            out.push(fill(vec![
                g.p(2.0, 10.0),
                g.p(9.0, 3.0),
                g.p(14.0, 8.0),
                g.p(7.0, 15.0),
            ]));
            out.push(Shape::line(
                vec![g.p(5.5, 6.5), g.p(10.5, 11.5)],
                Stroke::new(w, Color32::from_black_alpha(140)),
            ));
        }
        Icon::Zoom => {
            out.push(Shape::circle_stroke(g.p(6.5, 6.5), g.len(4.2), s));
            out.push(Shape::line(
                vec![g.p(9.6, 9.6), g.p(14.0, 14.0)],
                Stroke::new(g.len(2.2), color),
            ));
        }
        Icon::Mute => {
            let thick = Stroke::new(g.len(2.0), color);
            out.push(Shape::line(vec![g.p(3.0, 3.0), g.p(13.0, 13.0)], thick));
            out.push(Shape::line(vec![g.p(13.0, 3.0), g.p(3.0, 13.0)], thick));
        }
        Icon::Draw => {
            out.push(fill(vec![
                g.p(10.5, 2.5),
                g.p(13.5, 5.5),
                g.p(5.5, 13.5),
                g.p(2.5, 10.5),
            ]));
            out.push(fill(vec![g.p(2.5, 10.5), g.p(5.5, 13.5), g.p(1.5, 14.5)]));
        }
        Icon::Play => {
            // loudspeaker with sound waves
            out.push(fill(vec![
                g.p(2.0, 6.0),
                g.p(5.0, 6.0),
                g.p(9.0, 2.5),
                g.p(9.0, 13.5),
                g.p(5.0, 10.0),
                g.p(2.0, 10.0),
            ]));
            out.push(line(vec![g.p(11.0, 5.5), g.p(12.0, 8.0), g.p(11.0, 10.5)]));
            out.push(line(vec![g.p(13.0, 3.5), g.p(14.5, 8.0), g.p(13.0, 12.5)]));
        }
        Icon::TimeWarp => {
            out.push(Shape::circle_stroke(g.p(8.0, 8.0), g.len(6.0), s));
            out.push(line(vec![g.p(8.0, 4.0), g.p(8.0, 8.0), g.p(11.0, 10.0)]));
        }
        Icon::Snap => {
            // magnet: a thick U with light pole tips
            let thick = Stroke::new(g.len(3.0), color);
            let arc: Vec<Pos2> = (0..=12)
                .map(|i| {
                    let a = std::f32::consts::PI * i as f32 / 12.0;
                    g.p(8.0 - 4.5 * a.cos(), 8.0 + 4.5 * a.sin())
                })
                .collect();
            let mut u = vec![g.p(3.5, 2.0)];
            u.extend(arc);
            u.push(g.p(12.5, 2.0));
            out.push(Shape::line(u, thick));
            let tip = color.gamma_multiply(0.45);
            out.push(Shape::rect_filled(g.r(2.0, 1.0, 5.0, 4.0), 0.0, tip));
            out.push(Shape::rect_filled(g.r(11.0, 1.0, 14.0, 4.0), 0.0, tip));
        }
        Icon::ZoneLeft | Icon::ZoneLower | Icon::ZoneRight => {
            let frame = g.r(1.5, 2.5, 14.5, 13.5);
            out.push(Shape::rect_stroke(
                frame,
                CornerRadius::same(1),
                s,
                StrokeKind::Inside,
            ));
            let part = match icon {
                Icon::ZoneLeft => g.r(1.5, 2.5, 6.0, 13.5),
                Icon::ZoneLower => g.r(1.5, 9.0, 14.5, 13.5),
                _ => g.r(10.0, 2.5, 14.5, 13.5),
            };
            out.push(Shape::rect_filled(part, CornerRadius::same(1), color));
        }
        Icon::ToStart => {
            out.push(Shape::rect_filled(g.r(3.0, 3.0, 5.0, 13.0), 0.0, color));
            out.push(fill(vec![g.p(13.0, 3.0), g.p(13.0, 13.0), g.p(5.5, 8.0)]));
        }
        Icon::Rewind => {
            out.push(fill(vec![g.p(8.0, 3.0), g.p(8.0, 13.0), g.p(1.0, 8.0)]));
            out.push(fill(vec![g.p(15.0, 3.0), g.p(15.0, 13.0), g.p(8.0, 8.0)]));
        }
        Icon::Forward => {
            out.push(fill(vec![g.p(1.0, 3.0), g.p(8.0, 8.0), g.p(1.0, 13.0)]));
            out.push(fill(vec![g.p(8.0, 3.0), g.p(15.0, 8.0), g.p(8.0, 13.0)]));
        }
        Icon::Stop => {
            out.push(Shape::rect_filled(
                g.r(3.5, 3.5, 12.5, 12.5),
                CornerRadius::same(1),
                color,
            ));
        }
        Icon::Start => {
            out.push(fill(vec![g.p(4.0, 2.5), g.p(13.5, 8.0), g.p(4.0, 13.5)]));
        }
        Icon::Record => {
            out.push(Shape::circle_filled(g.p(8.0, 8.0), g.len(5.0), color));
        }
        Icon::Cycle => {
            // a loop of two arrows: right along the top, left along the bottom
            out.push(line(vec![g.p(2.0, 9.0), g.p(2.0, 5.0), g.p(11.0, 5.0)]));
            out.push(fill(vec![g.p(10.5, 2.0), g.p(14.5, 5.0), g.p(10.5, 8.0)]));
            out.push(line(vec![g.p(14.0, 7.0), g.p(14.0, 11.0), g.p(5.0, 11.0)]));
            out.push(fill(vec![g.p(5.5, 8.0), g.p(1.5, 11.0), g.p(5.5, 14.0)]));
        }
        Icon::Metronome => {
            out.push(Shape::convex_polygon(
                vec![
                    g.p(5.5, 2.0),
                    g.p(10.5, 2.0),
                    g.p(13.5, 14.0),
                    g.p(2.5, 14.0),
                ],
                Color32::TRANSPARENT,
                s,
            ));
            out.push(line(vec![g.p(8.0, 12.0), g.p(12.0, 4.0)]));
            out.push(Shape::rect_filled(g.r(3.0, 11.5, 13.0, 14.0), 0.0, color));
        }
        Icon::QuantizePanel => {
            // three sliders with their knobs at different positions
            for (y, x) in [(4.0, 10.0), (8.0, 5.0), (12.0, 8.5)] {
                out.push(line(vec![g.p(2.0, y), g.p(14.0, y)]));
                out.push(Shape::circle_filled(g.p(x, y), g.len(2.0), color));
            }
        }
        Icon::TimeMusical => {
            // eighth note: head, stem, flag
            out.push(Shape::convex_polygon(
                (0..12)
                    .map(|i| {
                        let a = std::f32::consts::TAU * i as f32 / 12.0;
                        g.p(6.0 + 2.6 * a.cos(), 12.0 + 1.9 * a.sin())
                    })
                    .collect(),
                color,
                Stroke::NONE,
            ));
            out.push(line(vec![g.p(8.4, 12.0), g.p(8.4, 2.5)]));
            out.push(line(vec![g.p(8.4, 2.5), g.p(12.0, 5.5), g.p(11.5, 8.0)]));
        }
        Icon::TimeLinear => {
            out.push(Shape::circle_stroke(g.p(8.0, 8.0), g.len(6.0), s));
            out.push(line(vec![g.p(8.0, 4.0), g.p(8.0, 8.0), g.p(11.0, 9.5)]));
        }
        Icon::Lanes => {
            for y in [3.0, 7.0, 11.0] {
                out.push(fill(vec![
                    g.p(2.0, y),
                    g.p(14.0, y),
                    g.p(14.0, y + 2.5),
                    g.p(2.0, y + 2.5),
                ]));
            }
        }
        Icon::PunchIn | Icon::PunchOut => {
            let x = if icon == Icon::PunchIn { 5.0 } else { 11.0 };
            out.push(line(vec![g.p(x, 2.0), g.p(x, 14.0)]));
            let tri = if icon == Icon::PunchIn {
                vec![g.p(7.0, 5.0), g.p(13.0, 8.0), g.p(7.0, 11.0)]
            } else {
                vec![g.p(3.0, 5.0), g.p(9.0, 8.0), g.p(3.0, 11.0)]
            };
            out.push(fill(tri));
        }
    }
    out
}

/// Side length of a toolbar button in points.
pub const BUTTON: f32 = 24.0;

/// A square symbol button. `selected` shows it pressed (accent background); `tint` overrides the
/// symbol colour when it is on (e.g. red for recording). `label` is the tooltip and the
/// accessible name.
pub fn icon_button(
    ui: &mut Ui,
    icon: Icon,
    selected: bool,
    tint: Option<Color32>,
    label: &str,
) -> Response {
    let t = crate::theme::tokens(ui.ctx());
    let (rect, response) = ui.allocate_exact_size(Vec2::splat(BUTTON), Sense::click());
    response.widget_info(|| WidgetInfo::selected(Role::Button, true, selected, label));
    if ui.is_rect_visible(rect) {
        let painter = ui.painter();
        let hovered = response.hovered();
        let bg = match (selected, hovered) {
            (true, _) if tint.is_some() => t.button_hi,
            (true, _) => t.accent_dark,
            (false, true) => t.button_hi,
            (false, false) => t.button,
        };
        painter.rect_filled(rect, CornerRadius::same(3), bg);
        if hovered {
            painter.rect_stroke(
                rect,
                CornerRadius::same(3),
                Stroke::new(1.0, t.border_light),
                StrokeKind::Inside,
            );
        }
        let fg = match (selected, tint) {
            (true, Some(c)) => c,
            (true, None) => t.text_on_accent,
            _ => t.text,
        };
        paint(painter, rect.shrink(4.0), icon, fg);
    }
    response.on_hover_text(label)
}

/// A small state switch (Mute, Solo, Aufnahme bereit) that fills with its status colour when on,
/// the same colours as in the track headers.
pub fn state_toggle(ui: &mut Ui, value: &mut bool, label: &str, color: Color32) -> Response {
    let t = crate::theme::tokens(ui.ctx());
    let text = egui::RichText::new(label).color(if *value { Color32::BLACK } else { t.text });
    let fill = if *value { color } else { t.button };
    let mut r = ui.add(
        egui::Button::new(text)
            .fill(fill)
            .min_size(vec2(20.0, 18.0)),
    );
    if r.clicked() {
        *value = !*value;
        r.mark_changed();
    }
    r
}

/// A thin vertical divider between toolbar groups.
pub fn group_gap(ui: &mut Ui) {
    ui.add_space(4.0);
    ui.separator();
    ui.add_space(4.0);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_icon_stays_inside_its_rect() {
        let rect = Rect::from_min_size(egui::pos2(10.0, 20.0), Vec2::splat(16.0));
        for icon in Icon::ALL {
            let shapes = shapes(rect, icon, Color32::WHITE);
            assert!(!shapes.is_empty(), "{icon:?} draws nothing");
            for shape in shapes {
                let b = shape.visual_bounding_rect();
                assert!(
                    rect.expand(1.5).contains_rect(b),
                    "{icon:?} leaves its rect: {b:?}"
                );
            }
        }
    }
}
