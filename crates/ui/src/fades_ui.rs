//! Kurveneditor für Fades (Spec „Fades und Crossfades“, Issue #5): Doppelklick auf einen
//! Fade-Griff öffnet ihn; Form und Länge beider Fades des Events, nicht-destruktiv.

use egui::{Pos2, Sense, Stroke, pos2, vec2};
use studiocraft_project::{EventId, FadeCurve};

/// Welcher Fade den Editor geöffnet hat (er zeigt beide, dieser zuerst).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Side {
    In,
    Out,
}

/// Zustand des Fenster „Fade“.
#[derive(Debug, Clone)]
pub(crate) struct FadeEditor {
    pub id: EventId,
    pub side: Side,
    /// Stand vor der laufenden Änderung: ein Ziehen am Wert ist ein Undo-Schritt.
    before: Option<studiocraft_project::Project>,
}

/// Kleine Vorschau einer Kurve (steigend für Fade-In, fallend für Fade-Out).
fn preview(ui: &mut egui::Ui, curve: FadeCurve, rising: bool) {
    let (rect, _) = ui.allocate_exact_size(vec2(64.0, 32.0), Sense::hover());
    let t = crate::theme::tokens(ui.ctx());
    let p = ui.painter_at(rect);
    p.rect_filled(rect, 2.0, t.lane_alt);
    let pts: Vec<Pos2> = (0..=16)
        .map(|i| {
            let x = i as f32 / 16.0;
            let g = curve.shape(if rising { x } else { 1.0 - x });
            pos2(
                rect.left() + x * rect.width(),
                rect.bottom() - g * rect.height(),
            )
        })
        .collect();
    p.add(egui::Shape::line(pts, Stroke::new(1.5, t.accent)));
}

impl crate::ProjectWindow {
    pub(crate) fn open_fade_editor(&mut self, id: EventId, side: Side) {
        self.fade_editor = Some(FadeEditor {
            id,
            side,
            before: None,
        });
    }

    /// Das Fenster „Fade“; jede Änderung ist ein Undo-Schritt.
    pub(crate) fn fade_editor_dialog(&mut self, ctx: &egui::Context) {
        let Some(ed) = self.fade_editor.clone() else {
            return;
        };
        let Some(ev) = self.project.event(ed.id).cloned() else {
            self.fade_editor = None;
            return;
        };
        let mut open = true;
        let mut new = ev.clone();
        egui::Window::new("Fade")
            .open(&mut open)
            .resizable(false)
            .collapsible(false)
            .show(ctx, |ui| {
                let sides = match ed.side {
                    Side::In => [Side::In, Side::Out],
                    Side::Out => [Side::Out, Side::In],
                };
                for side in sides {
                    let (title, curve, len) = match side {
                        Side::In => ("Fade-In", &mut new.fade_in_curve, &mut new.fade_in),
                        Side::Out => ("Fade-Out", &mut new.fade_out_curve, &mut new.fade_out),
                    };
                    ui.strong(title);
                    ui.horizontal(|ui| {
                        preview(ui, *curve, side == Side::In);
                        ui.vertical(|ui| {
                            for c in FadeCurve::ALL {
                                ui.radio_value(curve, c, c.label())
                                    .on_hover_text(format!("{title}: {}", c.label()));
                            }
                        });
                    });
                    ui.horizontal(|ui| {
                        let l = ui.label("Länge (Ticks)");
                        ui.add(egui::DragValue::new(len).range(0..=ev.length).speed(10.0))
                            .labelled_by(l.id);
                    });
                    ui.separator();
                }
                let gl = ui.label("Lautstärke (dB)");
                ui.add(
                    egui::DragValue::new(&mut new.gain_db)
                        .range(studiocraft_project::MIN_DB..=studiocraft_project::EVENT_MAX_DB)
                        .speed(0.1)
                        .fixed_decimals(1),
                )
                .labelled_by(gl.id);
            });
        // Beide Fades zusammen höchstens so lang wie das Event.
        new.fade_in = new.fade_in.min(new.length);
        new.fade_out = new.fade_out.min(new.length - new.fade_in);
        let mut before = ed.before;
        if new != ev {
            before.get_or_insert_with(|| self.project.clone());
            if let Some(e) = self.project.event_mut(ed.id) {
                *e = new;
            }
        }
        // Erst wenn nichts mehr gezogen wird, wird die Änderung ein Undo-Schritt.
        if ctx.dragged_id().is_none()
            && let Some(b) = before.take()
        {
            self.checkpoint(b);
        }
        self.fade_editor = open.then_some(FadeEditor {
            id: ed.id,
            side: ed.side,
            before,
        });
    }
}
