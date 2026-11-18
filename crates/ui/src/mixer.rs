//! Mixer in der unteren Zone (Spec-Abschnitt „Mixer“): ein Kanalzug je Spur
//! mit Kanal, rechts der Ausgangsbus. F3 blendet ihn ein und aus.

use egui::{
    Align, Align2, Color32, FontId, Layout, Rect, Response, RichText, Sense, Stroke, StrokeKind,
    Ui, WidgetInfo, pos2, vec2,
};
use studiocraft_project::{MAX_DB, MIN_DB, TrackKind};

use crate::ProjectWindow;

const STRIP_W: f32 = 76.0;
/// Abfall der Pegelanzeige pro Sekunde (Faktor), etwa 20 dB/s.
const METER_FALL_PER_SEC: f32 = 0.1;

/// Fader-Kennlinie: Position 0..1 aus dB. Oben feiner aufgelöst als unten,
/// damit der Bereich um 0 dB gut einzustellen ist.
pub(crate) fn db_to_pos(db: f32) -> f32 {
    if db <= MIN_DB {
        return 0.0;
    }
    ((db - MIN_DB) / (MAX_DB - MIN_DB)).clamp(0.0, 1.0).powi(2)
}

pub(crate) fn pos_to_db(pos: f32) -> f32 {
    if pos <= 0.0005 {
        return MIN_DB;
    }
    MIN_DB + pos.clamp(0.0, 1.0).sqrt() * (MAX_DB - MIN_DB)
}

fn lin_to_db(v: f32) -> f32 {
    if v <= 0.0 { MIN_DB } else { 20.0 * v.log10() }
}

pub(crate) fn db_label(db: f32) -> String {
    if db <= MIN_DB {
        "−∞".into()
    } else {
        format!("{db:+.1}")
    }
}

fn pan_label(pan: f32) -> String {
    let v = (pan * 100.0).round() as i32;
    match v {
        0 => "C".into(),
        v if v < 0 => format!("L{}", -v),
        v => format!("R{v}"),
    }
}

/// Ob eine Spur einen Mixer-Kanal hat (Spec „Spurtypen“).
pub(crate) fn has_channel(kind: TrackKind) -> bool {
    matches!(
        kind,
        TrackKind::Audio | TrackKind::Instrument | TrackKind::Group | TrackKind::Fx
    )
}

/// Pegelanzeigen: neue Spitzen übernehmen, sonst langsam fallen lassen.
pub(crate) fn update_meters(w: &mut ProjectWindow, dt: f32) {
    let fall = METER_FALL_PER_SEC.powf(dt.clamp(0.0, 0.5));
    let n = w.project.tracks.len();
    w.meter_levels.resize(n, [0.0; 2]);
    let fresh: Option<(Vec<[f32; 2]>, [f32; 2])> = w.transport.as_ref().map(|t| {
        let tracks = (0..n).map(|i| t.meters.take_track(i)).collect();
        (tracks, t.meters.take_master())
    });
    let (tracks, master) = fresh.unwrap_or_else(|| (vec![[0.0; 2]; n], [0.0; 2]));
    for (shown, new) in w.meter_levels.iter_mut().zip(tracks) {
        for c in 0..2 {
            shown[c] = new[c].max(shown[c] * fall);
        }
    }
    for c in 0..2 {
        w.master_meter[c] = master[c].max(w.master_meter[c] * fall);
    }
}

pub(crate) fn show(w: &mut ProjectWindow, ui: &mut Ui) {
    let height = ui.available_height();
    let mut changed = false;
    ui.horizontal(|ui| {
        let master_w = STRIP_W + 16.0;
        let tracks_w = (ui.available_width() - master_w).max(STRIP_W);
        ui.allocate_ui_with_layout(
            vec2(tracks_w, height),
            Layout::left_to_right(Align::Min),
            |ui| {
                // Nur sichtbare Kanalzüge bauen: auch bei hunderten Spuren bleibt
                // der Aufwand pro Frame konstant.
                let channels: Vec<usize> = (0..w.project.tracks.len())
                    .filter(|&i| has_channel(w.project.tracks[i].kind))
                    .collect();
                let pitch = STRIP_W + ui.spacing().item_spacing.x;
                egui::ScrollArea::horizontal()
                    .id_salt("mixer-kanaele")
                    .show_viewport(ui, |ui, viewport| {
                        let origin = ui.max_rect().min;
                        ui.set_min_size(vec2(channels.len() as f32 * pitch, height));
                        let first = (viewport.min.x / pitch).floor().max(0.0) as usize;
                        let last =
                            ((viewport.max.x / pitch).ceil() as usize + 1).min(channels.len());
                        for (k, &i) in channels.iter().enumerate().take(last).skip(first) {
                            let rect = Rect::from_min_size(
                                origin + vec2(k as f32 * pitch, 0.0),
                                vec2(STRIP_W, height),
                            );
                            let meter = w.meter_levels.get(i).copied().unwrap_or_default();
                            let selected = w.selected_track == Some(i);
                            let t = &mut w.project.tracks[i];
                            let color = Color32::from_rgb(t.color[0], t.color[1], t.color[2]);
                            let strip = Strip {
                                id: i,
                                name: &t.name,
                                color,
                                selected,
                                volume_db: &mut t.volume_db,
                                pan: Some(&mut t.pan),
                                mute: Some(&mut t.mute),
                                solo: Some(&mut t.solo),
                                meter,
                            };
                            let (c, clicked) = ui
                                .scope_builder(egui::UiBuilder::new().max_rect(rect), |ui| {
                                    strip.show(ui, height)
                                })
                                .inner;
                            changed |= c;
                            if clicked {
                                w.selected_track = Some(i);
                            }
                        }
                    });
            },
        );
        ui.separator();
        let master = Strip {
            id: usize::MAX,
            name: "Ausgang",
            color: Color32::from_gray(150),
            selected: false,
            volume_db: &mut w.project.master_db,
            pan: None,
            mute: None,
            solo: None,
            meter: w.master_meter,
        };
        changed |= master.show(ui, height).0;
    });
    if changed {
        w.dirty = true;
    }
}

struct Strip<'a> {
    id: usize,
    name: &'a str,
    color: Color32,
    selected: bool,
    volume_db: &'a mut f32,
    pan: Option<&'a mut f32>,
    mute: Option<&'a mut bool>,
    solo: Option<&'a mut bool>,
    meter: [f32; 2],
}

impl Strip<'_> {
    /// Zeichnet den Kanalzug. Rückgabe: (Wert geändert, Kanal angeklickt).
    fn show(self, ui: &mut Ui, height: f32) -> (bool, bool) {
        let mut changed = false;
        let fill = if self.selected {
            ui.visuals().selection.bg_fill.gamma_multiply(0.35)
        } else {
            ui.visuals().faint_bg_color
        };
        let frame = egui::Frame::new()
            .fill(fill)
            .corner_radius(4.0)
            .inner_margin(4.0);
        let resp = frame.show(ui, |ui| {
            ui.set_width(STRIP_W - 8.0);
            ui.set_height(height - 8.0);
            ui.with_layout(Layout::top_down(Align::Center), |ui| {
                ui.spacing_mut().item_spacing.y = 4.0;
                if let Some(pan) = self.pan {
                    changed |= pan_control(ui, self.id, pan);
                } else {
                    ui.add_space(18.0);
                }
                ui.horizontal(|ui| {
                    ui.spacing_mut().item_spacing.x = 2.0;
                    if let Some(m) = self.mute {
                        changed |= ui.toggle_value(m, "M").on_hover_text("Mute").changed();
                    }
                    if let Some(s) = self.solo {
                        changed |= ui.toggle_value(s, "S").on_hover_text("Solo").changed();
                    }
                });
                ui.label(
                    RichText::new(db_label(*self.volume_db))
                        .monospace()
                        .size(11.0),
                );
                let fader_h = (ui.available_height() - 24.0).max(60.0);
                changed |= fader(ui, self.name, self.volume_db, self.meter, fader_h).changed();
                let (rect, _) = ui.allocate_exact_size(vec2(STRIP_W - 8.0, 18.0), Sense::hover());
                ui.painter().rect_filled(rect, 2.0, self.color);
                ui.painter().with_clip_rect(rect).text(
                    rect.center(),
                    Align2::CENTER_CENTER,
                    self.name,
                    FontId::proportional(11.0),
                    Color32::from_gray(15),
                );
            });
        });
        let clicked = resp.response.interact(Sense::click()).clicked();
        (changed, clicked)
    }
}

fn pan_control(ui: &mut Ui, id: usize, pan: &mut f32) -> bool {
    let (rect, resp) = ui.allocate_exact_size(vec2(STRIP_W - 12.0, 16.0), Sense::click_and_drag());
    let resp = resp.on_hover_text("Panorama – Strg + Klick: Mitte");
    let mut changed = false;
    let command = ui.input(|i| i.modifiers.command);
    if resp.clicked() && command {
        *pan = 0.0;
        changed = true;
    } else if resp.dragged() {
        let fine = if ui.input(|i| i.modifiers.shift) {
            0.1
        } else {
            1.0
        };
        *pan = (*pan + resp.drag_delta().x / rect.width() * 2.0 * fine).clamp(-1.0, 1.0);
        changed = true;
    }
    let p = ui.painter();
    p.rect_filled(rect, 3.0, ui.visuals().extreme_bg_color);
    let cx = rect.center().x;
    let x = cx + *pan * rect.width() * 0.5;
    let bar = Rect::from_x_y_ranges(cx.min(x)..=cx.max(x).max(cx.min(x) + 1.0), rect.y_range());
    p.rect_filled(bar, 2.0, ui.visuals().selection.bg_fill);
    // Mittenmarke nur an den Rändern, damit sie die Beschriftung nicht kreuzt.
    let mark = Stroke::new(1.0, ui.visuals().weak_text_color());
    p.vline(cx, rect.top()..=rect.top() + 3.0, mark);
    p.vline(cx, rect.bottom() - 3.0..=rect.bottom(), mark);
    p.text(
        rect.center(),
        Align2::CENTER_CENTER,
        pan_label(*pan),
        FontId::monospace(10.0),
        ui.visuals().text_color(),
    );
    let value = f64::from(*pan);
    resp.widget_info(|| WidgetInfo::slider(true, value, format!("Panorama Kanal {id}")));
    changed
}

/// Fader mit Pegelanzeige daneben. Strg + Klick: 0 dB, Umschalt + Ziehen: fein.
fn fader(ui: &mut Ui, name: &str, db: &mut f32, meter: [f32; 2], height: f32) -> Response {
    let size = vec2(STRIP_W - 12.0, height);
    let (rect, mut resp) = ui.allocate_exact_size(size, Sense::click_and_drag());
    let track = Rect::from_min_max(
        pos2(rect.left() + 6.0, rect.top() + 6.0),
        pos2(rect.left() + 26.0, rect.bottom() - 6.0),
    );
    let to_y = |pos: f32| track.bottom() - pos * track.height();

    let command = ui.input(|i| i.modifiers.command);
    if resp.clicked() && command {
        *db = 0.0;
        resp.mark_changed();
    } else if resp.dragged() {
        let fine = if ui.input(|i| i.modifiers.shift) {
            0.1
        } else {
            1.0
        };
        let pos = db_to_pos(*db) - resp.drag_delta().y / track.height() * fine;
        *db = pos_to_db(pos);
        resp.mark_changed();
    }

    let p = ui.painter();
    let v = ui.visuals();
    // Skala
    for mark in [6.0, 0.0, -6.0, -12.0, -24.0, -48.0] {
        let y = to_y(db_to_pos(mark));
        p.hline(
            track.right() + 2.0..=track.right() + 6.0,
            y,
            Stroke::new(1.0, v.weak_text_color()),
        );
        p.text(
            pos2(rect.right() - 1.0, y),
            Align2::RIGHT_CENTER,
            format!("{}", mark as i32),
            FontId::proportional(8.5),
            v.weak_text_color(),
        );
    }
    // Pegel: zwei schmale Balken links neben der Skala
    for (c, level) in meter.into_iter().enumerate() {
        let x0 = track.left() + 13.0 + c as f32 * 6.0;
        let bar = Rect::from_x_y_ranges(x0..=x0 + 4.0, track.y_range());
        p.rect_filled(bar, 0.0, v.extreme_bg_color);
        let ldb = lin_to_db(level);
        if ldb > MIN_DB {
            let top = to_y(db_to_pos(ldb.min(MAX_DB)));
            let col = if ldb > 0.0 {
                crate::theme::tokens(ui.ctx()).meter_red
            } else if ldb > -6.0 {
                crate::theme::tokens(ui.ctx()).meter_yellow
            } else {
                crate::theme::tokens(ui.ctx()).meter_green
            };
            p.rect_filled(
                Rect::from_x_y_ranges(bar.x_range(), top..=bar.bottom()),
                0.0,
                col,
            );
        }
    }
    // Fader-Schiene und Knopf
    let rail = Rect::from_x_y_ranges(track.left() + 4.0..=track.left() + 7.0, track.y_range());
    p.rect_filled(rail, 1.0, v.extreme_bg_color);
    let ky = to_y(db_to_pos(*db));
    let knob = Rect::from_center_size(pos2(rail.center().x, ky), vec2(16.0, 10.0));
    let knob_fill = if resp.hovered() || resp.dragged() {
        v.widgets.hovered.bg_fill
    } else {
        v.widgets.inactive.bg_fill
    };
    p.rect_filled(knob, 2.0, knob_fill);
    p.rect_stroke(
        knob,
        2.0,
        Stroke::new(1.0, v.widgets.inactive.fg_stroke.color),
        StrokeKind::Inside,
    );
    p.hline(knob.x_range(), ky, Stroke::new(1.0, v.strong_text_color()));

    let value = f64::from(*db);
    let label = format!("Lautstärke {name}");
    resp.widget_info(|| WidgetInfo::slider(true, value, &label));
    resp.on_hover_text("Fader – Strg + Klick: 0 dB, Umschalt + Ziehen: fein")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fader_law_roundtrip_and_ends() {
        for db in [-48.0, -12.0, -6.0, 0.0, 3.0, 6.0] {
            assert!((pos_to_db(db_to_pos(db)) - db).abs() < 1e-3, "{db}");
        }
        assert_eq!(db_to_pos(MIN_DB), 0.0);
        assert_eq!(pos_to_db(0.0), MIN_DB);
        assert_eq!(db_to_pos(MAX_DB), 1.0);
        assert!(db_to_pos(0.0) > 0.8, "0 dB liegt im oberen Fünftel");
    }

    #[test]
    fn labels() {
        assert_eq!(db_label(MIN_DB), "−∞");
        assert_eq!(db_label(-3.04), "-3.0");
        assert_eq!(pan_label(0.0), "C");
        assert_eq!(pan_label(-0.3), "L30");
        assert_eq!(pan_label(1.0), "R100");
    }
}
