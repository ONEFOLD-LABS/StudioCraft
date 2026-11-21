//! Dialog „Audio- und MIDI-Geräte“ (Issue #9): Host, Aus- und Eingang, Abtastrate,
//! Blockgröße mit Latenz, MIDI-Eingänge. Die Wahl liegt in den Einstellungen.
//!
//! Den Audio-Stream besitzt die App: „Übernehmen“ hinterlegt eine Anfrage
//! ([`ProjectWindow::take_device_request`]), die App wechselt das Gerät und meldet das
//! Ergebnis mit [`ProjectWindow::audio_switched`]. Der Eingang wird bei der nächsten
//! Aufnahme mit dem neuen Gerät geöffnet.

use crate::i18n::tr;
use studiocraft_engine::output::{BLOCK_SIZES, SAMPLE_RATES, available_hosts};
use studiocraft_engine::{DeviceChoice, EngineControl};

use crate::ProjectWindow;

/// Zustand des Dialogs.
#[derive(Debug, Clone, Default)]
pub(crate) struct DeviceDialog {
    pub open: bool,
    /// Die Wahl im Dialog (noch nicht übernommen).
    pub choice: DeviceChoice,
    pub outputs: Vec<String>,
    pub inputs: Vec<String>,
    /// Host, für den die Gerätelisten gelten.
    listed_for: Option<Option<String>>,
    pub message: Option<String>,
}

/// Auswahlliste mit „Standard“ (= `None`) und den Werten.
fn pick<T: Clone + PartialEq>(
    ui: &mut egui::Ui,
    label: &str,
    value: &mut Option<T>,
    options: &[T],
    show: impl Fn(&T) -> String,
) -> bool {
    let before = value.clone();
    ui.label(label);
    let text = value
        .as_ref()
        .map_or_else(|| tr("Standard").to_owned(), &show);
    egui::ComboBox::from_id_salt(label)
        .selected_text(text)
        .width(220.0)
        .show_ui(ui, |ui| {
            ui.selectable_value(value, None, tr("Standard"));
            for o in options {
                ui.selectable_value(value, Some(o.clone()), show(o));
            }
        })
        .response
        .widget_info(|| egui::WidgetInfo::labeled(egui::Role::ComboBox, true, label));
    ui.end_row();
    *value != before
}

/// Latenz eines Blocks in Millisekunden.
pub(crate) fn block_ms(block: u32, rate: u32) -> f32 {
    if rate == 0 {
        return 0.0;
    }
    block as f32 * 1000.0 / rate as f32
}

impl ProjectWindow {
    /// Dialog öffnen, mit der aktuellen Wahl und frischen Gerätelisten.
    pub(crate) fn open_device_dialog(&mut self) {
        self.device_dialog.open = true;
        self.device_dialog.choice = self.audio_choice.clone();
        self.device_dialog.listed_for = None;
        self.device_dialog.message = None;
    }

    /// Gerätelisten des gewählten Hosts (nur bei Bedarf, das Abfragen kann dauern).
    fn refresh_devices(&mut self) {
        let host = self.device_dialog.choice.host.clone();
        if self.device_dialog.listed_for.as_ref() == Some(&host) {
            return;
        }
        self.device_dialog.listed_for = Some(host.clone());
        match studiocraft_engine::device_names(host.as_deref()) {
            Ok((outputs, inputs)) => {
                self.device_dialog.outputs = outputs;
                self.device_dialog.inputs = inputs;
            }
            Err(e) => {
                self.device_dialog.outputs.clear();
                self.device_dialog.inputs.clear();
                self.device_dialog.message = Some(e);
            }
        }
    }

    /// Die App bietet den KI-Assistenten an: „Ansicht → KI-Assistent“ erscheint.
    pub fn enable_assistant(&mut self) {
        self.assistant_available = true;
    }

    /// Ob der KI-Assistent gerade zu sehen sein soll.
    pub fn assistant_open(&self) -> bool {
        self.assistant_available && self.show_assistant
    }

    pub fn set_assistant_open(&mut self, open: bool) {
        self.show_assistant = open;
    }

    /// Die App wechselt das Audiogerät auf diese Wahl (einmal abholen).
    pub fn take_device_request(&mut self) -> Option<DeviceChoice> {
        self.device_request.take()
    }

    /// Aktuelle Wahl (z. B. beim Start aus den Einstellungen).
    pub fn audio_choice(&self) -> &DeviceChoice {
        &self.audio_choice
    }

    /// Wahl, mit der die App tatsächlich gestartet ist (z. B. Host von der Kommandozeile).
    pub fn set_audio_choice(&mut self, choice: DeviceChoice) {
        self.audio_host = choice.host.clone();
        self.audio_choice = choice;
    }

    /// Abtastrate, mit der Engine und geladene Clips laufen.
    pub fn sample_rate(&self) -> u32 {
        self.sample_rate
    }

    /// Ergebnis eines Gerätewechsels durch die App: neue Statuszeile oder Fehler.
    /// Hängt eine Engine an, die erst nach dem Start entsteht (im Browser gibt es Ton erst
    /// nach der ersten Eingabe). Song und Inserts werden danach neu an sie übergeben.
    pub fn attach_transport(&mut self, transport: EngineControl, status: String) {
        self.sample_rate = transport.sample_rate;
        self.transport = Some(transport);
        self.audio_status = status;
        self.dirty = true;
    }

    pub fn audio_switched(&mut self, result: Result<String, String>) {
        match result {
            Ok(status) => {
                self.audio_status = status;
                self.device_dialog.message = Some(tr("Übernommen").into());
            }
            Err(e) => {
                self.device_dialog.message = Some(e.clone());
                self.message = Some(e);
            }
        }
    }

    /// Das Audiogerät ist verschwunden (Fehler des Streams).
    pub fn audio_lost(&mut self) {
        let msg = crate::i18n::trf(
            "Audiogerät nicht mehr verfügbar ({status}). Im Menü Transport unter „Audio- und MIDI-Geräte …“ ein anderes wählen.",
            &[("status", &self.audio_status)],
        );
        self.message = Some(msg);
        self.audio_status = crate::i18n::tr("Kein Audio: Gerät getrennt").into();
    }

    /// MIDI-Eingänge an oder aus (sofort).
    pub(crate) fn set_midi_inputs(&mut self, on: bool) {
        self.midi_inputs_on = on;
        if on {
            self.open_midi_inputs();
        } else {
            self.midi_inputs = None;
            self.midi_status = crate::i18n::tr("MIDI: aus").into();
        }
        self.save_settings();
    }

    pub(crate) fn device_dialog(&mut self, ctx: &egui::Context) {
        if !self.device_dialog.open {
            return;
        }
        self.refresh_devices();
        let mut open = true;
        let mut apply = false;
        let mut midi = self.midi_inputs_on;
        let rate_now = self.sample_rate;
        let status = self.audio_status.clone();
        let hosts: Vec<String> = available_hosts().into_iter().map(str::to_owned).collect();
        let d = &mut self.device_dialog;
        self.dialogs.escape_closes(
            ctx,
            crate::dialog::window_id(tr("Audio- und MIDI-Geräte")),
            &mut open,
        );
        egui::Window::new(tr("Audio- und MIDI-Geräte"))
            .open(&mut open)
            .resizable(false)
            .collapsible(false)
            .show(ctx, |ui| {
                ui.label(egui::RichText::new(status).weak());
                ui.separator();
                egui::Grid::new("geraete").num_columns(2).show(ui, |ui| {
                    if pick(
                        ui,
                        crate::i18n::tr("Host"),
                        &mut d.choice.host,
                        &hosts,
                        Clone::clone,
                    ) {
                        // Andere Geräte: Auswahl zurück auf Standard.
                        d.choice.output = None;
                        d.choice.input = None;
                    }
                    pick(
                        ui,
                        tr("Ausgang"),
                        &mut d.choice.output,
                        &d.outputs,
                        Clone::clone,
                    );
                    pick(
                        ui,
                        crate::i18n::tr("Eingang"),
                        &mut d.choice.input,
                        &d.inputs,
                        Clone::clone,
                    );
                    pick(
                        ui,
                        tr("Abtastrate"),
                        &mut d.choice.sample_rate,
                        &SAMPLE_RATES,
                        |r| format!("{r} Hz"),
                    );
                    let rate = d.choice.sample_rate.unwrap_or(rate_now);
                    pick(
                        ui,
                        tr("Blockgröße"),
                        &mut d.choice.block_frames,
                        &BLOCK_SIZES,
                        |b| format!("{b} ({:.1} ms)", block_ms(*b, rate)),
                    );
                });
                if d.choice.sample_rate.is_some_and(|r| r != rate_now) {
                    ui.colored_label(
                        ui.visuals().warn_fg_color,
                        tr("Eine andere Abtastrate gilt nach einem Neustart."),
                    );
                }
                ui.checkbox(&mut midi, tr("MIDI-Eingänge"));
                ui.horizontal(|ui| {
                    if ui.button(tr("Übernehmen")).clicked() {
                        apply = true;
                    }
                    if let Some(m) = &d.message {
                        ui.label(m);
                    }
                });
            });
        if midi != self.midi_inputs_on {
            self.set_midi_inputs(midi);
        }
        if apply {
            let choice = self.device_dialog.choice.clone();
            if choice.input != self.audio_choice.input || choice.host != self.audio_choice.host {
                // Neuer Eingang: bei der nächsten Aufnahme mit dem neuen Gerät öffnen.
                self.recorder = None;
                self.input = None;
            }
            self.audio_host = choice.host.clone();
            self.audio_choice = choice.clone();
            self.device_request = Some(choice);
            self.device_dialog.message = Some(tr("Wird übernommen …").into());
            self.save_settings();
        }
        if !open {
            self.device_dialog.open = false;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn block_latency_in_ms() {
        assert!((block_ms(256, 48_000) - 5.333).abs() < 0.01);
        assert_eq!(block_ms(256, 0), 0.0);
    }
}
