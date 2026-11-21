//! Fokus-Regel (Spec „Tastaturkonzept“ → Fokus-Regel): Ein Klick in eine Zone gibt ihr den
//! Fokus, ein schmaler Rahmen in Akzentfarbe zeigt ihn. Hat der Key-Editor in der unteren Zone
//! den Fokus, wirken Bearbeitungsbefehle auf seine Noten.

use egui::{Context, Id, LayerId, Order, Pos2, Rect, Stroke, StrokeKind};

use crate::{LowerZone, ProjectWindow};

/// Die Zonen des Projektfensters, die den Fokus haben können.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub(crate) enum Zone {
    /// Spurliste und Event-Anzeige.
    #[default]
    Project,
    /// Linke Zone (Inspector).
    Left,
    /// Untere Zone (Mixer, Key-Editor).
    Lower,
    /// Rechte Zone (Medien, Plugins).
    Right,
}

/// Lage der Zonen im letzten Frame; `None`, wenn eine Zone ausgeblendet ist.
#[derive(Debug, Clone, Copy, Default)]
pub(crate) struct ZoneRects {
    pub project: Option<Rect>,
    pub left: Option<Rect>,
    pub lower: Option<Rect>,
    pub right: Option<Rect>,
}

impl ZoneRects {
    fn get(&self, zone: Zone) -> Option<Rect> {
        match zone {
            Zone::Project => self.project,
            Zone::Left => self.left,
            Zone::Lower => self.lower,
            Zone::Right => self.right,
        }
    }

    /// Die Zone unter `p`.
    pub fn at(&self, p: Pos2) -> Option<Zone> {
        [Zone::Lower, Zone::Left, Zone::Right, Zone::Project]
            .into_iter()
            .find(|&z| self.get(z).is_some_and(|r| r.contains(p)))
    }
}

impl ProjectWindow {
    /// Nach dem Zeichnen aller Zonen: Fokus beim Mausdruck umsetzen und den Rahmen malen.
    pub(crate) fn update_focus(&mut self, ctx: &Context, rects: ZoneRects) {
        let press = ctx.input(|i| {
            i.pointer
                .any_pressed()
                .then(|| i.pointer.press_origin())
                .flatten()
        });
        // Über einem schwebenden Fenster (z. B. Quantisierungs-Panel) bleibt der Fokus.
        if let Some(p) = press
            && !ctx
                .layer_id_at(p)
                .is_some_and(|l| l.order != Order::Background)
            && let Some(zone) = rects.at(p)
        {
            self.focus = zone;
            // Key- und Noten-Editor bearbeiten dieselben Noten (Entf, Pfeiltasten).
            self.key_editor.focused =
                zone == Zone::Lower && matches!(self.lower, LowerZone::Editor | LowerZone::Score);
        }
        // Eine ausgeblendete Zone gibt den Fokus an die Event-Anzeige zurück.
        if rects.get(self.focus).is_none() {
            self.focus = Zone::Project;
            self.key_editor.focused = false;
        }
        let Some(rect) = rects.get(self.focus) else {
            return;
        };
        let accent = crate::theme::tokens(ctx).accent;
        ctx.layer_painter(LayerId::new(Order::Foreground, Id::unique("fokusrahmen")))
            .rect_stroke(
                rect.shrink(1.0),
                0.0,
                Stroke::new(1.5, accent),
                StrokeKind::Inside,
            );
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use egui::pos2;

    #[test]
    fn the_lower_zone_wins_where_rects_touch() {
        let rects = ZoneRects {
            project: Some(Rect::from_min_max(pos2(0.0, 0.0), pos2(100.0, 100.0))),
            left: None,
            lower: Some(Rect::from_min_max(pos2(0.0, 80.0), pos2(100.0, 100.0))),
            right: None,
        };
        assert_eq!(rects.at(pos2(50.0, 90.0)), Some(Zone::Lower));
        assert_eq!(rects.at(pos2(50.0, 10.0)), Some(Zone::Project));
        assert_eq!(rects.at(pos2(500.0, 10.0)), None);
    }
}
