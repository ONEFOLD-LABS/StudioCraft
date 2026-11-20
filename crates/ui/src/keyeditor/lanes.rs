//! Controller-Spuren im Key-Editor (Spec „Editoren“ → Controller-Lanes, Issue #17): unter
//! der Velocity-Spur beliebig viele Spuren für Pitchbend, Controller 0–127 und Aftertouch.
//! Werte erscheinen als Stufen (ein Wert gilt bis zum nächsten).
//!
//! Zeichnen-Werkzeug: Ziehen setzt Werte im Rasterabstand und ersetzt, was im überstrichenen
//! Bereich lag. Auswahl: Klick wählt einen Punkt (Umschalt: dazu), Ziehen verschiebt die
//! gewählten Punkte in Zeit und Wert. Löschen-Werkzeug: entfernt Punkte unter dem Zeiger.
//! Entf löscht die gewählten Punkte. Jede Geste ist ein Rückgängig-Schritt.

use crate::i18n::tr;
use egui::{Align2, Color32, FontId, Painter, Pos2, Rect, Stroke, Ui, pos2, vec2};
use studiocraft_project::{CtrlKind, CtrlPoint, EventId};

use super::KGeo;
use crate::{ProjectWindow, Tool};

/// Höhe einer Controller-Spur in Punkten.
pub(super) const CTRL_H: f32 = 64.0;
/// Höchstzahl Controller-Spuren.
pub(crate) const MAX_LANES: usize = 8;
/// Fangbereich um einen Punkt in Pixel.
const HIT: f32 = 5.0;

/// Laufende Geste in einer Controller-Spur.
#[derive(Debug, Clone)]
pub(crate) enum CtrlDrag {
    /// Zeichnen; `last` = zuletzt gesetzte Position in Ticks (relativ zum Part).
    Draw { kind: CtrlKind, last: Option<i64> },
    /// Punkte unter dem Zeiger löschen; `last_x` = Zeigerposition im letzten Bild, damit
    /// schnelle Bewegungen nichts überspringen.
    Erase { kind: CtrlKind, last_x: Option<f32> },
    /// Gewählte Punkte verschieben; `original` = Punkte vor dem Ziehen, Anker in Ticks und Wert.
    Move {
        kind: CtrlKind,
        original: Vec<CtrlPoint>,
        tick: f64,
        value: f64,
    },
}

/// Eine Controller-Spur auf dem Bildschirm.
#[derive(Debug, Clone, Copy)]
pub(super) struct Lane {
    pub index: usize,
    pub kind: CtrlKind,
    pub rect: Rect,
}

fn value_at(rect: Rect, kind: CtrlKind, y: f32) -> u16 {
    let f = ((rect.bottom() - y) / rect.height()).clamp(0.0, 1.0);
    (f * f32::from(kind.max())).round() as u16
}

fn y_of(rect: Rect, kind: CtrlKind, v: u16) -> f32 {
    rect.bottom() - rect.height() * f32::from(v) / f32::from(kind.max())
}

/// Kurzname für den Spurkopf in der schmalen Klaviaturspalte.
pub(crate) fn short(kind: CtrlKind) -> String {
    match kind {
        CtrlKind::PitchBend => "PB".into(),
        CtrlKind::Cc(n) => format!("CC{n}"),
        CtrlKind::Aftertouch => "AT".into(),
    }
}

/// Punkte einer Art, nach Zeit sortiert, mit ihrem Index in `controllers`.
fn points(ctrls: &[CtrlPoint], kind: CtrlKind) -> Vec<(usize, CtrlPoint)> {
    let mut v: Vec<(usize, CtrlPoint)> = ctrls
        .iter()
        .copied()
        .enumerate()
        .filter(|(_, c)| c.kind == kind)
        .collect();
    v.sort_by_key(|(_, c)| c.start);
    v
}

fn hit(g: &KGeo, lane: &Lane, ctrls: &[CtrlPoint], p: Pos2) -> Option<usize> {
    points(ctrls, lane.kind)
        .into_iter()
        .rev()
        .find(|(_, c)| {
            let x = g.x(g.part_start as f64 + c.start as f64);
            let y = y_of(lane.rect, lane.kind, c.value);
            (x - p.x).abs() <= HIT && (y - p.y).abs() <= HIT
        })
        .map(|(i, _)| i)
}

impl ProjectWindow {
    /// Neue Controller-Spur (höchstens [`MAX_LANES`]; eine Art nur einmal).
    pub(crate) fn add_ctrl_lane(&mut self, kind: CtrlKind) {
        let lanes = &mut self.key_editor.lanes;
        if lanes.contains(&kind) {
            return;
        }
        if lanes.len() >= MAX_LANES {
            self.message = Some(crate::i18n::trf(
                "Höchstens {max} Controller-Spuren",
                &[("max", &MAX_LANES)],
            ));
            return;
        }
        lanes.push(kind);
    }

    /// Gewählte Controller-Punkte des aktiven Parts löschen (ein Rückgängig-Schritt).
    pub(crate) fn delete_selected_ctrls(&mut self) {
        let Some(part) = self.key_editor.part else {
            return;
        };
        let mut sel = std::mem::take(&mut self.key_editor.ctrl_sel);
        if sel.is_empty() {
            return;
        }
        sel.sort_unstable();
        let before = self.project.clone();
        if let Some(ev) = self.project.event_mut(part) {
            for i in sel.into_iter().rev() {
                if i < ev.controllers.len() {
                    ev.controllers.remove(i);
                }
            }
        }
        self.checkpoint(before);
    }
}

/// Spurköpfe in der Klaviaturspalte: Art wählen und Spur entfernen.
pub(super) fn heads(w: &mut ProjectWindow, ui: &mut Ui, lanes: &[Lane], keys_left: f32) {
    let mut remove = None;
    let mut change = None;
    for lane in lanes {
        let head = Rect::from_min_size(
            pos2(keys_left + 2.0, lane.rect.top() + 2.0),
            vec2(super::KEYS_W - 4.0, 18.0),
        );
        ui.scope_builder(egui::UiBuilder::new().max_rect(head), |ui| {
            ui.horizontal(|ui| {
                ui.spacing_mut().item_spacing.x = 1.0;
                ui.menu_button(short(lane.kind), |ui| {
                    if let Some(k) = kind_menu(ui) {
                        change = Some((lane.index, k));
                        ui.close();
                    }
                })
                .response
                .on_hover_text(crate::i18n::tr_dyn(&lane.kind.label()));
                if ui
                    .small_button(tr("x"))
                    .on_hover_text(crate::i18n::trf(
                        "Spur {name} entfernen",
                        &[("name", &crate::i18n::tr_dyn(&lane.kind.label()))],
                    ))
                    .clicked()
                {
                    remove = Some(lane.index);
                }
            });
        });
    }
    if let Some((i, k)) = change
        && !w.key_editor.lanes.contains(&k)
        && let Some(slot) = w.key_editor.lanes.get_mut(i)
    {
        *slot = k;
        w.key_editor.ctrl_sel.clear();
    }
    if let Some(i) = remove
        && i < w.key_editor.lanes.len()
    {
        w.key_editor.lanes.remove(i);
        w.key_editor.ctrl_sel.clear();
    }
}

/// Liste der Controller-Arten zum Auswählen; gibt die gewählte zurück.
pub(crate) fn kind_menu(ui: &mut Ui) -> Option<CtrlKind> {
    let mut picked = None;
    for k in [
        CtrlKind::PitchBend,
        CtrlKind::Aftertouch,
        CtrlKind::Cc(1),
        CtrlKind::Cc(7),
        CtrlKind::Cc(10),
        CtrlKind::Cc(11),
    ] {
        if ui.button(crate::i18n::tr_dyn(&k.label())).clicked() {
            picked = Some(k);
        }
    }
    ui.menu_button(tr("Weitere Controller"), |ui| {
        egui::ScrollArea::vertical()
            .max_height(260.0)
            .show(ui, |ui| {
                for n in 0..=127u8 {
                    if ui
                        .button(crate::i18n::tr_dyn(&CtrlKind::Cc(n).label()))
                        .clicked()
                    {
                        picked = Some(CtrlKind::Cc(n));
                    }
                }
            });
    });
    picked
}

/// Maus in den Controller-Spuren. Gibt `true` zurück, wenn die Geste dort stattfand.
pub(super) fn interact(
    w: &mut ProjectWindow,
    ui: &Ui,
    resp: &egui::Response,
    g: &KGeo,
    part: EventId,
    lanes: &[Lane],
) -> bool {
    let (mods, press, now) = ui.input(|i| {
        (
            i.modifiers,
            i.pointer.press_origin(),
            i.pointer.interact_pos(),
        )
    });
    let press = press.or_else(|| resp.interact_pointer_pos());
    let starting = resp.drag_started() || resp.clicked();
    if starting
        && w.key_editor.ctrl_drag.is_none()
        && let Some(p) = press
        && let Some(lane) = lanes.iter().find(|l| l.rect.contains(p))
    {
        let ctrls = w
            .project
            .event(part)
            .map(|e| e.controllers.clone())
            .unwrap_or_default();
        w.drag_before = Some(w.project.clone());
        w.key_editor.ctrl_drag = match w.tool {
            Tool::Draw => Some(CtrlDrag::Draw {
                kind: lane.kind,
                last: None,
            }),
            Tool::Erase => Some(CtrlDrag::Erase {
                kind: lane.kind,
                last_x: None,
            }),
            _ => match hit(g, lane, &ctrls, p) {
                Some(i) => {
                    let sel = &mut w.key_editor.ctrl_sel;
                    if mods.shift {
                        if !sel.contains(&i) {
                            sel.push(i);
                        }
                    } else if !sel.contains(&i) {
                        *sel = vec![i];
                    }
                    Some(CtrlDrag::Move {
                        kind: lane.kind,
                        original: ctrls,
                        tick: g.tick(p.x),
                        value: f64::from(value_at(lane.rect, lane.kind, p.y)),
                    })
                }
                None => {
                    w.key_editor.ctrl_sel.clear();
                    None
                }
            },
        };
        w.key_editor.focused = true;
    }
    let Some(drag) = w.key_editor.ctrl_drag.clone() else {
        return press.is_some_and(|p| lanes.iter().any(|l| l.rect.contains(p))) && starting;
    };
    if let Some(p) = now.or(press) {
        apply(w, g, part, lanes, drag, p, mods.alt);
    }
    if resp.drag_stopped() || resp.clicked() || !ui.input(|i| i.pointer.any_down()) {
        w.key_editor.ctrl_drag = None;
        if let Some(b) = w.drag_before.take() {
            w.checkpoint(b);
        }
    }
    true
}

fn apply(
    w: &mut ProjectWindow,
    g: &KGeo,
    part: EventId,
    lanes: &[Lane],
    drag: CtrlDrag,
    p: Pos2,
    free: bool,
) {
    let kind = match &drag {
        CtrlDrag::Draw { kind, .. }
        | CtrlDrag::Erase { kind, .. }
        | CtrlDrag::Move { kind, .. } => *kind,
    };
    let Some(lane) = lanes.iter().find(|l| l.kind == kind).copied() else {
        return;
    };
    let step = w.grid_ticks().max(1) as f64;
    let rel = g.tick(p.x) - g.part_start as f64;
    let snapped = if free {
        rel.round()
    } else {
        (rel / step).round() * step
    };
    let at = (snapped.max(0.0)) as i64;
    let value = value_at(lane.rect, kind, p.y);
    let sel_before = w.key_editor.ctrl_sel.clone();
    let Some(ev) = w.project.event_mut(part) else {
        return;
    };
    match drag {
        CtrlDrag::Draw { last, .. } => {
            // Was zwischen dem letzten und diesem Punkt lag, ersetzen.
            let (a, b) = match last {
                Some(l) => (l.min(at), l.max(at)),
                None => (at, at),
            };
            ev.controllers.retain(|c| {
                c.kind != kind || c.start < a || c.start > b || c.start == last.unwrap_or(-1)
            });
            ev.controllers.retain(|c| c.kind != kind || c.start != at);
            ev.controllers.push(CtrlPoint {
                kind,
                start: at,
                value,
            });
            ev.controllers.sort_by_key(|c| c.start);
            w.key_editor.ctrl_sel.clear();
            w.key_editor.ctrl_drag = Some(CtrlDrag::Draw {
                kind,
                last: Some(at),
            });
        }
        CtrlDrag::Erase { last_x, .. } => {
            // Alles zwischen der letzten und der jetzigen Zeigerposition (ganze Spurhöhe).
            let (x0, x1) = match last_x {
                Some(l) => (l.min(p.x) - HIT, l.max(p.x) + HIT),
                None => (p.x - HIT, p.x + HIT),
            };
            let ps = g.part_start as f64;
            ev.controllers.retain(|c| {
                let x = g.x(ps + c.start as f64);
                c.kind != kind || x < x0 || x > x1
            });
            w.key_editor.ctrl_sel.clear();
            w.key_editor.ctrl_drag = Some(CtrlDrag::Erase {
                kind,
                last_x: Some(p.x),
            });
        }
        CtrlDrag::Move {
            original,
            tick,
            value: v0,
            ..
        } => {
            let dt = ((g.tick(p.x) - tick) / step).round() * step;
            let dt = if free {
                (g.tick(p.x) - tick).round()
            } else {
                dt
            } as i64;
            let dv = f64::from(value) - v0;
            ev.controllers = original.clone();
            let max = f64::from(kind.max());
            for &i in &sel_before {
                if let Some(c) = ev.controllers.get_mut(i)
                    && c.kind == kind
                {
                    c.start = (c.start + dt).max(0);
                    c.value = (f64::from(c.value) + dv).clamp(0.0, max).round() as u16;
                }
            }
            // Nicht sortieren: die Auswahl bleibt so bei ihren Indizes (Anzeige und
            // Wiedergabe sortieren selbst).
            w.key_editor.ctrl_drag = Some(CtrlDrag::Move {
                kind,
                original,
                tick,
                value: v0,
            });
        }
    }
}

/// Zeichnet die Controller-Spuren des aktiven Parts.
pub(super) fn paint(
    w: &ProjectWindow,
    p: &Painter,
    g: &KGeo,
    lanes: &[Lane],
    ctrls: &[CtrlPoint],
    color: Color32,
) {
    let t = crate::theme::tokens(p.ctx());
    let v = p.ctx().global_style().visuals.clone();
    for lane in lanes {
        let r = lane.rect;
        let p = p.with_clip_rect(r);
        p.rect_filled(r, 0.0, v.extreme_bg_color);
        p.hline(
            r.x_range(),
            r.top(),
            Stroke::new(1.0, v.widgets.noninteractive.bg_stroke.color),
        );
        if lane.kind == CtrlKind::PitchBend {
            p.hline(
                r.x_range(),
                y_of(r, lane.kind, lane.kind.rest()),
                Stroke::new(1.0, v.weak_text_color().gamma_multiply(0.5)),
            );
        }
        p.text(
            r.right_top() + vec2(-4.0, 2.0),
            Align2::RIGHT_TOP,
            crate::i18n::tr_dyn(&lane.kind.label()),
            FontId::proportional(10.0),
            v.weak_text_color(),
        );
        let pts = points(ctrls, lane.kind);
        let line = Stroke::new(1.5, color);
        for (k, (i, c)) in pts.iter().enumerate() {
            let x = g.x(g.part_start as f64 + c.start as f64);
            let y = y_of(r, lane.kind, c.value);
            let next_x = pts.get(k + 1).map_or(r.right(), |(_, n)| {
                g.x(g.part_start as f64 + n.start as f64)
            });
            p.hline(x..=next_x, y, line);
            if let Some((_, n)) = pts.get(k + 1) {
                p.vline(
                    next_x,
                    y.min(y_of(r, lane.kind, n.value))..=y.max(y_of(r, lane.kind, n.value)),
                    line,
                );
            }
            let selected = w.key_editor.ctrl_sel.contains(i);
            p.circle_filled(
                pos2(x, y),
                if selected { 3.5 } else { 2.5 },
                if selected { t.note_selected } else { color },
            );
        }
    }
}
