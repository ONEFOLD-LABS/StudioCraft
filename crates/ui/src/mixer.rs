//! Mixer in der unteren Zone (Spec-Abschnitt „Mixer“): ein Kanalzug je Spur
//! mit Kanal, rechts der Ausgangsbus. F3 blendet ihn ein und aus.

use crate::i18n::tr;
use egui::{
    Align, Align2, Color32, FontId, Layout, Rect, Response, RichText, Sense, Stroke, StrokeKind,
    Ui, WidgetInfo, pos2, vec2,
};
use studiocraft_project::{MAX_DB, MIN_DB, TrackKind};

use crate::ProjectWindow;

const STRIP_W: f32 = 76.0;
/// Breite eines schmalen Kanalzugs (ohne Panorama und Skala).
const NARROW_W: f32 = 48.0;
/// Höhe des Racks über den Fadern (Inserts, EQ, Sends).
const RACK_H: f32 = 132.0;
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
    update_bus_meters(w, fall);
}

/// Ein- und Ausgangsbusse: Pegel der Eingangskanalzüge an den Eingang geben, Spitzenwerte
/// abholen.
fn update_bus_meters(w: &mut ProjectWindow, fall: f32) {
    use studiocraft_engine::MAX_INPUT_CHANNELS;
    let io = &w.project.io;
    let fresh_out: Vec<[f32; 2]> = (0..io.outputs.len())
        .map(|k| {
            w.transport
                .as_ref()
                .map_or([0.0; 2], |t| t.meters.take_output(k))
        })
        .collect();
    let mut fresh_in = vec![[0.0f32; 2]; io.inputs.len()];
    if let Some(input) = &w.input {
        let levels = &input.levels;
        // Gerätekanäle ohne Eingangsbus bleiben bei 1.
        let mut gains = [1.0f32; MAX_INPUT_CHANNELS];
        let mut peaks = [0.0f32; MAX_INPUT_CHANNELS];
        for (c, p) in peaks.iter_mut().enumerate() {
            *p = levels.take(c);
        }
        for (b, fresh) in io.inputs.iter().zip(fresh_in.iter_mut()) {
            let first = usize::from(b.first);
            let right = if b.stereo { first + 1 } else { first };
            let gain = studiocraft_project::db_to_gain(b.gain_db);
            for ch in [first, right] {
                if let Some(g) = gains.get_mut(ch) {
                    *g = gain;
                }
            }
            *fresh = [
                peaks.get(first).copied().unwrap_or(0.0),
                peaks.get(right).copied().unwrap_or(0.0),
            ];
        }
        for (c, &g) in gains.iter().enumerate() {
            if levels.gain(c) != g {
                levels.set_gain(c, g);
            }
        }
    }
    for (shown, fresh) in [
        (&mut w.input_meters, fresh_in),
        (&mut w.output_meters, fresh_out),
    ] {
        shown.resize(fresh.len(), [0.0; 2]);
        for (s, f) in shown.iter_mut().zip(fresh) {
            for c in 0..2 {
                let f = if f[c].is_finite() { f[c] } else { 0.0 };
                s[c] = f.max(s[c] * fall);
            }
        }
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
    let mut changed = header(w, ui);
    let mut exclusive = None;
    let height = ui.available_height();
    ui.horizontal(|ui| {
        if w.mixer_list {
            changed |= channel_list(w, ui, height);
            ui.separator();
        }
        let gap = ui.spacing().item_spacing.x;
        if w.mixer_io && !w.project.io.inputs.is_empty() {
            changed |= io_strips(w, ui, height, false);
            ui.separator();
        }
        let extra_outputs = if w.mixer_io {
            w.project.io.outputs.len().saturating_sub(1)
        } else {
            0
        };
        let master_w = (STRIP_W + gap) * (1 + extra_outputs) as f32 + 16.0;
        let tracks_w = (ui.available_width() - master_w).max(NARROW_W);
        ui.allocate_ui_with_layout(
            vec2(tracks_w, height),
            Layout::left_to_right(Align::Min),
            |ui| {
                // Nur sichtbare Kanalzüge bauen: auch bei hunderten Spuren bleibt
                // der Aufwand pro Frame klein. Breiten sind verschieden (schmal/breit),
                // deshalb über aufaddierte Positionen.
                let gap = ui.spacing().item_spacing.x;
                let channels: Vec<(usize, f32)> = (0..w.project.tracks.len())
                    .filter(|&i| {
                        let t = &w.project.tracks[i];
                        has_channel(t.kind) && !t.mixer_hidden
                    })
                    .map(|i| {
                        let narrow = w.project.tracks[i].mixer_narrow;
                        (i, if narrow { NARROW_W } else { STRIP_W })
                    })
                    .collect();
                let mut xs = Vec::with_capacity(channels.len() + 1);
                let mut x = 0.0;
                for &(_, width) in &channels {
                    xs.push(x);
                    x += width + gap;
                }
                xs.push(x);
                let total = x;
                egui::ScrollArea::horizontal()
                    .id_salt("mixer-kanaele")
                    .show_viewport(ui, |ui, viewport| {
                        let origin = ui.max_rect().min;
                        ui.set_min_size(vec2(total, height));
                        let first = xs
                            .partition_point(|&x| x <= viewport.min.x)
                            .saturating_sub(1);
                        let last = xs
                            .partition_point(|&x| x < viewport.max.x)
                            .min(channels.len());
                        for k in first..last {
                            let (i, width) = channels[k];
                            let rect =
                                Rect::from_min_size(origin + vec2(xs[k], 0.0), vec2(width, height));
                            let names: Vec<String> = w.project.tracks[i]
                                .sends
                                .iter()
                                .map(|s| w.track_name(s.target))
                                .collect();
                            let meter = w.meter_levels.get(i).copied().unwrap_or_default();
                            let hold = hold_of(&w.meter_hold, i);
                            let selected = w.selected_track == Some(i);
                            let show_rack = w.mixer_rack;
                            let t = &mut w.project.tracks[i];
                            let color = Color32::from_rgb(t.color[0], t.color[1], t.color[2]);
                            let (holds_events, audio) =
                                (t.kind.holds_events(), t.kind == TrackKind::Audio);
                            let wide = width >= STRIP_W;
                            let rack = (show_rack && wide).then(|| Rack {
                                inserts: &mut t.inserts,
                                eq_on: &mut t.eq.on,
                                sends: &mut t.sends,
                                send_names: names,
                            });
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
                                width,
                                rack,
                                rack_space: show_rack,
                                meter,
                                hold,
                                clipped: w.clipped.get_mut(i),
                                link_group: Some(&mut t.link_group),
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
        if extra_outputs > 0 {
            changed |= io_strips(w, ui, height, true);
        }
        let n = w.project.tracks.len();
        let main_name = w
            .project
            .io
            .outputs
            .first()
            .map_or_else(|| tr("Ausgang").to_owned(), |b| b.name.clone());
        let master = Strip {
            id: usize::MAX,
            name: &main_name,
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
            width: STRIP_W,
            rack: None,
            rack_space: w.mixer_rack,
            meter: w.master_meter,
            hold: hold_of(&w.meter_hold, n),
            clipped: w.clipped.get_mut(n),
            link_group: None,
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

/// Kanalzüge der Eingangsbusse (`outputs == false`) oder der weiteren Ausgangsbusse (ohne
/// den Hauptmix, der hat den Ausgangsfader des Projekts): Pegel, bei Ausgängen Mute.
fn io_strips(w: &mut ProjectWindow, ui: &mut Ui, height: f32, outputs: bool) -> bool {
    let mut changed = false;
    let rack_space = w.mixer_rack;
    let (buses, meters, skip) = if outputs {
        (&mut w.project.io.outputs, &w.output_meters, 1)
    } else {
        (&mut w.project.io.inputs, &w.input_meters, 0)
    };
    let tokens = crate::theme::tokens(ui.ctx());
    for (k, bus) in buses.iter_mut().enumerate().skip(skip) {
        let meter = meters.get(k).copied().unwrap_or_default();
        // Eigene Id-Bereiche unterhalb des Hauptmixes (usize::MAX).
        let id = if outputs {
            usize::MAX - 1 - k
        } else {
            usize::MAX - 1 - studiocraft_project::MAX_IO_BUSES - k
        };
        let name = bus.name.clone();
        let strip = Strip {
            id,
            name: &name,
            color: if outputs {
                tokens.bus_output
            } else {
                tokens.bus_input
            },
            selected: false,
            volume_db: &mut bus.gain_db,
            pan: None,
            pan_mode: None,
            mute: outputs.then_some(&mut bus.mute),
            solo: None,
            solo_defeat: None,
            record: None,
            monitor: None,
            button_row: true,
            width: STRIP_W,
            rack: None,
            rack_space,
            meter,
            hold: meter,
            clipped: None,
            link_group: None,
        };
        changed |= strip.show(ui, height).changed;
    }
    changed
}

/// Kopfzeile des Mixers: Kanalliste, Rack, Breite aller Kanäle, gespeicherte Ansichten.
fn header(w: &mut ProjectWindow, ui: &mut Ui) -> bool {
    let mut changed = false;
    ui.horizontal(|ui| {
        ui.toggle_value(&mut w.mixer_list, tr("Kanalliste"))
            .on_hover_text(tr("Liste aller Kanäle mit Sichtbarkeit"));
        ui.toggle_value(&mut w.mixer_rack, tr("Rack"))
            .on_hover_text(tr("Inserts, EQ und Sends über den Fadern"));
        ui.toggle_value(&mut w.mixer_io, tr("Ein-/Ausgänge"))
            .on_hover_text(tr("Eingangskanäle links, Ausgangsbusse rechts"));
        ui.separator();
        for (label, narrow) in [(tr("Alle breit"), false), (tr("Alle schmal"), true)] {
            if ui.button(label).clicked() {
                for t in &mut w.project.tracks {
                    t.mixer_narrow = narrow;
                }
                changed = true;
            }
        }
        ui.separator();
        ui.menu_button(tr("Ansichten"), |ui| {
            ui.horizontal(|ui| {
                let l = ui.label(tr("Name"));
                ui.add(egui::TextEdit::singleline(&mut w.mixer_view_name).desired_width(120.0))
                    .labelled_by(l.id);
                if ui.button(tr("Speichern")).clicked() {
                    let name = std::mem::take(&mut w.mixer_view_name);
                    if !w.project.save_mixer_view(&name) {
                        w.message = Some(crate::i18n::trf(
                            "Ansicht nicht gespeichert (Name leer oder schon {max} Ansichten)",
                            &[("max", &studiocraft_project::MAX_MIXER_VIEWS)],
                        ));
                    }
                }
            });
            let mut apply = None;
            let mut remove = None;
            for (k, v) in w.project.mixer_views.iter().enumerate() {
                ui.horizontal(|ui| {
                    if ui.button(&v.name).clicked() {
                        apply = Some(k);
                    }
                    if ui
                        .small_button(tr("x"))
                        .on_hover_text(crate::i18n::trf(
                            "Ansicht „{name}“ entfernen",
                            &[("name", &v.name)],
                        ))
                        .clicked()
                    {
                        remove = Some(k);
                    }
                });
            }
            if let Some(k) = apply {
                w.project.apply_mixer_view(k);
                changed = true;
                ui.close();
            }
            if let Some(k) = remove {
                w.project.mixer_views.remove(k);
            }
        });
    });
    changed
}

/// Kanalliste links im Mixer: alle Kanäle mit Sichtbarkeit und Breite.
fn channel_list(w: &mut ProjectWindow, ui: &mut Ui, height: f32) -> bool {
    let mut changed = false;
    ui.allocate_ui_with_layout(vec2(160.0, height), Layout::top_down(Align::Min), |ui| {
        ui.set_min_width(160.0);
        egui::ScrollArea::vertical()
            .id_salt("mixer-kanalliste")
            .show(ui, |ui| {
                for t in w.project.tracks.iter_mut().filter(|t| has_channel(t.kind)) {
                    ui.horizontal(|ui| {
                        let mut visible = !t.mixer_hidden;
                        if ui.checkbox(&mut visible, &t.name).changed() {
                            t.mixer_hidden = !visible;
                            changed = true;
                        }
                        let mut narrow = t.mixer_narrow;
                        if ui
                            .toggle_value(&mut narrow, tr("schmal"))
                            .on_hover_text(crate::i18n::trf(
                                "„{name}“ schmal anzeigen",
                                &[("name", &t.name)],
                            ))
                            .changed()
                        {
                            t.mixer_narrow = narrow;
                            changed = true;
                        }
                    });
                }
            });
    });
    changed
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
        ui.weak(tr("Kein Kanal"));
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
    slot_list(ui, tr("Inserts"), &inserts);
    slot_list(ui, tr("Sends"), &sends);
    ui.add_space(2.0);
    ui.horizontal(|ui| {
        ui.spacing_mut().item_spacing.x = 2.0;
        if track.kind.holds_events()
            && crate::icons::icon_button(
                ui,
                crate::icons::Icon::Record,
                track.record,
                Some(t.rec),
                tr("Aufnahme bereit"),
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
                tr("Monitor"),
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
            .on_hover_text(tr("Automation lesen"));
        crate::icons::state_toggle(ui, &mut track.write, "W", t.rec)
            .on_hover_text(tr("Automation schreiben"));
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
        width: STRIP_W,
        rack: None,
        rack_space: false,
        meter,
        hold: hold_of(&w.meter_hold, i),
        clipped: w.clipped.get_mut(i),
        link_group: Some(&mut track.link_group),
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
        ui.weak(RichText::new(tr("leer")).size(10.0));
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
    /// Breite des Kanalzugs (schmal: ohne Panorama und Skala).
    width: f32,
    /// Rack über den Fadern; `rack_space`: Platz dafür auch ohne Rack (Ausrichtung).
    rack: Option<Rack<'a>>,
    rack_space: bool,
    meter: [f32; 2],
    /// Peak-Hold je Kanal (linear).
    hold: [f32; 2],
    /// Übersteuert seit dem letzten Zurücksetzen.
    clipped: Option<&'a mut bool>,
    /// Link-Gruppe (Spuren; Rechtsklick auf das Namensfeld).
    link_group: Option<&'a mut Option<u8>>,
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
        let mut clicked = false;
        // Klickfläche des ganzen Kanalzugs (Kanal wählen) zuerst anlegen: Was danach kommt,
        // liegt obenauf, so erreichen Klicks M, S, Pan und Fader und nur der Rest wählt.
        let area = egui::Rect::from_min_size(ui.cursor().min, vec2(self.width, height));
        let narrow = self.width < STRIP_W;
        let select = ui.interact(
            area,
            ui.make_persistent_id(("kanalzug", self.id)),
            Sense::click(),
        );
        let label = crate::i18n::trf("Kanal {name} wählen", &[("name", &self.name)]);
        select.widget_info(|| {
            egui::WidgetInfo::selected(egui::Role::Button, true, self.selected, &label)
        });
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
            ui.set_width(self.width - 8.0);
            ui.set_height(height - 8.0);
            ui.with_layout(Layout::top_down(Align::Center), |ui| {
                ui.spacing_mut().item_spacing.y = 4.0;
                if let Some(rack) = self.rack {
                    changed |= rack_ui(ui, rack, self.id, self.width - 8.0);
                } else if self.rack_space {
                    ui.add_space(RACK_H + 4.0);
                }
                if narrow {
                    ui.add_space(18.0);
                } else if let Some(pan) = self.pan {
                    let (c, resp) = pan_control(ui, self.id, pan);
                    changed |= c;
                    if let Some(mode) = self.pan_mode {
                        resp.context_menu(|ui| {
                            for m in [
                                studiocraft_project::PanMode::Balance,
                                studiocraft_project::PanMode::Combined,
                            ] {
                                if ui.radio(*mode == m, tr(m.label())).clicked() {
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
                    if self.button_row {
                        // Gleiche Höhe in jedem Kanalzug, mit und ohne M/S.
                        ui.allocate_exact_size(vec2(1.0, crate::icons::BUTTON), Sense::hover());
                    }
                    if let Some(m) = self.mute {
                        changed |= crate::icons::state_toggle(
                            ui,
                            m,
                            tr("M"),
                            crate::theme::tokens(ui.ctx()).mute,
                        )
                        .on_hover_text(tr("Mute"))
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
                                tr("Aufnahme bereit"),
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
                                tr("Monitor"),
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
                    self.width,
                    self.volume_db,
                    Meter {
                        level: self.meter,
                        hold: self.hold,
                        clipped: self.clipped,
                    },
                    fader_h,
                )
                .changed();
                let (rect, plate) =
                    ui.allocate_exact_size(vec2(self.width - 8.0, 18.0), Sense::click());
                ui.painter().rect_filled(rect, 2.0, self.color);
                let painter = ui.painter().with_clip_rect(rect);
                // Mit Gruppenkennzeichen bekommt der Name nur den Platz links davon.
                let linked = self.link_group.as_deref().is_some_and(Option::is_some);
                let name_rect = if linked {
                    Rect::from_min_max(rect.min, rect.max - vec2(18.0, 0.0))
                } else {
                    rect
                };
                painter.with_clip_rect(name_rect).text(
                    name_rect.center(),
                    Align2::CENTER_CENTER,
                    self.name,
                    FontId::proportional(11.0),
                    Color32::from_gray(15),
                );
                if let Some(link) = self.link_group {
                    if let Some(g) = *link {
                        // Kennzeichen der Gruppe rechts im Namensfeld.
                        let t = crate::theme::tokens(ui.ctx());
                        let badge = Rect::from_min_size(
                            rect.right_top() - vec2(16.0, -2.0),
                            vec2(14.0, rect.height() - 4.0),
                        );
                        painter.rect_filled(badge, 2.0, t.button);
                        painter.text(
                            badge.center(),
                            Align2::CENTER_CENTER,
                            g.to_string(),
                            FontId::proportional(10.0),
                            t.text,
                        );
                    }
                    let plate = plate.on_hover_text(match *link {
                        Some(g) => crate::i18n::trf(
                            "Link-Gruppe {nr} – Rechtsklick: ändern",
                            &[("nr", &g)],
                        ),
                        None => tr("Rechtsklick: Link-Gruppe").to_owned(),
                    });
                    let label = crate::i18n::trf("Link-Gruppe von {name}", &[("name", &self.name)]);
                    plate.widget_info(|| {
                        egui::WidgetInfo::labeled(egui::Role::Button, true, label.clone())
                    });
                    clicked |= plate.clicked();
                    plate.context_menu(|ui| {
                        ui.label(RichText::new(tr("Link-Gruppe")).strong());
                        if ui.radio(link.is_none(), tr("Keine")).clicked() {
                            *link = None;
                            changed = true;
                            ui.close();
                        }
                        for g in 1..=studiocraft_project::MAX_LINK_GROUPS {
                            let label = crate::i18n::trf("Gruppe {nr}", &[("nr", &g)]);
                            if ui.radio(*link == Some(g), label).clicked() {
                                *link = Some(g);
                                changed = true;
                                ui.close();
                            }
                        }
                    });
                } else {
                    clicked |= plate.clicked();
                }
            });
        });
        let _ = resp;
        StripOut {
            changed,
            clicked: clicked || select.clicked(),
            exclusive_solo,
        }
    }
}

/// Rack eines Kanalzugs (Spec „Sichtbarkeit und Ansicht“): Inserts (Klick: an/umgehen),
/// EQ an/aus, Sends (an/aus und Pegel).
struct Rack<'a> {
    inserts: &'a mut [studiocraft_project::InsertSlot],
    eq_on: &'a mut bool,
    sends: &'a mut [studiocraft_project::SendSlot],
    /// Namen der Send-Ziele, wie `sends`.
    send_names: Vec<String>,
}

/// Kurzname für die schmale Rack-Spalte.
fn short_name(s: &str) -> String {
    let n: String = s.chars().take(9).collect();
    if s.chars().count() > 9 {
        format!("{n}…")
    } else {
        n
    }
}

fn rack_ui(ui: &mut Ui, rack: Rack, id: usize, width: f32) -> bool {
    let mut changed = false;
    ui.allocate_ui(vec2(width, RACK_H), |ui| {
        ui.set_min_size(vec2(width, RACK_H));
        ui.set_max_height(RACK_H);
        ui.spacing_mut().item_spacing.y = 1.0;
        let small = |s: &str| RichText::new(s).size(10.0);
        ui.weak(small(tr("Inserts")));
        if rack.inserts.is_empty() {
            ui.weak(small("–"));
        }
        for (k, slot) in rack.inserts.iter_mut().enumerate() {
            let mut on = !slot.bypass;
            if ui
                .toggle_value(&mut on, small(&short_name(&slot.name)))
                .on_hover_text(crate::i18n::trf(
                    "Insert {nr}: {name} – Klick: an/umgehen",
                    &[("nr", &(k + 1)), ("name", &slot.name)],
                ))
                .changed()
            {
                slot.bypass = !on;
                changed = true;
            }
        }
        changed |= ui
            .toggle_value(rack.eq_on, small(tr("EQ")))
            .on_hover_text(tr("Kanal-EQ an/aus"))
            .changed();
        ui.weak(small(tr("Sends")));
        if rack.sends.is_empty() {
            ui.weak(small("–"));
        }
        for (k, (s, name)) in rack.sends.iter_mut().zip(&rack.send_names).enumerate() {
            ui.horizontal(|ui| {
                ui.spacing_mut().item_spacing.x = 2.0;
                let l = ui
                    .toggle_value(&mut s.on, small(&short_name(name)))
                    .on_hover_text(crate::i18n::trf(
                        "Send {nr} an {name}: an/aus",
                        &[("nr", &(k + 1)), ("name", &name)],
                    ));
                changed |= l.changed();
                changed |= ui
                    .add(
                        egui::DragValue::new(&mut s.level_db)
                            .range(MIN_DB..=MAX_DB)
                            .speed(0.2)
                            .max_decimals(1),
                    )
                    .labelled_by(l.id)
                    .changed();
            });
        }
        let _ = id;
    });
    changed
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
    let b = egui::Button::new(RichText::new(tr("S")).color(text_color))
        .fill(if on { t.solo } else { t.button })
        .stroke(Stroke::new(1.5, frame))
        .min_size(vec2(20.0, 18.0));
    ui.add(b)
        .on_hover_text(tr("Solo – Alt+Klick: exklusiv, Strg+Klick: Solo-Defeat"))
}

/// Pegelanzeige eines Faders: Pegel, Peak-Hold und Übersteuerung.
struct Meter<'a> {
    level: [f32; 2],
    hold: [f32; 2],
    clipped: Option<&'a mut bool>,
}

fn pan_control(ui: &mut Ui, id: usize, pan: &mut f32) -> (bool, Response) {
    let (rect, resp) = ui.allocate_exact_size(vec2(STRIP_W - 12.0, 16.0), Sense::click_and_drag());
    let resp = resp.on_hover_text(tr("Panorama – Strg + Klick: Mitte, Rechtsklick: Pan-Modus"));
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
    resp.widget_info(|| {
        WidgetInfo::slider(
            true,
            value,
            crate::i18n::trf("Panorama Kanal {id}", &[("id", &id)]),
        )
    });
    (changed, resp)
}

/// Fader mit Pegelanzeige daneben. Strg + Klick: 0 dB, Umschalt + Ziehen: fein.
fn fader(ui: &mut Ui, name: &str, width: f32, db: &mut f32, meter: Meter, height: f32) -> Response {
    let size = vec2(width - 12.0, height);
    let scale_labels = width >= STRIP_W;
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
        if scale_labels {
            p.text(
                pos2(rect.right() - 1.0, y),
                Align2::RIGHT_CENTER,
                format!("{}", mark as i32),
                FontId::proportional(8.5),
                v.weak_text_color(),
            );
        }
    }
    // Übersteuerung: rotes Feld über den Pegelbalken, Klick setzt zurück.
    if let Some(clipped) = meter.clipped {
        let x0 = track.left() + 13.0;
        let r = Rect::from_x_y_ranges(x0..=x0 + 10.0, rect.top()..=rect.top() + 5.0);
        let id = ui.make_persistent_id(("uebersteuerung", name));
        let hit = ui.interact(r.expand(2.0), id, Sense::click());
        hit.widget_info(|| {
            WidgetInfo::labeled(egui::Role::Button, true, tr("Übersteuerung zurücksetzen"))
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
    // Kante der Schiene, damit sie sich vom Kanalzug abhebt (3:1).
    p.rect_stroke(
        rail.expand(1.0),
        1.0,
        Stroke::new(1.0, crate::theme::tokens(ui.ctx()).fader_track),
        StrokeKind::Inside,
    );
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
    let label = crate::i18n::trf("Lautstärke {name}", &[("name", &name)]);
    resp.widget_info(|| WidgetInfo::slider(true, value, &label));
    resp.on_hover_text(tr("Fader – Strg + Klick: 0 dB, Umschalt + Ziehen: fein"))
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
