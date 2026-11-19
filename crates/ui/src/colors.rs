//! Spur- und Event-Farben (Spec „Farbe“, Issue #19): Jede Spur hat eine Farbe, neue Events
//! erben sie. Ein Klick auf die Farbleiste im Spurkopf öffnet die Palette; über
//! Bearbeiten → Event-Farbe bekommen gewählte Events eine eigene Farbe oder wieder die der Spur.

use egui::{Color32, Context, Id, Pos2, Response, Sense, Stroke, Ui, vec2};
use studiocraft_project::TRACK_COLORS;

use crate::ProjectWindow;

/// Namen der Palettenfarben (Reihenfolge wie [`TRACK_COLORS`]).
const NAMES: [&str; 8] = [
    "Rot", "Orange", "Gelb", "Grün", "Türkis", "Blau", "Violett", "Pink",
];

/// Wofür die Palette gerade offen ist.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Target {
    Track(usize),
}

fn rgb(c: [u8; 3]) -> Color32 {
    Color32::from_rgb(c[0], c[1], c[2])
}

/// Ein Farbfeld mit zugänglichem Namen.
pub(crate) fn swatch(ui: &mut Ui, color: [u8; 3], name: &str, current: bool) -> Response {
    let (rect, resp) = ui.allocate_exact_size(vec2(20.0, 20.0), Sense::click());
    resp.widget_info(|| egui::WidgetInfo::selected(egui::Role::Button, true, current, name));
    let t = crate::theme::tokens(ui.ctx());
    ui.painter().rect_filled(rect.shrink(2.0), 3.0, rgb(color));
    if current || resp.hovered() {
        ui.painter().rect_stroke(
            rect,
            4.0,
            Stroke::new(1.5, t.text),
            egui::StrokeKind::Inside,
        );
    }
    resp.on_hover_text(name)
}

/// Die Palette: gibt die gewählte Farbe zurück.
pub(crate) fn palette(ui: &mut Ui, current: Option<[u8; 3]>) -> Option<[u8; 3]> {
    let mut picked = None;
    ui.horizontal(|ui| {
        ui.spacing_mut().item_spacing.x = 2.0;
        for (c, name) in TRACK_COLORS.iter().zip(NAMES) {
            if swatch(ui, *c, name, current == Some(*c)).clicked() {
                picked = Some(*c);
            }
        }
    });
    picked
}

impl ProjectWindow {
    /// Öffnet die Palette für `target` an `pos` (Klick auf die Farbleiste).
    pub(crate) fn open_color_popup(&mut self, target: Target, pos: Pos2) {
        self.color_popup = Some((target, pos));
        self.color_popup_age = 0;
    }

    /// Zeichnet die offene Palette als schwebende Fläche; ein Klick daneben schließt sie.
    pub(crate) fn color_popup(&mut self, ctx: &Context) {
        let Some((Target::Track(row), pos)) = self.color_popup else {
            return;
        };
        let Some(current) = self.project.tracks.get(row).map(|t| t.color) else {
            self.color_popup = None;
            return;
        };
        let resp = egui::Area::new(Id::unique("spurfarbe-palette"))
            .order(egui::Order::Foreground)
            .fixed_pos(pos)
            .show(ctx, |ui| {
                egui::Frame::popup(ui.style())
                    .show(ui, |ui| {
                        ui.label("Spurfarbe");
                        let picked = palette(ui, Some(current));
                        let mut custom = rgb(current);
                        let l = ui.weak("Eigene");
                        let changed = ui
                            .color_edit_button_srgba(&mut custom)
                            .labelled_by(l.id)
                            .changed();
                        picked.or(changed.then(|| [custom.r(), custom.g(), custom.b()]))
                    })
                    .inner
            });
        if let Some(c) = resp.inner {
            let before = self.project.clone();
            if let Some(t) = self.project.tracks.get_mut(row) {
                t.color = c;
            }
            self.checkpoint(before);
            self.color_popup = None;
            return;
        }
        let clicked_outside = ctx.input(|i| i.pointer.any_click())
            && ctx
                .input(|i| i.pointer.interact_pos())
                .is_some_and(|p| !resp.response.rect.contains(p));
        // Nicht im selben Bild schließen, in dem der Klick sie geöffnet hat.
        if clicked_outside && self.color_popup_age > 0 {
            self.color_popup = None;
        }
        self.color_popup_age = self.color_popup_age.saturating_add(1);
    }

    /// Gewählten Events eine eigene Farbe geben (`None`: wieder die Spurfarbe).
    pub(crate) fn set_event_color(&mut self, color: Option<[u8; 3]>) {
        let before = self.project.clone();
        for id in self.selection.clone() {
            if let Some(e) = self.project.event_mut(id) {
                e.color = color;
            }
        }
        self.checkpoint(before);
    }

    /// Menü Bearbeiten → Event-Farbe.
    pub(crate) fn event_color_menu(&mut self, ui: &mut Ui) {
        let enabled = !self.selection.is_empty();
        ui.add_enabled_ui(enabled, |ui| {
            ui.menu_button("Event-Farbe", |ui| {
                if let Some(c) = palette(ui, None) {
                    self.set_event_color(Some(c));
                    ui.close();
                }
                if ui.button("Spurfarbe übernehmen").clicked() {
                    self.set_event_color(None);
                    ui.close();
                }
            });
        });
    }
}
