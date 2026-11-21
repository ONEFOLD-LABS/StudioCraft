//! Zeichenfläche des Notensatzes: Der Bildschirm (egui) und der PDF-Export zeichnen mit
//! derselben Logik. Koordinaten in Punkten, y nach unten.

use egui::{Align2, Color32, FontFamily, FontId, Painter, Pos2, Shape, Stroke, Vec2};

/// Was der Notensatz zum Zeichnen braucht.
pub(crate) trait Canvas {
    fn hline(&self, x: std::ops::RangeInclusive<f32>, y: f32, stroke: Stroke);
    fn vline(&self, x: f32, y: std::ops::RangeInclusive<f32>, stroke: Stroke);
    fn line_segment(&self, points: [Pos2; 2], stroke: Stroke);
    /// Gefüllte Vielecke (Balken) und Bézierkurven (Haltebögen).
    fn add(&self, shape: Shape);
    /// Text, unten links bei `pos` verankert.
    fn text(&self, pos: Pos2, anchor: Align2, text: String, font: FontId, color: Color32);
    /// Zeichen der Notenschrift mit der Grundlinie bei `at` (links); gibt die Breite zurück.
    fn glyph(&self, ch: char, at: Pos2, size: f32, color: Color32) -> f32;
}

impl Canvas for Painter {
    fn hline(&self, x: std::ops::RangeInclusive<f32>, y: f32, stroke: Stroke) {
        Painter::hline(self, x, y, stroke);
    }
    fn vline(&self, x: f32, y: std::ops::RangeInclusive<f32>, stroke: Stroke) {
        Painter::vline(self, x, y, stroke);
    }
    fn line_segment(&self, points: [Pos2; 2], stroke: Stroke) {
        Painter::line_segment(self, points, stroke);
    }
    fn add(&self, shape: Shape) {
        Painter::add(self, shape);
    }
    fn text(&self, pos: Pos2, anchor: Align2, text: String, font: FontId, color: Color32) {
        Painter::text(self, pos, anchor, text, font, color);
    }
    fn glyph(&self, ch: char, at: Pos2, size: f32, color: Color32) -> f32 {
        let galley = self.layout_no_wrap(
            ch.to_string(),
            FontId::new(size, FontFamily::Name(super::view::FAMILY.into())),
            color,
        );
        let (baseline, width) = galley
            .rows
            .first()
            .map(|r| {
                let g = r.row.glyphs.first();
                (
                    r.pos.y + g.map_or(0.0, |g| g.pos.y),
                    g.map_or(0.0, |g| g.advance_width),
                )
            })
            .unwrap_or((0.0, 0.0));
        self.galley(at - Vec2::new(0.0, baseline), galley, color);
        width
    }
}
