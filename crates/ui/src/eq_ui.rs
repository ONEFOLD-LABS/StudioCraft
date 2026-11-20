//! Abschnitt „EQ“ im Inspector (Spec „Mixer“ → Signalfluss, Punkt 4; Issue #20): Kurve des
//! Frequenzgangs und vier Bänder mit Form, Frequenz, Gain und Güte, je ein- und abschaltbar.

use crate::i18n::tr;
use egui::{Align2, Color32, DragValue, FontId, Sense, Stroke, Ui, pos2, vec2};
use studiocraft_engine::eq::response_db;
use studiocraft_project::{BandKind, ChannelEq, EQ_FREQ, EQ_GAIN, EQ_Q};

use crate::ProjectWindow;

/// Sichtbarer Gain-Bereich der Kurve.
const VIEW_DB: f32 = 18.0;

fn freq_x(f: f32, left: f32, width: f32) -> f32 {
    let (lo, hi) = (EQ_FREQ.0.ln(), EQ_FREQ.1.ln());
    left + (f.max(EQ_FREQ.0).ln() - lo) / (hi - lo) * width
}

fn x_freq(x: f32, left: f32, width: f32) -> f32 {
    let (lo, hi) = (EQ_FREQ.0.ln(), EQ_FREQ.1.ln());
    (lo + (x - left) / width * (hi - lo)).exp()
}

/// Kurve des EQs mit Rasterlinien und den Bändern als Punkte.
fn curve(ui: &mut Ui, eq: &ChannelEq, rate: u32) {
    let t = crate::theme::tokens(ui.ctx());
    let (rect, _) = ui.allocate_exact_size(vec2(ui.available_width(), 90.0), Sense::hover());
    let p = ui.painter_at(rect);
    p.rect_filled(rect, 3.0, t.lane_bg);
    let y_of =
        |db: f32| rect.center().y - db.clamp(-VIEW_DB, VIEW_DB) / VIEW_DB * rect.height() / 2.0;
    for db in [-12.0, 0.0, 12.0] {
        let c = if db == 0.0 { t.text_dim } else { t.lane_alt };
        p.hline(rect.x_range(), y_of(db), Stroke::new(1.0, c));
    }
    for f in [100.0, 1000.0, 10_000.0] {
        let x = freq_x(f, rect.left(), rect.width());
        p.vline(x, rect.y_range(), Stroke::new(1.0, t.lane_alt));
        let label = if f >= 1000.0 {
            format!("{}k", f / 1000.0)
        } else {
            format!("{f}")
        };
        p.text(
            pos2(x + 2.0, rect.bottom() - 2.0),
            Align2::LEFT_BOTTOM,
            label,
            FontId::proportional(9.0),
            t.text_dim,
        );
    }
    let color = if eq.on { t.accent } else { t.text_dim };
    let n = (rect.width() as usize / 2).max(2);
    let line: Vec<_> = (0..=n)
        .map(|i| {
            let x = rect.left() + rect.width() * i as f32 / n as f32;
            let db = if eq.on {
                response_db(eq, x_freq(x, rect.left(), rect.width()), rate)
            } else {
                0.0
            };
            pos2(x, y_of(db))
        })
        .collect();
    p.line(line, Stroke::new(1.5, color));
    for (i, b) in eq.bands.iter().enumerate() {
        if !b.on {
            continue;
        }
        let gain = if b.kind.uses_gain() { b.gain } else { 0.0 };
        let c = pos2(freq_x(b.freq, rect.left(), rect.width()), y_of(gain));
        p.circle_filled(c, 6.0, color.gamma_multiply(0.8));
        p.text(
            c,
            Align2::CENTER_CENTER,
            (i + 1).to_string(),
            FontId::proportional(9.0),
            Color32::BLACK,
        );
    }
}

impl ProjectWindow {
    pub(crate) fn eq_ui(&mut self, ui: &mut Ui, idx: usize) {
        let rate = self.sample_rate;
        let Some(track) = self.project.tracks.get_mut(idx) else {
            return;
        };
        let mut eq = track.eq;
        ui.horizontal(|ui| {
            ui.checkbox(&mut eq.on, tr("EQ an"));
            if ui.small_button(tr("Zurücksetzen")).clicked() {
                eq = ChannelEq::default();
            }
        });
        curve(ui, &eq, rate);
        for (i, band) in eq.bands.iter_mut().enumerate() {
            ui.horizontal(|ui| {
                ui.checkbox(&mut band.on, format!("{}", i + 1))
                    .on_hover_text(tr("Band ein/aus"));
                let l = ui.weak(tr("Form"));
                egui::ComboBox::from_id_salt(("eq-form", idx, i))
                    .selected_text(tr(band.kind.label()))
                    .width(90.0)
                    .show_ui(ui, |ui| {
                        for k in BandKind::ALL {
                            ui.selectable_value(&mut band.kind, k, tr(k.label()));
                        }
                    })
                    .response
                    .labelled_by(l.id);
            });
            ui.horizontal(|ui| {
                ui.add_space(22.0);
                let speed = f64::from(band.freq) * 0.01;
                let l = ui.weak(tr("Hz"));
                ui.add(
                    DragValue::new(&mut band.freq)
                        .range(EQ_FREQ.0..=EQ_FREQ.1)
                        .speed(speed)
                        .max_decimals(0),
                )
                .labelled_by(l.id);
                let l = ui.weak(tr("dB"));
                ui.add_enabled(
                    band.kind.uses_gain(),
                    DragValue::new(&mut band.gain)
                        .range(EQ_GAIN.0..=EQ_GAIN.1)
                        .speed(0.1)
                        .max_decimals(1),
                )
                .labelled_by(l.id);
                let l = ui.weak(tr("Q"));
                ui.add(
                    DragValue::new(&mut band.q)
                        .range(EQ_Q.0..=EQ_Q.1)
                        .speed(0.01)
                        .max_decimals(2),
                )
                .labelled_by(l.id);
            });
        }
        if let Some(track) = self.project.tracks.get_mut(idx)
            && track.eq != eq
        {
            track.eq = eq;
            self.dirty = true;
        }
    }
}
