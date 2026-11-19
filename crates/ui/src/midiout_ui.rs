//! MIDI-Spuren zu externen Geräten im Inspector (Issue #12): Ausgang (MIDI-Port), Kanal und
//! Programm. Das Programm geht beim Wählen und bei jedem Start der Wiedergabe an das Gerät.

use egui::Ui;

use crate::ProjectWindow;

impl ProjectWindow {
    /// Programmwechsel aller MIDI-Spuren mit Ausgang und Programm senden (beim Start).
    pub(crate) fn send_programs(&self) {
        let Some(out) = &self.midi_out else {
            return;
        };
        for t in &self.project.tracks {
            if let (Some(port), Some(prog)) = (&t.midi_port, t.program) {
                let ch = t.midi_channel.clamp(1, 16) - 1;
                out.send_now(port, [0xc0 | ch, prog.min(127), 0]);
            }
        }
    }

    /// Sektion „MIDI-Ausgang“ einer MIDI-Spur.
    pub(crate) fn midi_out_ui(&mut self, ui: &mut Ui, idx: usize) {
        let Some(track) = self.project.tracks.get(idx) else {
            return;
        };
        let (port, channel, program) = (track.midi_port.clone(), track.midi_channel, track.program);
        let ports = self
            .midi_out_ports
            .get_or_insert_with(studiocraft_engine::midi_out::output_ports)
            .clone();
        let mut new_port = port.clone();
        let mut new_channel = channel;
        let mut new_program = program;
        ui.horizontal(|ui| {
            let l = ui.label("Ausgang");
            egui::ComboBox::from_id_salt(("midi-ausgang", idx))
                .width(120.0)
                .truncate()
                .selected_text(port.clone().unwrap_or_else(|| "Kein Ausgang".into()))
                .show_ui(ui, |ui| {
                    ui.selectable_value(&mut new_port, None, "Kein Ausgang");
                    for p in &ports {
                        ui.selectable_value(&mut new_port, Some(p.clone()), p);
                    }
                    // Ein gespeicherter Port, der gerade fehlt, bleibt wählbar.
                    if let Some(p) = port.as_ref().filter(|p| !ports.contains(p)) {
                        ui.selectable_value(&mut new_port, Some(p.clone()), format!("{p} (fehlt)"));
                    }
                })
                .response
                .labelled_by(l.id);
        });
        if ui
            .small_button("Ausgänge neu einlesen")
            .on_hover_text("MIDI-Ausgänge des Systems neu einlesen")
            .clicked()
        {
            self.midi_out_ports = None;
        }
        ui.horizontal(|ui| {
            let l = ui.label("Kanal");
            egui::ComboBox::from_id_salt(("midi-kanal", idx))
                .selected_text(channel.to_string())
                .show_ui(ui, |ui| {
                    for c in 1..=16u8 {
                        ui.selectable_value(&mut new_channel, c, c.to_string());
                    }
                })
                .response
                .labelled_by(l.id);
        });
        ui.horizontal(|ui| {
            let mut on = program.is_some();
            ui.checkbox(&mut on, "Programm");
            // Anzeige 1–128 wie auf den meisten Geräten, gespeichert 0–127.
            let mut shown = i32::from(program.unwrap_or(0)) + 1;
            let l = ui.weak("Nr.");
            ui.add_enabled(on, egui::DragValue::new(&mut shown).range(1..=128))
                .labelled_by(l.id);
            new_program = on.then(|| (shown.clamp(1, 128) - 1) as u8);
        });
        if (new_port.clone(), new_channel, new_program) != (port, channel, program) {
            let before = self.project.clone();
            if let Some(t) = self.project.tracks.get_mut(idx) {
                t.midi_port = new_port;
                t.midi_channel = new_channel;
                t.program = new_program;
            }
            self.checkpoint(before);
            // Gewähltes Programm gleich hören lassen.
            if let (Some(out), Some(t)) = (&self.midi_out, self.project.tracks.get(idx))
                && let (Some(port), Some(prog)) = (&t.midi_port, t.program)
            {
                out.send_now(port, [0xc0 | (t.midi_channel.clamp(1, 16) - 1), prog, 0]);
            }
        }
    }
}
