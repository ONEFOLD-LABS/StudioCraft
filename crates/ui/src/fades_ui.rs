//! Kurveneditor für Fades (Spec „Fades und Crossfades“, Issue #5): Doppelklick auf einen
//! Fade-Griff öffnet ihn; Form und Länge beider Fades des Events, nicht-destruktiv.

use crate::i18n::tr;
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

/// Zustand des Fensters „Crossfade“: früheres und späteres Event.
#[derive(Debug, Clone)]
pub(crate) struct CrossfadeEditor {
    pub first: EventId,
    pub second: EventId,
}

/// Beide Kurven eines Crossfades übereinander, dazu als dünne Linie ihre gemeinsame Leistung:
/// bei „Gleiche Leistung“ bleibt sie waagerecht.
fn crossfade_preview(ui: &mut egui::Ui, out_curve: FadeCurve, in_curve: FadeCurve) {
    let (rect, _) = ui.allocate_exact_size(vec2(200.0, 64.0), Sense::hover());
    let t = crate::theme::tokens(ui.ctx());
    let p = ui.painter_at(rect);
    p.rect_filled(rect, 2.0, t.lane_alt);
    let line = |f: &dyn Fn(f32) -> f32| -> Vec<Pos2> {
        (0..=32)
            .map(|i| {
                let x = i as f32 / 32.0;
                pos2(
                    rect.left() + x * rect.width(),
                    rect.bottom() - f(x).clamp(0.0, 1.2) / 1.2 * rect.height(),
                )
            })
            .collect()
    };
    p.add(egui::Shape::line(
        line(&|x| out_curve.shape(1.0 - x)),
        Stroke::new(1.5, t.accent),
    ));
    p.add(egui::Shape::line(
        line(&|x| in_curve.shape(x)),
        Stroke::new(1.5, t.monitor),
    ));
    p.add(egui::Shape::line(
        line(&|x| (out_curve.shape(1.0 - x).powi(2) + in_curve.shape(x).powi(2)).sqrt()),
        Stroke::new(1.0, t.text_dim),
    ));
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
    /// Doppelklick auf einen Fade: Gehört er zu einem Crossfade, öffnet der Crossfade-Editor,
    /// sonst der Fade-Editor.
    pub(crate) fn open_fade_editor(&mut self, id: EventId, side: Side) {
        if let Some((first, second)) = self.project.crossfade_pair(id, side == Side::In) {
            self.fade_editor = None;
            self.crossfade_editor = Some(CrossfadeEditor { first, second });
            return;
        }
        self.fade_editor = Some(FadeEditor {
            id,
            side,
            before: None,
        });
    }

    /// Das Fenster „Crossfade“: eine Kurvenform für beide Seiten, Vorschau beider Kurven.
    pub(crate) fn crossfade_editor_dialog(&mut self, ctx: &egui::Context) {
        let Some(ed) = self.crossfade_editor.clone() else {
            return;
        };
        let (Some(a), Some(b)) = (
            self.project.event(ed.first).cloned(),
            self.project.event(ed.second).cloned(),
        ) else {
            self.crossfade_editor = None;
            return;
        };
        let mut open = true;
        let (mut out_curve, mut in_curve) = (a.fade_out_curve, b.fade_in_curve);
        let overlap = a.end().saturating_sub(b.start.0);
        let tempo = self.project.tempo.clone();
        egui::Window::new(tr("Crossfade"))
            .open(&mut open)
            .resizable(false)
            .collapsible(false)
            .show(ctx, |ui| {
                ui.label(format!("{} / {}", a.name, b.name));
                ui.label(crate::i18n::trf(
                    "Länge {len}",
                    &[("len", &tempo.format_length(overlap))],
                ));
                crossfade_preview(ui, out_curve, in_curve);
                let mut shared = (out_curve == in_curve).then_some(out_curve);
                for (c, label) in [
                    (FadeCurve::Linear, tr("Gleiche Lautstärke")),
                    (FadeCurve::EqualPower, tr("Gleiche Leistung")),
                    (FadeCurve::SCurve, tr("S-Kurve")),
                    (FadeCurve::Exponential, tr("Exponentiell")),
                ] {
                    if ui.radio_value(&mut shared, Some(c), label).changed() {
                        out_curve = c;
                        in_curve = c;
                    }
                }
                ui.separator();
                ui.horizontal(|ui| {
                    ui.label(tr("Einzeln:"));
                    egui::ComboBox::from_id_salt("xfade-out")
                        .selected_text(tr(out_curve.label()))
                        .show_ui(ui, |ui| {
                            for c in FadeCurve::ALL {
                                ui.selectable_value(&mut out_curve, c, tr(c.label()));
                            }
                        })
                        .response
                        .widget_info(|| {
                            egui::WidgetInfo::labeled(
                                egui::Role::ComboBox,
                                true,
                                tr("Kurve Fade-Out"),
                            )
                        });
                    egui::ComboBox::from_id_salt("xfade-in")
                        .selected_text(tr(in_curve.label()))
                        .show_ui(ui, |ui| {
                            for c in FadeCurve::ALL {
                                ui.selectable_value(&mut in_curve, c, tr(c.label()));
                            }
                        })
                        .response
                        .widget_info(|| {
                            egui::WidgetInfo::labeled(
                                egui::Role::ComboBox,
                                true,
                                tr("Kurve Fade-In"),
                            )
                        });
                });
            });
        if (out_curve, in_curve) != (a.fade_out_curve, b.fade_in_curve) {
            let before = self.project.clone();
            if let Some(e) = self.project.event_mut(ed.first) {
                e.fade_out_curve = out_curve;
            }
            if let Some(e) = self.project.event_mut(ed.second) {
                e.fade_in_curve = in_curve;
            }
            self.checkpoint(before);
        }
        if !open {
            self.crossfade_editor = None;
        }
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
        egui::Window::new(tr("Fade"))
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
                        Side::In => (tr("Fade-In"), &mut new.fade_in_curve, &mut new.fade_in),
                        Side::Out => (tr("Fade-Out"), &mut new.fade_out_curve, &mut new.fade_out),
                    };
                    ui.strong(title);
                    ui.horizontal(|ui| {
                        preview(ui, *curve, side == Side::In);
                        ui.vertical(|ui| {
                            for c in FadeCurve::ALL {
                                ui.radio_value(curve, c, tr(c.label()))
                                    .on_hover_text(format!("{title}: {}", tr(c.label())));
                            }
                        });
                    });
                    ui.horizontal(|ui| {
                        let l = ui.label(tr("Länge (Ticks)"));
                        ui.add(egui::DragValue::new(len).range(0..=ev.length).speed(10.0))
                            .labelled_by(l.id);
                    });
                    ui.separator();
                }
                let gl = ui.label(tr("Lautstärke (dB)"));
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
