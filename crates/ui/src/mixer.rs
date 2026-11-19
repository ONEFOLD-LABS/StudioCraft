//! Mixer in der unteren Zone (Spec-Abschnitt „Mixer“): ein Kanalzug je Spur
//! mit Kanal, rechts der Ausgangsbus. F3 blendet ihn ein und aus.

use egui::{
    Align, Align2, Color32, FontId, Layout, Rect, Response, RichText, Sense, Stroke, StrokeKind,
    Ui, WidgetInfo, pos2, vec2,
};
use studiocraft_project::{MAX_DB, MIN_DB, TrackKind};

use crate::ProjectWindow;

const STRIP_W: f32 = 76.0;
/// So lange hält die Peak-Hold-Linie den Spitzenwert, in Sekunden.
const HOLD_SECONDS: f32 = 2.0;
/// Ab diesem Pegel (0 dBFS) leuchtet die Übersteuerungsanzeige.
const CLIP_LEVEL: f32 = 1.0;

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
    // Peak-Hold und Übersteuerung: Spuren, dahinter der Ausgangsbus.
    w.meter_hold.resize(n + 1, [(0.0, 0.0); 2]);
    w.clipped.resize(n + 1, false);
    let levels: Vec<[f32; 2]> = tracks
        .iter()
        .chain(std::iter::once(&master))
        .copied()
        .collect();
    update_hold(
        &mut w.meter_hold,
        &mut w.clipped,
        &levels,
        dt.clamp(0.0, 0.5),
        fall,
    );
    for (shown, new) in w.meter_levels.iter_mut().zip(tracks) {
        for c in 0..2 {
            shown[c] = new[c].max(shown[c] * fall);
        }
    }
    for c in 0..2 {
        w.master_meter[c] = master[c].max(w.master_meter[c] * fall);
    }
}

/// Peak-Hold-Werte des Kanals `k` (beide Seiten).
fn hold_of(hold: &[[(f32, f32); 2]], k: usize) -> [f32; 2] {
    hold.get(k).map_or([0.0; 2], |h| [h[0].0, h[1].0])
}

/// Peak-Hold und Übersteuerung nachführen: Ein neuer Spitzenwert wird gehalten und fällt erst
/// nach [`HOLD_SECONDS`]; ab [`CLIP_LEVEL`] gilt der Kanal als übersteuert (bis zum Klick).
fn update_hold(
    hold: &mut [[(f32, f32); 2]],
    clipped: &mut [bool],
    levels: &[[f32; 2]],
    dt: f32,
    fall: f32,
) {
    for (k, new) in levels.iter().enumerate() {
        let (Some(h), Some(clip)) = (hold.get_mut(k), clipped.get_mut(k)) else {
            break;
        };
        for c in 0..2 {
            let level = if new[c].is_finite() { new[c] } else { 0.0 };
            let (value, age) = &mut h[c];
            if level >= *value {
                *value = level;
                *age = 0.0;
            } else {
                *age += dt;
                if *age > HOLD_SECONDS {
                    *value *= fall;
                }
            }
            if level >= CLIP_LEVEL {
                *clip = true;
            }
        }
    }
}

pub(crate) fn show(w: &mut ProjectWindow, ui: &mut Ui) {
    let height = ui.available_height();
    let mut changed = false;
    let mut exclusive = None;
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
                            let hold = hold_of(&w.meter_hold, i);
                            let selected = w.selected_track == Some(i);
                            let t = &mut w.project.tracks[i];
                            let color = Color32::from_rgb(t.color[0], t.color[1], t.color[2]);
                            let (holds_events, audio) =
                                (t.kind.holds_events(), t.kind == TrackKind::Audio);
                            let strip = Strip {
                                id: i,
                                name: &t.name,
                                color,
                                selected,
                                volume_db: &mut t.volume_db,
                                pan: Some(&mut t.pan),
                                pan_mode: Some(&mut t.pan_mode),
                                mute: Some(&mut t.mute),
                                solo: Some(&mut t.solo),
                                solo_defeat: Some(&mut t.solo_defeat),
                                record: holds_events.then_some(&mut t.record),
                                monitor: audio.then_some(&mut t.monitor),
                                button_row: true,
                                meter,
                                hold,
                                clipped: w.clipped.get_mut(i),
                            };
                            let out = ui
                                .scope_builder(egui::UiBuilder::new().max_rect(rect), |ui| {
                                    strip.show(ui, height)
                                })
                                .inner;
                            changed |= out.changed;
                            if out.exclusive_solo {
                                exclusive = Some(i);
                            }
                            if out.clicked {
                                w.selected_track = Some(i);
                            }
                        }
                    });
            },
        );
        ui.separator();
        let n = w.project.tracks.len();
        let master = Strip {
            id: usize::MAX,
            name: "Ausgang",
            color: Color32::from_gray(150),
            selected: false,
            volume_db: &mut w.project.master_db,
            pan: None,
            pan_mode: None,
            mute: None,
            solo: None,
            solo_defeat: None,
            record: None,
            monitor: None,
            button_row: true,
            meter: w.master_meter,
            hold: hold_of(&w.meter_hold, n),
            clipped: w.clipped.get_mut(n),
        };
        changed |= master.show(ui, height).changed;
    });
    if let Some(i) = exclusive {
        w.project.solo_exclusive(i);
    }
    if changed {
        w.dirty = true;
    }
}

/// Breite der Kanalzug-Spalte in der linken Zone.
pub(crate) const COLUMN_W: f32 = STRIP_W + 8.0;

/// Kanalzug der gewählten Spur als eigene Spalte links neben dem Inspector
/// (docs/ui-parity.md, „Linke Zone“): oben Name in Spurfarbe, Inserts und Sends,
/// darunter Aufnahme, Monitor, R/W, dann Panorama, M/S und Fader mit Pegel.
pub(crate) fn channel_column(w: &mut ProjectWindow, ui: &mut Ui) {
    ui.set_width(COLUMN_W - 8.0);
    let Some(i) = w
        .selected_track
        .filter(|&i| w.project.tracks.get(i).is_some_and(|t| has_channel(t.kind)))
    else {
        ui.weak("Kein Kanal");
        return;
    };
    let sends: Vec<(String, bool)> = w.project.tracks[i]
        .sends
        .iter()
        .map(|s| (w.track_name(s.target), s.on))
        .collect();
    let meter = w.meter_levels.get(i).copied().unwrap_or_default();
    let t = crate::theme::tokens(ui.ctx());
    let track = &mut w.project.tracks[i];
    let color = Color32::from_rgb(track.color[0], track.color[1], track.color[2]);
    let mut changed = false;

    let (head, _) = ui.allocate_exact_size(vec2(COLUMN_W - 8.0, 20.0), Sense::hover());
    ui.painter().rect_filled(head, 3.0, color);
    ui.painter().with_clip_rect(head).text(
        head.left_center() + vec2(5.0, 0.0),
        Align2::LEFT_CENTER,
        &track.name,
        FontId::proportional(11.0),
        on_color(color),
    );
    let inserts: Vec<(String, bool)> = track
        .inserts
        .iter()
        .map(|s| (s.name.clone(), !s.bypass))
        .collect();
    slot_list(ui, "Inserts", &inserts);
    slot_list(ui, "Sends", &sends);
    ui.add_space(2.0);
    ui.horizontal(|ui| {
        ui.spacing_mut().item_spacing.x = 2.0;
        if track.kind.holds_events()
            && crate::icons::icon_button(
                ui,
                crate::icons::Icon::Record,
                track.record,
                Some(t.rec),
                "Aufnahme bereit",
            )
            .clicked()
        {
            track.record = !track.record;
        }
        if track.kind == TrackKind::Audio
            && crate::icons::icon_button(
                ui,
                crate::icons::Icon::Play,
                track.monitor,
                Some(t.monitor),
                "Monitor",
            )
            .clicked()
        {
            track.monitor = !track.monitor;
            changed = true;
        }
    });
    ui.horizontal(|ui| {
        ui.spacing_mut().item_spacing.x = 2.0;
        crate::icons::state_toggle(ui, &mut track.read, "R", t.monitor)
            .on_hover_text("Automation lesen");
        crate::icons::state_toggle(ui, &mut track.write, "W", t.rec)
            .on_hover_text("Automation schreiben");
    });
    let height = ui.available_height().max(160.0);
    let strip = Strip {
        id: i,
        name: &track.name,
        color,
        selected: false,
        volume_db: &mut track.volume_db,
        pan: Some(&mut track.pan),
        pan_mode: Some(&mut track.pan_mode),
        mute: Some(&mut track.mute),
        solo: Some(&mut track.solo),
        solo_defeat: Some(&mut track.solo_defeat),
        record: None,
        monitor: None,
        button_row: false,
        meter,
        hold: hold_of(&w.meter_hold, i),
        clipped: w.clipped.get_mut(i),
    };
    let out = ui
        .push_id("kanalzug-spalte", |ui| strip.show(ui, height))
        .inner;
    changed |= out.changed;
    if out.exclusive_solo {
        w.project.solo_exclusive(i);
    }
    if changed {
        w.dirty = true;
    }
}

/// Kurze Liste der Inserts bzw. Sends: Titel, dann je Slot eine Zeile (umgangen gedimmt).
fn slot_list(ui: &mut Ui, title: &str, slots: &[(String, bool)]) {
    ui.add_space(2.0);
    let color = crate::theme::tokens(ui.ctx()).text;
    ui.label(RichText::new(title).size(10.0).strong().color(color));
    if slots.is_empty() {
        ui.weak(RichText::new("leer").size(10.0));
    }
    for (name, active) in slots {
        let text = RichText::new(name).size(10.0);
        let text = if *active { text } else { text.weak() };
        ui.add(egui::Label::new(text).truncate());
    }
}

/// Lesbare Schriftfarbe auf einer Spurfarbe.
fn on_color(bg: Color32) -> Color32 {
    let l = 0.299 * f32::from(bg.r()) + 0.587 * f32::from(bg.g()) + 0.114 * f32::from(bg.b());
    if l > 140.0 {
        Color32::BLACK
    } else {
        Color32::WHITE
    }
}

struct Strip<'a> {
    id: usize,
    name: &'a str,
    color: Color32,
    selected: bool,
    volume_db: &'a mut f32,
    pan: Option<&'a mut f32>,
    /// Pan-Modus (Rechtsklick auf den Pan-Regler).
    pan_mode: Option<&'a mut studiocraft_project::PanMode>,
    mute: Option<&'a mut bool>,
    solo: Option<&'a mut bool>,
    /// Solo-Defeat (Strg+Klick auf Solo).
    solo_defeat: Option<&'a mut bool>,
    /// Aufnahme bereit und Monitor auf dem Kanalzug (Spuren, die es haben).
    record: Option<&'a mut bool>,
    monitor: Option<&'a mut bool>,
    /// Platz für die Knopfreihe auch ohne Knöpfe, damit die Fader im Mixer auf einer Höhe
    /// stehen.
    button_row: bool,
    meter: [f32; 2],
    /// Peak-Hold je Kanal (linear).
    hold: [f32; 2],
    /// Übersteuert seit dem letzten Zurücksetzen.
    clipped: Option<&'a mut bool>,
}

/// Ergebnis eines Kanalzugs.
#[derive(Debug, Clone, Copy, Default)]
struct StripOut {
    changed: bool,
    clicked: bool,
    /// Alt+Klick auf Solo: nur dieser Kanal soll solo sein.
    exclusive_solo: bool,
}

impl Strip<'_> {
    /// Zeichnet den Kanalzug.
    fn show(self, ui: &mut Ui, height: f32) -> StripOut {
        let mut changed = false;
        let mut exclusive_solo = false;
        // Klickfläche des ganzen Kanalzugs (Kanal wählen) zuerst anlegen: Was danach kommt,
        // liegt obenauf, so erreichen Klicks M, S, Pan und Fader und nur der Rest wählt.
        let area = egui::Rect::from_min_size(ui.cursor().min, vec2(STRIP_W, height));
        let select = ui.interact(
            area,
            ui.make_persistent_id(("kanalzug", self.id)),
            Sense::click(),
        );
        let fill = if self.selected {
            crate::theme::tokens(ui.ctx()).row_selected
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
                    let (c, resp) = pan_control(ui, self.id, pan);
                    changed |= c;
                    if let Some(mode) = self.pan_mode {
                        resp.context_menu(|ui| {
                            for m in [
                                studiocraft_project::PanMode::Balance,
                                studiocraft_project::PanMode::Combined,
                            ] {
                                if ui.radio(*mode == m, m.label()).clicked() {
                                    *mode = m;
                                    changed = true;
                                    ui.close();
                                }
                            }
                        });
                    }
                } else {
                    ui.add_space(18.0);
                }
                ui.horizontal(|ui| {
                    ui.spacing_mut().item_spacing.x = 2.0;
                    if let Some(m) = self.mute {
                        changed |= crate::icons::state_toggle(
                            ui,
                            m,
                            "M",
                            crate::theme::tokens(ui.ctx()).mute,
                        )
                        .on_hover_text("Mute")
                        .changed();
                    }
                    if let Some(s) = self.solo {
                        let defeat = self.solo_defeat;
                        let on_defeat = defeat.as_deref().copied().unwrap_or(false);
                        let r = solo_button(ui, *s, on_defeat);
                        if r.clicked() {
                            let mods = ui.input(|i| i.modifiers);
                            if mods.command {
                                if let Some(d) = defeat {
                                    *d = !*d;
                                }
                            } else if mods.alt {
                                *s = true;
                                exclusive_solo = true;
                            } else {
                                *s = !*s;
                            }
                            changed = true;
                        }
                    }
                });
                if self.record.is_none() && self.monitor.is_none() {
                    if self.button_row {
                        // Gleich hoch wie die Knopfreihe: eine Zeile mit leerem Platzhalter.
                        ui.horizontal(|ui| {
                            ui.allocate_exact_size(vec2(1.0, crate::icons::BUTTON), Sense::hover());
                        });
                    }
                } else {
                    ui.horizontal(|ui| {
                        ui.spacing_mut().item_spacing.x = 2.0;
                        let t = crate::theme::tokens(ui.ctx());
                        if let Some(r) = self.record
                            && crate::icons::icon_button(
                                ui,
                                crate::icons::Icon::Record,
                                *r,
                                Some(t.rec),
                                "Aufnahme bereit",
                            )
                            .clicked()
                        {
                            *r = !*r;
                            changed = true;
                        }
                        if let Some(m) = self.monitor
                            && crate::icons::icon_button(
                                ui,
                                crate::icons::Icon::Play,
                                *m,
                                Some(t.monitor),
                                "Monitor",
                            )
                            .clicked()
                        {
                            *m = !*m;
                            changed = true;
                        }
                    });
                }
                ui.label(
                    RichText::new(db_label(*self.volume_db))
                        .monospace()
                        .size(11.0),
                );
                let fader_h = (ui.available_height() - 24.0).max(60.0);
                changed |= fader(
                    ui,
                    self.name,
                    self.volume_db,
                    Meter {
                        level: self.meter,
                        hold: self.hold,
                        clipped: self.clipped,
                    },
                    fader_h,
                )
                .changed();
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
        let _ = resp;
        StripOut {
            changed,
            clicked: select.clicked(),
            exclusive_solo,
        }
    }
}

/// Solo-Knopf: gelb bei Solo; bei Solo-Defeat gelb umrandet mit gelber Schrift.
fn solo_button(ui: &mut Ui, on: bool, defeat: bool) -> Response {
    let t = crate::theme::tokens(ui.ctx());
    let text_color = if on {
        Color32::BLACK
    } else if defeat {
        t.solo
    } else {
        t.text
    };
    // Rahmen immer gleich breit (ohne Defeat unsichtbar), damit der Knopf gleich groß bleibt.
    let frame = if defeat { t.solo } else { Color32::TRANSPARENT };
    let b = egui::Button::new(RichText::new("S").color(text_color))
        .fill(if on { t.solo } else { t.button })
        .stroke(Stroke::new(1.5, frame))
        .min_size(vec2(20.0, 18.0));
    ui.add(b)
        .on_hover_text("Solo – Alt+Klick: exklusiv, Strg+Klick: Solo-Defeat")
}

/// Pegelanzeige eines Faders: Pegel, Peak-Hold und Übersteuerung.
struct Meter<'a> {
    level: [f32; 2],
    hold: [f32; 2],
    clipped: Option<&'a mut bool>,
}

fn pan_control(ui: &mut Ui, id: usize, pan: &mut f32) -> (bool, Response) {
    let (rect, resp) = ui.allocate_exact_size(vec2(STRIP_W - 12.0, 16.0), Sense::click_and_drag());
    let resp = resp.on_hover_text("Panorama – Strg + Klick: Mitte, Rechtsklick: Pan-Modus");
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
    (changed, resp)
}

/// Fader mit Pegelanzeige daneben. Strg + Klick: 0 dB, Umschalt + Ziehen: fein.
fn fader(ui: &mut Ui, name: &str, db: &mut f32, meter: Meter, height: f32) -> Response {
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
    // Übersteuerung: rotes Feld über den Pegelbalken, Klick setzt zurück.
    if let Some(clipped) = meter.clipped {
        let x0 = track.left() + 13.0;
        let r = Rect::from_x_y_ranges(x0..=x0 + 10.0, rect.top()..=rect.top() + 5.0);
        let id = ui.make_persistent_id(("uebersteuerung", name));
        let hit = ui.interact(r.expand(2.0), id, Sense::click());
        hit.widget_info(|| {
            WidgetInfo::labeled(egui::Role::Button, true, "Übersteuerung zurücksetzen")
        });
        if hit.clicked() {
            *clipped = false;
        }
        let fill = if *clipped {
            crate::theme::tokens(ui.ctx()).meter_red
        } else {
            ui.visuals().extreme_bg_color
        };
        ui.painter().rect_filled(r, 1.0, fill);
    }
    // Pegel: zwei schmale Balken links neben der Skala
    let p = ui.painter();
    for (c, level) in meter.level.into_iter().enumerate() {
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
        // Peak-Hold: dünne Linie am gehaltenen Spitzenwert.
        let hdb = lin_to_db(meter.hold[c]);
        if hdb > MIN_DB {
            let y = to_y(db_to_pos(hdb.min(MAX_DB)));
            let col = if hdb > 0.0 {
                crate::theme::tokens(ui.ctx()).meter_red
            } else {
                v.text_color()
            };
            p.hline(bar.x_range(), y, Stroke::new(1.5, col));
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
    p.hline(
        knob.x_range(),
        ky,
        Stroke::new(1.0, crate::theme::tokens(ui.ctx()).text),
    );

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
    fn peak_hold_holds_then_falls_and_clips_stay_until_reset() {
        let mut hold = vec![[(0.0, 0.0); 2]];
        let mut clip = vec![false];
        update_hold(&mut hold, &mut clip, &[[0.5, 1.2]], 0.1, 0.5);
        assert_eq!((hold[0][0].0, hold[0][1].0), (0.5, 1.2));
        assert!(clip[0], "1.2 is above 0 dBFS");
        // Quieter: the hold stays for HOLD_SECONDS, then falls.
        for _ in 0..15 {
            update_hold(&mut hold, &mut clip, &[[0.1, 0.1]], 0.1, 0.5);
        }
        assert_eq!(hold[0][0].0, 0.5, "still held after 1.5 s");
        for _ in 0..10 {
            update_hold(&mut hold, &mut clip, &[[0.1, 0.1]], 0.1, 0.5);
        }
        assert!(hold[0][0].0 < 0.5, "falls after 2 s");
        assert!(clip[0], "clip stays until reset");
        // NaN never becomes a hold value.
        update_hold(&mut hold, &mut clip, &[[f32::NAN, 0.0]], 0.1, 0.5);
        assert!(hold[0][0].0.is_finite());
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
