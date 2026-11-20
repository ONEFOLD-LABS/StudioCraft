//! Eingang und Pre-Sektion eines Kanalzugs im Inspector (Spec „Mixer → Signalfluss“,
//! Punkte 1 und 2; Issue #19): Eingangsgain, Phase umkehren, Hoch- und Tiefpass.

use crate::i18n::tr;
use egui::Ui;
use studiocraft_project::model::{INPUT_GAIN_DB, PRE_FILTER_HZ};

use crate::ProjectWindow;

impl ProjectWindow {
    pub(crate) fn pre_ui(&mut self, ui: &mut Ui, idx: usize) {
        let Some(t) = self.project.tracks.get_mut(idx) else {
            return;
        };
        let mut changed = false;
        ui.horizontal(|ui| {
            let l = ui.label(tr("Gain"));
            changed |= ui
                .add(
                    egui::Slider::new(&mut t.input_gain_db, INPUT_GAIN_DB.0..=INPUT_GAIN_DB.1)
                        .suffix(" dB")
                        .max_decimals(1),
                )
                .labelled_by(l.id)
                .changed();
        });
        changed |= ui
            .checkbox(&mut t.phase_invert, tr("Phase umkehren"))
            .changed();
        for (label, filter, default) in [
            (tr("Hochpass"), &mut t.pre_hpf, 80.0),
            (tr("Tiefpass"), &mut t.pre_lpf, 12_000.0),
        ] {
            ui.horizontal(|ui| {
                let mut on = filter.is_some();
                if ui.checkbox(&mut on, label).changed() {
                    *filter = on.then_some(default);
                    changed = true;
                }
                let mut hz = filter.unwrap_or(default);
                let speed = f64::from(hz) * 0.01;
                let l = ui.weak(tr("Hz"));
                let r = ui
                    .add_enabled(
                        on,
                        egui::DragValue::new(&mut hz)
                            .range(PRE_FILTER_HZ.0..=PRE_FILTER_HZ.1)
                            .speed(speed)
                            .max_decimals(0),
                    )
                    .labelled_by(l.id);
                if r.changed() && on {
                    *filter = Some(hz);
                    changed = true;
                }
            });
        }
        self.dirty |= changed;
    }
}
