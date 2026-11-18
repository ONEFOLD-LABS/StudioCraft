//! Oberfläche von StudioCraft (egui), aufgebaut nach `docs/spec.md`,
//! Abschnitte „Fenster-Architektur“, „Werkzeuge“, „Raster, Snap und Bearbeitung“.
//!
//! Prototyp-Stand: Projektfenster mit Werkzeugzeile, Inspector, Spurliste,
//! Lineal, Event-Anzeige und Transportleiste.

mod inserts;
mod keyeditor;
mod menu;
mod mixer;
mod recording;
mod session;
mod timeline;
mod tools;
pub mod waveform;

use std::{
    collections::HashMap,
    path::{Path, PathBuf},
    sync::Arc,
};

use egui::{Color32, Key, RichText, Ui};
use studiocraft_engine::{AudioClip, Command, EngineControl, Song, TransportState};
use studiocraft_project::{
    ClipId, EventId, PoolClip, Position, Project, TICKS_PER_QUARTER, TrackKind,
};
use waveform::Peaks;

/// Samples und Wellenform eines Pool-Clips (in Engine-Abtastrate).
pub struct LoadedClip {
    pub samples: Arc<[f32]>,
    pub peaks: Peaks,
}

pub use tools::{Grid, Tool};

/// Der Zustand des Projektfensters.
pub struct ProjectWindow {
    pub project: Project,
    transport: Option<EngineControl>,
    /// Text für die Statuszeile (Audio-Gerät oder Fehler).
    pub audio_status: String,
    tool: Tool,
    snap: bool,
    grid: Grid,
    show_inspector: bool,
    view: timeline::View,
    selected_track: Option<usize>,
    selection: Vec<EventId>,
    drag: Option<timeline::Drag>,
    /// Cursor und Locatoren in Ticks (Spiegel des Engine-Zustands).
    cursor: u64,
    left_locator: u64,
    right_locator: u64,
    cycle: bool,
    frame_ms: f32,
    /// Audio-Daten zum Pool des Projekts.
    clips: HashMap<ClipId, LoadedClip>,
    /// Abtastrate der Engine; Clips werden beim Import darauf umgerechnet.
    sample_rate: u32,
    /// Projekt geändert: Abspielliste der Engine neu bauen.
    dirty: bool,
    /// Letzte Meldung (z. B. Importfehler) für die Statuszeile.
    message: Option<String>,
    show_lower: bool,
    /// Angezeigte Pegel je Spur und Ausgang (linear, mit Abfall).
    meter_levels: Vec<[f32; 2]>,
    master_meter: [f32; 2],
    /// Undo- und Redo-Verlauf (ganze Projektstände; Spec „Undo“).
    undo: Vec<Project>,
    redo: Vec<Project>,
    /// Projektstand zu Beginn eines Ziehvorgangs.
    drag_before: Option<Project>,
    /// Lage der Event-Anzeige im letzten Frame (für Tests mit Mauseingaben).
    lanes_rect: egui::Rect,
    /// Projektdatei, unter der zuletzt gespeichert oder geöffnet wurde.
    project_path: Option<PathBuf>,
    /// Zuletzt gespeicherter Stand, um ungesicherte Änderungen zu erkennen.
    saved: Option<Project>,
    /// Befehl, der auf die Rückfrage „Speichern?“ wartet.
    pending: Option<menu::Action>,
    /// Schließen ist bestätigt (keine weitere Rückfrage).
    allow_close: bool,
    last_title: String,
    /// Audio-Host, mit dem auch der Eingang geöffnet wird.
    pub audio_host: Option<String>,
    input: Option<studiocraft_engine::AudioInput>,
    recorder: Option<studiocraft_engine::Recorder>,
    input_status: String,
    recording: bool,
    punch_in: bool,
    punch_out: bool,
    overruns_at_start: u64,
    plugins: inserts::PluginState,
    /// Was die untere Zone zeigt.
    lower: LowerZone,
    key_editor: keyeditor::KeyEditor,
    midi_inputs: Option<studiocraft_engine::midi::MidiInputs>,
    /// Anzeige der MIDI-Eingänge für die Statuszeile.
    midi_status: String,
    /// Zuletzt an die Engine gemeldetes Keyboard-Ziel.
    midi_target_sent: Option<Option<usize>>,
    /// Während der Aufnahme gespielte Noten.
    midi_take: Vec<studiocraft_engine::midi::RecordedMidi>,
    /// Die laufende Aufnahme schreibt auch Audio.
    audio_recording: bool,
}

/// Inhalt der unteren Zone (Spec „Fenster-Architektur“).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum LowerZone {
    Mixer,
    Editor,
}

impl ProjectWindow {
    pub fn new(project: Project, transport: Option<EngineControl>, audio_status: String) -> Self {
        let bar = project.tempo.signature.bar_ticks();
        let sample_rate = transport.as_ref().map_or(48_000, |t| t.sample_rate);
        let mut w = Self {
            project,
            transport,
            audio_status,
            tool: Tool::Select,
            snap: true,
            grid: Grid::Beat,
            show_inspector: true,
            view: timeline::View::default(),
            selected_track: Some(0),
            selection: Vec::new(),
            drag: None,
            cursor: 0,
            left_locator: 0,
            right_locator: 4 * bar,
            cycle: false,
            frame_ms: 0.0,
            clips: HashMap::new(),
            sample_rate,
            dirty: true,
            message: None,
            show_lower: true,
            meter_levels: Vec::new(),
            master_meter: [0.0; 2],
            undo: Vec::new(),
            redo: Vec::new(),
            drag_before: None,
            lanes_rect: egui::Rect::NOTHING,
            project_path: None,
            saved: None,
            pending: None,
            allow_close: false,
            last_title: String::new(),
            audio_host: None,
            input: None,
            recorder: None,
            input_status: String::new(),
            recording: false,
            punch_in: false,
            punch_out: false,
            overruns_at_start: 0,
            plugins: inserts::PluginState::default(),
            lower: LowerZone::Mixer,
            key_editor: keyeditor::KeyEditor::default(),
            midi_inputs: None,
            midi_status: String::new(),
            midi_target_sent: None,
            midi_take: Vec::new(),
            audio_recording: false,
        };
        w.saved = Some(w.project.clone());
        w.send_locators();
        w
    }

    fn send(&mut self, cmd: Command) {
        if let Some(t) = &mut self.transport {
            t.send(cmd);
        }
    }

    fn send_locators(&mut self) {
        if let Some(t) = &mut self.transport {
            let l = t.frames(Position(self.left_locator));
            let r = t.frames(Position(self.right_locator));
            t.send(Command::SetLocators(l, r));
        }
    }

    pub fn is_playing(&self) -> bool {
        self.transport
            .as_ref()
            .is_some_and(|t| t.status.state() != TransportState::Stopped)
    }

    fn locate(&mut self, ticks: u64) {
        self.cursor = ticks;
        if let Some(t) = &mut self.transport {
            let f = t.frames(Position(ticks));
            t.send(Command::Locate(f));
        }
    }

    fn toggle_play(&mut self) {
        if self.is_playing() || self.recording {
            self.stop_transport();
        } else {
            self.send(Command::Play);
        }
    }

    fn toggle_cycle(&mut self) {
        self.cycle = !self.cycle;
        self.send(Command::ToggleCycle);
    }

    pub(crate) fn snap_ticks(&self, ticks: f64) -> u64 {
        let t = ticks.max(0.0);
        if !self.snap {
            return t.round() as u64;
        }
        let g = self.grid.ticks(&self.project) as f64;
        ((t / g).round() * g) as u64
    }

    /// Lädt eine WAV-Datei in den Pool und legt ein Event an. Ohne Ziel
    /// (Spur, Position) entsteht eine neue Audio-Spur ab Takt 1.
    pub fn import_audio(
        &mut self,
        path: &Path,
        target: Option<(usize, u64)>,
    ) -> Result<EventId, String> {
        let clip = AudioClip::load_wav(path).map_err(|e| format!("{}: {e}", path.display()))?;
        let name = path
            .file_stem()
            .map_or("Audio".into(), |s| s.to_string_lossy().into_owned());
        self.import_clip(&name, Some(path.display().to_string()), clip, target)
    }

    /// Wie [`Self::import_audio`], mit bereits geladenen Samples.
    pub fn import_clip(
        &mut self,
        name: &str,
        path: Option<String>,
        clip: AudioClip,
        target: Option<(usize, u64)>,
    ) -> Result<EventId, String> {
        let clip = clip.resampled(self.sample_rate);
        let id = self.project.add_clip(PoolClip {
            name: name.to_owned(),
            path,
            sample_rate: clip.sample_rate,
            frames: clip.frames(),
        });
        let peaks = Peaks::new(&clip.samples);
        self.clips.insert(
            id,
            LoadedClip {
                samples: clip.samples,
                peaks,
            },
        );
        let (track, start) = match target {
            Some((t, at))
                if self
                    .project
                    .tracks
                    .get(t)
                    .is_some_and(|t| t.kind == TrackKind::Audio) =>
            {
                (t, at)
            }
            Some((_, at)) => (self.project.add_track(TrackKind::Audio, name), at),
            None => (self.project.add_track(TrackKind::Audio, name), 0),
        };
        let ev = self
            .project
            .add_audio_event(track, start, id)
            .ok_or_else(|| format!("{name}: Event konnte nicht angelegt werden"))?;
        self.selected_track = Some(track);
        self.selection = vec![ev];
        self.dirty = true;
        Ok(ev)
    }

    /// Legt `before` im Undo-Verlauf ab, falls sich das Projekt seitdem geändert hat.
    pub(crate) fn checkpoint(&mut self, before: Project) {
        if before == self.project {
            return;
        }
        self.undo.push(before);
        if self.undo.len() > UNDO_STEPS {
            self.undo.remove(0);
        }
        self.redo.clear();
        self.dirty = true;
    }

    pub fn undo(&mut self) {
        if let Some(prev) = self.undo.pop() {
            self.redo.push(std::mem::replace(&mut self.project, prev));
            self.after_history_step();
        }
    }

    pub fn redo(&mut self) {
        if let Some(next) = self.redo.pop() {
            self.undo.push(std::mem::replace(&mut self.project, next));
            self.after_history_step();
        }
    }

    fn after_history_step(&mut self) {
        let p = &self.project;
        self.selection.retain(|&id| p.event(id).is_some());
        if self.selected_track.is_some_and(|t| t >= p.tracks.len()) {
            self.selected_track = p.tracks.len().checked_sub(1);
        }
        self.drag = None;
        self.dirty = true;
    }

    pub(crate) fn delete_selection(&mut self) {
        let before = self.project.clone();
        for id in std::mem::take(&mut self.selection) {
            self.project.delete(id);
        }
        self.checkpoint(before);
    }

    /// Verlauf leeren, z. B. nach dem Laden der Startdateien.
    pub fn clear_history(&mut self) {
        self.undo.clear();
        self.redo.clear();
    }

    pub(crate) fn clip(&self, id: ClipId) -> Option<&LoadedClip> {
        self.clips.get(&id)
    }

    /// Baut die Abspielliste neu und übergibt sie der Engine.
    fn rebuild_song(&mut self) {
        let Some(t) = &mut self.transport else { return };
        let samples: HashMap<ClipId, Arc<[f32]>> = self
            .clips
            .iter()
            .map(|(&id, c)| (id, c.samples.clone()))
            .collect();
        let song = Song::build(&self.project, &samples, self.sample_rate);
        t.send(Command::SetSong(Arc::new(song)));
    }

    fn sync_from_engine(&mut self) {
        if let Some(t) = &self.transport {
            self.cursor = t.position(t.status.cursor()).0;
        }
    }

    fn handle_keys(&mut self, ui: &Ui) {
        if ui.ctx().egui_wants_keyboard_input() {
            return;
        }
        // `*` (auch auf dem Nummernblock): Aufnahme. egui hat dafür keine eigene Taste.
        let star = ui.input(|i| {
            i.events
                .iter()
                .any(|e| matches!(e, egui::Event::Text(t) if t == "*"))
        });
        if star {
            self.toggle_record();
        }
        let (keys, mods) = ui.input(|i| {
            let mut pressed = Vec::new();
            for k in [
                Key::Num1,
                Key::Num2,
                Key::Num3,
                Key::Num4,
                Key::Num5,
                Key::Num6,
                Key::Num7,
                Key::Num8,
                Key::Num9,
                Key::Num0,
                Key::J,
                Key::H,
                Key::G,
                Key::Space,
                Key::Delete,
                Key::Backspace,
                Key::Home,
                Key::I,
                Key::Comma,
                Key::Slash,
                Key::F3,
                Key::O,
                Key::Z,
                Key::Y,
            ] {
                if i.key_pressed(k) {
                    pressed.push(k);
                }
            }
            (pressed, i.modifiers)
        });
        let alt = mods.alt;
        for k in keys {
            // Strg+Z / Strg+Umschalt+Z (und Strg+Y) vor allem anderen.
            if mods.command && (k == Key::Z || k == Key::Y) {
                if k == Key::Y || mods.shift {
                    self.redo();
                } else {
                    self.undo();
                }
                continue;
            }
            if mods.command {
                // Datei-Kürzel behandelt `file_shortcuts`.
                continue;
            }
            if let Some(tool) = Tool::from_key(k) {
                self.tool = tool;
                continue;
            }
            match k {
                Key::J => self.snap = !self.snap,
                Key::H => self.view.zoom(1.25, None),
                Key::G => self.view.zoom(0.8, None),
                Key::Space => self.toggle_play(),
                Key::Slash => self.toggle_cycle(),
                Key::F3 => {
                    // F3: Mixer ein/aus; ist gerade der Editor zu sehen, zum Mixer wechseln.
                    if self.show_lower && self.lower == LowerZone::Mixer {
                        self.show_lower = false;
                    } else {
                        self.show_lower = true;
                        self.lower = LowerZone::Mixer;
                    }
                }
                Key::Delete | Key::Backspace => {
                    if self.key_editor.focused && self.show_lower && self.lower == LowerZone::Editor
                    {
                        self.delete_selected_notes();
                    } else {
                        self.delete_selection();
                    }
                }
                Key::Home | Key::Comma => self.locate(0),
                Key::I if alt => self.show_inspector = !self.show_inspector,
                Key::I => self.punch_in = !self.punch_in,
                Key::O => self.punch_out = !self.punch_out,
                _ => {}
            }
        }
    }

    /// Zeichnet das ganze Projektfenster.
    pub fn ui(&mut self, ui: &mut Ui) {
        self.sync_from_engine();
        if let Some(t) = &mut self.transport {
            t.collect_garbage();
        }
        self.sync_midi();
        self.file_shortcuts(ui);
        self.handle_keys(ui);
        let dt_s = ui.input(|i| i.unstable_dt);
        mixer::update_meters(self, dt_s);
        let dt = dt_s * 1000.0;
        self.frame_ms = if self.frame_ms == 0.0 {
            dt
        } else {
            self.frame_ms * 0.95 + dt * 0.05
        };

        egui::Panel::top("menue").show(ui, |ui| self.menu_bar(ui));
        egui::Panel::top("werkzeugzeile").show(ui, |ui| self.toolbar(ui));
        egui::Panel::bottom("transportleiste").show(ui, |ui| self.transport_bar(ui));
        if self.show_lower {
            egui::Panel::bottom("untere-zone")
                .resizable(true)
                .default_size(300.0)
                .min_size(200.0)
                .show(ui, |ui| {
                    ui.horizontal(|ui| {
                        ui.selectable_value(&mut self.lower, LowerZone::Mixer, "Mixer");
                        ui.selectable_value(&mut self.lower, LowerZone::Editor, "Key-Editor");
                        if self.lower == LowerZone::Editor
                            && let Some(ev) =
                                self.key_editor.part.and_then(|p| self.project.event(p))
                        {
                            ui.separator();
                            ui.weak(format!("{} · {} Noten", ev.name, ev.notes.len()));
                        }
                    });
                    match self.lower {
                        LowerZone::Mixer => mixer::show(self, ui),
                        LowerZone::Editor => keyeditor::show(self, ui),
                    }
                });
        }
        if self.show_inspector {
            egui::Panel::left("inspector")
                .resizable(true)
                .default_size(200.0)
                .show(ui, |ui| self.inspector(ui));
        }
        egui::CentralPanel::default().show(ui, |ui| timeline::show(self, ui));

        if self.dirty {
            self.dirty = false;
            self.sync_inserts();
            self.rebuild_song();
        }

        self.confirm_dialog(ui);
        let title = self.title();
        if title != self.last_title {
            ui.ctx()
                .send_viewport_cmd(egui::ViewportCommand::Title(title.clone()));
            self.last_title = title;
        }

        let meters_moving = self.master_meter.iter().any(|&v| v > 1e-4)
            || self.meter_levels.iter().flatten().any(|&v| v > 1e-4);
        if self.is_playing() || meters_moving {
            ui.ctx().request_repaint();
        }
    }

    fn toolbar(&mut self, ui: &mut Ui) {
        ui.horizontal_wrapped(|ui| {
            for tool in Tool::ALL {
                let text = format!("{} {}", tool.key_label(), tool.label());
                let r = ui.selectable_label(self.tool == tool, text);
                if r.clicked() {
                    self.tool = tool;
                }
                if !tool.implemented() {
                    r.on_hover_text("Im Prototyp noch ohne Funktion");
                }
            }
            ui.separator();
            ui.toggle_value(&mut self.snap, "Snap (J)");
            egui::ComboBox::from_label("Raster")
                .selected_text(self.grid.label())
                .show_ui(ui, |ui| {
                    for g in Grid::ALL {
                        ui.selectable_value(&mut self.grid, g, g.label());
                    }
                });
        });
    }

    fn inspector(&mut self, ui: &mut Ui) {
        ui.heading("Inspector");
        let Some(idx) = self
            .selected_track
            .filter(|&i| i < self.project.tracks.len())
        else {
            ui.label("Keine Spur ausgewählt");
            return;
        };
        let channels = self.input_channels();
        let track = &mut self.project.tracks[idx];
        ui.horizontal(|ui| {
            let label = ui.label("Name");
            ui.add(egui::TextEdit::singleline(&mut track.name))
                .labelled_by(label.id);
        });
        ui.label(format!("Typ: {}", track.kind.label()));
        if track.kind == TrackKind::Audio {
            egui::ComboBox::from_label("Eingang")
                .selected_text(track.input.label())
                .show_ui(ui, |ui| {
                    for opt in studiocraft_project::TrackInput::options(channels) {
                        ui.selectable_value(&mut track.input, opt, opt.label());
                    }
                });
        }
        ui.horizontal(|ui| {
            let m = ui.toggle_value(&mut track.mute, "M").changed();
            let s = ui.toggle_value(&mut track.solo, "S").changed();
            self.dirty |= m || s;
            if track.kind.holds_events() {
                ui.toggle_value(&mut track.record, "R");
            }
        });
        ui.horizontal(|ui| {
            let r = ui.add(
                egui::Slider::new(
                    &mut track.volume_db,
                    studiocraft_project::MIN_DB..=studiocraft_project::MAX_DB,
                )
                .text("dB")
                .custom_formatter(|v, _| mixer::db_label(v as f32)),
            );
            self.dirty |= r.changed();
        });
        ui.horizontal(|ui| {
            let r = ui.add(egui::Slider::new(&mut track.pan, -1.0..=1.0).text("Pan"));
            self.dirty |= r.changed();
        });
        let mut c = Color32::from_rgb(track.color[0], track.color[1], track.color[2]);
        if ui.color_edit_button_srgba(&mut c).changed() {
            track.color = [c.r(), c.g(), c.b()];
        }
        ui.label(format!("{} Events", track.events.len()));
        ui.separator();
        if self.project.tracks[idx].kind == TrackKind::Instrument {
            egui::CollapsingHeader::new("Instrument")
                .default_open(true)
                .show(ui, |ui| self.instrument_ui(ui, idx));
        }
        if matches!(
            self.project.tracks[idx].kind,
            TrackKind::Audio | TrackKind::Instrument
        ) {
            egui::CollapsingHeader::new("Inserts")
                .default_open(true)
                .show(ui, |ui| self.inserts_ui(ui, idx));
        }
        for section in ["EQ", "Sends", "Notizen"] {
            egui::CollapsingHeader::new(section)
                .default_open(false)
                .show(ui, |ui| {
                    ui.weak("Kommt mit dem Mixer");
                });
        }
    }

    fn transport_bar(&mut self, ui: &mut Ui) {
        ui.horizontal(|ui| {
            if ui.button("⏮").on_hover_text("Zum Anfang (Pos1)").clicked() {
                self.locate(0);
            }
            if ui.button("⏹").on_hover_text("Stopp").clicked() {
                self.stop_transport();
            }
            let playing = self.is_playing();
            if ui
                .selectable_label(playing, "▶")
                .on_hover_text("Start/Stopp (Leertaste)")
                .clicked()
            {
                self.toggle_play();
            }
            let rec = egui::RichText::new("⏺").color(if self.recording {
                Color32::from_rgb(240, 60, 60)
            } else {
                ui.visuals().text_color()
            });
            if ui
                .selectable_label(self.recording, rec)
                .on_hover_text("Aufnahme (*)")
                .clicked()
            {
                self.toggle_record();
            }
            if ui
                .selectable_label(self.cycle, "🔁")
                .on_hover_text("Cycle (/)")
                .clicked()
            {
                self.toggle_cycle();
            }
            ui.selectable_value_toggle_punch(
                &mut self.punch_in,
                "Punch-In",
                "Aufnahme erst ab dem linken Locator (I)",
            );
            ui.selectable_value_toggle_punch(
                &mut self.punch_out,
                "Punch-Out",
                "Aufnahme nur bis zum rechten Locator (O)",
            );
            ui.separator();
            let tempo = &self.project.tempo;
            let (bar, beat, six, tick) = tempo.bars_beats(Position(self.cursor));
            let secs = tempo.seconds(Position(self.cursor));
            ui.label(
                RichText::new(format!("{bar}.{beat}.{six}.{tick:03}"))
                    .monospace()
                    .size(18.0),
            );
            ui.label(
                RichText::new(format!("{:02}:{:06.3}", (secs / 60.0) as u32, secs % 60.0))
                    .monospace(),
            );
            ui.separator();
            let fmt = |t: u64| {
                let (b, be, s, _) = tempo.bars_beats(Position(t));
                format!("{b}.{be}.{s}")
            };
            ui.label(format!(
                "L {}  R {}",
                fmt(self.left_locator),
                fmt(self.right_locator)
            ));
            ui.label(format!(
                "{} BPM  {}/{}",
                tempo.bpm, tempo.signature.numerator, tempo.signature.denominator
            ));
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                ui.weak(format!("{:.1} ms/Frame", self.frame_ms));
                ui.separator();
                if !self.midi_status.is_empty() {
                    ui.weak(&self.midi_status);
                    ui.separator();
                }
                ui.weak(&self.audio_status);
                if let Some(m) = &self.message {
                    ui.separator();
                    ui.colored_label(ui.visuals().warn_fg_color, m);
                }
            });
        });
    }

    pub(crate) fn bar_ticks(&self) -> u64 {
        self.project.tempo.signature.bar_ticks()
    }

    pub(crate) fn beat_ticks(&self) -> u64 {
        self.project.tempo.signature.beat_ticks()
    }
}

/// Kleiner Umschalter für die Transportleiste.
trait PunchToggle {
    fn selectable_value_toggle_punch(&mut self, value: &mut bool, label: &str, hint: &str);
}

impl PunchToggle for Ui {
    fn selectable_value_toggle_punch(&mut self, value: &mut bool, label: &str, hint: &str) {
        if self
            .selectable_label(*value, label)
            .on_hover_text(hint)
            .clicked()
        {
            *value = !*value;
        }
    }
}

/// Ticks pro Viertel als `f64`, für Umrechnungen in Pixel.
pub(crate) const TPQ: f64 = TICKS_PER_QUARTER as f64;

/// So viele Schritte lassen sich rückgängig machen.
const UNDO_STEPS: usize = 200;

#[cfg(test)]
mod tests {
    use super::*;
    use egui_kittest::{Harness, kittest::Queryable};
    use studiocraft_project::TempoMap;

    /// Rendert das Projektfenster headless. Mit `STUDIOCRAFT_SHOTS=<ordner>`
    /// werden die Bilder gespeichert, sonst wird nur geprüft, dass es rendert.
    fn shot(name: &str, setup: impl FnOnce(&mut ProjectWindow)) {
        let mut w = ProjectWindow::new(Project::demo(100), None, "kein Audio (Test)".into());
        setup(&mut w);
        let mut harness = Harness::builder()
            .with_size(egui::vec2(1400.0, 860.0))
            .with_step_dt(1.0 / 60.0)
            .wgpu()
            .build_ui(move |ui| w.ui(ui));
        // Pegelanzeigen klingen ab und fordern weitere Frames an: feste Schrittzahl,
        // danach die Barrierefreiheit ausdrücklich prüfen.
        harness.run_steps(4);
        harness.check_accessibility();
        let Ok(img) = harness.render() else {
            eprintln!("kein GPU-Renderer verfügbar, Bild übersprungen");
            return;
        };
        if let Ok(dir) = std::env::var("STUDIOCRAFT_SHOTS") {
            img.save(std::path::Path::new(&dir).join(format!("{name}.png")))
                .unwrap();
        }
    }

    /// Misst die CPU-Zeit pro Frame (Layout, Zeichnen, Tessellierung) bei
    /// 100 und 1000 Spuren. `cargo test -p studiocraft-ui --release -- --ignored --nocapture`
    #[test]
    #[ignore = "Messung, kein Funktionstest"]
    fn frame_time_many_tracks() {
        for (n, audio) in [(100, false), (1000, false), (50, true)] {
            let mut w = if audio {
                let mut w = ProjectWindow::new(Project::default(), None, String::new());
                let clip = tone_clip(120.0, 110.0);
                for _ in 0..n {
                    w.import_clip("Audio", None, clip.clone(), None).unwrap();
                }
                w
            } else {
                ProjectWindow::new(Project::demo(n), None, String::new())
            };
            w.view.zoom(0.5, None);
            let calls = std::rc::Rc::new(std::cell::Cell::new(0u32));
            let counter = calls.clone();
            let mut harness = Harness::builder()
                .with_size(egui::vec2(1920.0, 1080.0))
                .build_ui(move |ui| {
                    // Jede Frame scrollen, damit nichts gecacht bleibt.
                    w.view.scroll_y = (w.view.scroll_y + 7.0) % 2000.0;
                    w.view.scroll_ticks = (w.view.scroll_ticks + 50.0) % 100_000.0;
                    w.ui(ui);
                    counter.set(counter.get() + 1);
                });
            harness.step();
            let before = calls.get();
            let frames = 300;
            let t = std::time::Instant::now();
            for _ in 0..frames {
                harness.step();
            }
            let ms = t.elapsed().as_secs_f64() * 1000.0 / f64::from(frames);
            assert!(
                calls.get() - before >= frames,
                "Oberfläche wurde nicht jede Frame gebaut"
            );
            let art = if audio {
                "Audio-Spuren mit Wellenform"
            } else {
                "Spuren"
            };
            println!(
                "{n} {art}: {ms:.2} ms CPU pro Frame ({} Durchläufe)",
                calls.get() - before
            );
        }
    }

    /// Synthetischer Clip: abklingende Töne, damit die Wellenform Struktur hat.
    fn tone_clip(seconds: f32, hz: f32) -> AudioClip {
        let rate = 48_000;
        let n = (seconds * rate as f32) as usize;
        let beat = rate as usize / 2; // Viertel bei 120 BPM
        let samples: Vec<f32> = (0..n)
            .flat_map(|i| {
                let t = i as f32 / rate as f32;
                let env = (-((i % beat) as f32) / (beat as f32 * 0.25)).exp();
                let v = (t * hz * std::f32::consts::TAU).sin() * env * 0.9;
                [v, v]
            })
            .collect();
        AudioClip {
            sample_rate: rate,
            samples: samples.into(),
        }
    }

    #[test]
    fn mute_and_solo_reach_the_engine() {
        let (mut core, ctl) = studiocraft_engine::engine(48_000, TempoMap::default());
        let mut w = ProjectWindow::new(Project::default(), Some(ctl), String::new());
        w.import_clip("A", None, tone_clip(1.0, 220.0), None)
            .unwrap();
        w.import_clip("B", None, tone_clip(1.0, 330.0), None)
            .unwrap();
        let mut out = vec![0.0f32; 512];
        let mut level = |w: &mut ProjectWindow| {
            w.rebuild_song();
            w.locate(0);
            w.send(Command::Play);
            core.process(&mut out);
            w.send(Command::Stop);
            core.process(&mut [0.0; 2]);
            out.iter().map(|s| s.abs()).fold(0.0f32, f32::max)
        };
        let both = level(&mut w);
        assert!(both > 0.5, "beide Spuren klingen: {both}");
        w.project.tracks[0].mute = true;
        let one = level(&mut w);
        assert!(one > 0.1 && one < both, "eine Spur stumm: {one}");
        w.project.tracks[0].mute = false;
        w.project.tracks[1].solo = true;
        w.project.tracks[1].mute = true;
        assert_eq!(level(&mut w), 0.0, "Solo auf stummer Spur: Stille");
    }

    #[test]
    fn renders_audio_events_with_waveforms() {
        shot("audio-wellenformen", |w| {
            w.project = Project::default();
            w.import_clip("Bass", None, tone_clip(8.0, 55.0), None)
                .unwrap();
            w.import_clip(
                "Gitarre",
                None,
                tone_clip(6.0, 196.0),
                Some((usize::MAX, 2 * w.bar_ticks())),
            )
            .unwrap();
            let id = w
                .import_clip(
                    "Gesang",
                    None,
                    tone_clip(8.0, 440.0),
                    Some((usize::MAX, w.bar_ticks())),
                )
                .unwrap();
            let right = w.project.split(id, 3 * w.bar_ticks()).unwrap();
            w.project.move_event(right, 2, 4 * w.bar_ticks(), false);
            w.project.tracks[1].events[0].muted = true;
            w.view.zoom(2.0, None);
            w.selection = vec![right];
        });
    }

    #[test]
    fn renders_mixer_with_meters() {
        shot("mixer", |w| {
            w.project = Project::default();
            for (name, hz) in [
                ("Kick", 55.0),
                ("Bass", 82.0),
                ("Gitarre", 196.0),
                ("Gesang", 440.0),
            ] {
                w.import_clip(name, None, tone_clip(8.0, hz), None).unwrap();
            }
            w.project.tracks[1].volume_db = -6.0;
            w.project.tracks[2].pan = -0.4;
            w.project.tracks[3].volume_db = 3.0;
            w.project.tracks[3].solo = true;
            w.project.master_db = -2.5;
            w.meter_levels = vec![[0.9, 0.85], [0.4, 0.38], [0.6, 0.2], [1.2, 1.1]];
            w.master_meter = [0.7, 0.65];
        });
    }

    /// Fenster mit einer Audio-Spur und einem Event von Takt 2 bis 6.
    fn one_event_window() -> (ProjectWindow, EventId) {
        let mut w = ProjectWindow::new(Project::default(), None, String::new());
        let bar = w.bar_ticks();
        let id = w
            .import_clip("Take", None, tone_clip(8.0, 220.0), Some((usize::MAX, bar)))
            .unwrap();
        w.clear_history();
        w.selection.clear();
        (w, id)
    }

    /// Zieht mit der Maus von `from` nach `to` (in Ticks, relativ zur Zeile 0).
    fn drag(
        harness: &mut Harness<'_, ProjectWindow>,
        from: (u64, f32),
        to: (u64, f32),
        shift_x: f32,
    ) {
        let pos = |h: &Harness<'_, ProjectWindow>, (t, y): (u64, f32)| {
            let w = h.state();
            let r = w.lanes_rect;
            let x =
                r.left() + ((t as f64 - w.view.scroll_ticks) / TPQ) as f32 * w.view.px_per_quarter;
            egui::pos2(x + shift_x, r.top() + y * w.view.row_h)
        };
        let a = pos(harness, from);
        let b = pos(harness, to);
        let button = |pos, pressed| egui::Event::PointerButton {
            pos,
            button: egui::PointerButton::Primary,
            pressed,
            modifiers: egui::Modifiers::NONE,
        };
        harness.event(egui::Event::PointerMoved(a));
        harness.run_steps(1);
        harness.event(button(a, true));
        harness.run_steps(1);
        for i in 1..=5 {
            harness.event(egui::Event::PointerMoved(a + (b - a) * (i as f32 / 5.0)));
            harness.run_steps(1);
        }
        harness.event(button(b, false));
        harness.run_steps(2);
    }

    fn window_harness(w: ProjectWindow) -> Harness<'static, ProjectWindow> {
        let mut h = Harness::builder()
            .with_size(egui::vec2(1400.0, 860.0))
            .with_step_dt(1.0 / 60.0)
            .build_ui_state(|ui, w: &mut ProjectWindow| w.ui(ui), w);
        h.run_steps(2);
        h
    }

    #[test]
    fn mouse_trims_edges_and_drags_fades() {
        let (w, id) = one_event_window();
        let bar = w.bar_ticks();
        let beat = w.beat_ticks();
        let mut h = window_harness(w);

        // Das Event reicht von Tick `bar` bis `5·bar`, also von Takt 2 bis Takt 6.
        assert_eq!(h.state().project.event(id).unwrap().end(), 5 * bar);
        // Untere Hälfte am rechten Rand: Ende von Takt 6 auf Takt 5.
        drag(&mut h, (5 * bar, 0.8), (4 * bar, 0.8), -2.0);
        assert_eq!(
            h.state().project.event(id).unwrap().end(),
            4 * bar,
            "Ende auf Takt 5"
        );

        // Untere Hälfte am linken Rand: Anfang eine Zählzeit nach rechts.
        drag(&mut h, (bar, 0.8), (bar + beat, 0.8), 2.0);
        let ev = h.state().project.event(id).unwrap().clone();
        assert_eq!(ev.start.0, bar + beat);
        assert_eq!(ev.offset, 24_000, "Inhalt bleibt an seiner Stelle");

        // Obere Hälfte am linken Rand: Fade-In über eine Zählzeit.
        drag(&mut h, (bar + beat, 0.4), (bar + 2 * beat, 0.4), 1.0);
        let fade = h.state().project.event(id).unwrap().fade_in;
        assert!(
            fade.abs_diff(beat) < beat / 8,
            "Fade-In etwa eine Zählzeit: {fade}"
        );

        // Drei Schritte rückgängig, dann einer wiederhergestellt.
        let w = h.state_mut();
        w.undo();
        w.undo();
        w.undo();
        assert_eq!(w.project.event(id).unwrap().end(), 5 * bar);
        assert_eq!(w.project.event(id).unwrap().fade_in, 0);
        w.redo();
        assert_eq!(w.project.event(id).unwrap().end(), 4 * bar);
    }

    #[test]
    fn undo_redo_split_and_delete() {
        let (mut w, id) = one_event_window();
        let original = w.project.clone();
        let before = w.project.clone();
        let right = w.project.split(id, 3 * w.bar_ticks()).unwrap();
        w.checkpoint(before);
        let before = w.project.clone();
        w.project.delete(right);
        w.checkpoint(before);
        w.undo();
        assert!(w.project.event(right).is_some());
        w.undo();
        assert_eq!(w.project, original);
        w.redo();
        w.redo();
        assert!(w.project.event(right).is_none());
        // Unveränderter Stand erzeugt keinen Undo-Schritt.
        let n = w.undo.len();
        w.checkpoint(w.project.clone());
        assert_eq!(w.undo.len(), n);
    }

    #[test]
    fn renders_fades_and_handles() {
        shot("fades", |w| {
            w.project = Project::default();
            let bar = w.bar_ticks();
            let a = w
                .import_clip("Fade-In", None, tone_clip(8.0, 110.0), None)
                .unwrap();
            let b = w
                .import_clip(
                    "Fade-Out",
                    None,
                    tone_clip(8.0, 220.0),
                    Some((usize::MAX, bar)),
                )
                .unwrap();
            w.project.set_fades(a, 2 * bar, 0);
            w.project.set_fades(b, bar / 2, bar + bar / 2);
            w.project.trim_end(b, 3 * bar + bar / 2);
            w.selection = vec![b];
            w.view.zoom(2.0, None);
            w.show_lower = false;
        });
    }

    fn temp_dir(name: &str) -> PathBuf {
        let d = std::env::temp_dir().join(format!("studiocraft-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(&d).unwrap();
        d
    }

    #[test]
    fn save_open_roundtrip_with_other_sample_rate() {
        let tmp = temp_dir("session");
        // Eine Datei außerhalb des Projekts (wird beim Speichern kopiert) …
        let outside = tmp.join("drums.wav");
        let tone = tone_clip(4.0, 110.0);
        studiocraft_engine::export::write_wav_f32(&tone.samples, 48_000, &outside).unwrap();
        let (core, ctl) = studiocraft_engine::engine(48_000, TempoMap::default());
        let mut w = ProjectWindow::new(Project::default(), Some(ctl), String::new());
        let bar = w.bar_ticks();
        w.import_audio(&outside, None).unwrap();
        // … und eine ohne Datei (wird als WAV abgelegt).
        let id = w
            .import_clip(
                "Synth",
                None,
                tone_clip(4.0, 330.0),
                Some((usize::MAX, bar)),
            )
            .unwrap();
        w.project.trim_start(id, bar + w.beat_ticks());
        w.project.set_fades(id, 480, 960);
        w.project.tracks[1].volume_db = -4.5;
        assert!(w.is_modified());

        let file = tmp.join("Song").join("Song.studiocraft");
        w.save_as(&file).unwrap();
        assert!(!w.is_modified());
        assert_eq!(w.title(), "Song – StudioCraft");
        let json = std::fs::read_to_string(&file).unwrap();
        assert!(
            json.contains("\"path\": \"Audio/drums.wav\""),
            "relativer Pfad"
        );
        assert!(json.contains("\"path\": \"Audio/Synth.wav\""));
        assert!(
            tmp.join("Song/Audio/drums.wav").exists() && tmp.join("Song/Audio/Synth.wav").exists()
        );
        drop(core);

        // In einem Fenster mit 44,1 kHz öffnen: Offsets und Längen werden umgerechnet.
        let mut w2 = ProjectWindow::new(Project::default(), None, String::new());
        w2.sample_rate = 44_100;
        w2.open(&file).unwrap();
        assert_eq!(w2.message, None);
        assert!(!w2.is_modified());
        let ev = w2.project.event(id).unwrap();
        let orig = w.project.event(id).unwrap();
        assert_eq!(
            (ev.start, ev.length, ev.fade_in, ev.fade_out),
            (orig.start, orig.length, orig.fade_in, orig.fade_out)
        );
        assert_eq!(orig.offset, 24_000, "eine Viertel bei 48 kHz");
        assert_eq!(ev.offset, 22_050, "eine Viertel bei 44,1 kHz");
        assert_eq!(w2.project.pool[0].sample_rate, 44_100);
        assert_eq!(w2.clips.len(), 2);
        assert_eq!(w2.project.tracks[1].volume_db, -4.5);
    }

    #[test]
    fn open_reports_missing_audio() {
        let tmp = temp_dir("missing");
        let mut w = ProjectWindow::new(Project::default(), None, String::new());
        w.import_clip("Weg", None, tone_clip(1.0, 220.0), None)
            .unwrap();
        let file = tmp.join("P/P.studiocraft");
        w.save_as(&file).unwrap();
        std::fs::remove_file(tmp.join("P/Audio/Weg.wav")).unwrap();
        let mut w2 = ProjectWindow::new(Project::default(), None, String::new());
        w2.open(&file).unwrap();
        assert!(w2.message.as_deref().unwrap().contains("Audio/Weg.wav"));
        assert_eq!(
            w2.project.tracks[0].events.len(),
            1,
            "Event bleibt erhalten"
        );
        assert!(w2.open(&tmp.join("gibt-es-nicht.studiocraft")).is_err());
    }

    #[test]
    fn export_mixdown_covers_locator_range() {
        let tmp = temp_dir("export");
        let mut w = ProjectWindow::new(Project::default(), None, String::new());
        w.import_clip("Ton", None, tone_clip(8.0, 220.0), None)
            .unwrap();
        w.left_locator = w.bar_ticks();
        w.right_locator = 3 * w.bar_ticks(); // 2 Takte = 4 s
        let out = tmp.join("mix.wav");
        w.export_mixdown(&out).unwrap();
        let clip = AudioClip::load_wav(&out).unwrap();
        assert_eq!(clip.frames(), 4 * 48_000);
        assert!(clip.samples.iter().any(|s| s.abs() > 0.5));
    }

    #[test]
    fn ctrl_n_with_changes_asks_first() {
        let mut w = ProjectWindow::new(Project::default(), None, String::new());
        w.import_clip("Ton", None, tone_clip(1.0, 220.0), None)
            .unwrap();
        let mut h = window_harness(w);
        h.key_press_modifiers(egui::Modifiers::COMMAND, Key::N);
        h.run_steps(2);
        assert_eq!(h.state().pending, Some(menu::Action::New));
        assert_eq!(h.state().project.tracks.len(), 1, "noch nichts verworfen");
        if let Ok(dir) = std::env::var("STUDIOCRAFT_SHOTS")
            && let Ok(img) = h.render()
        {
            img.save(Path::new(&dir).join("rueckfrage.png")).unwrap();
        }
        h.get_by_label("Nicht speichern").click();
        h.run_steps(2);
        assert_eq!(h.state().pending, None);
        assert!(h.state().project.tracks.is_empty(), "neues leeres Projekt");
    }

    /// Zweikanal-Aufnahme: links eine Rampe (Frame-Index / 1e6), rechts konstant 0,5.
    fn two_channel_recording(dir: &Path, frames: u64) -> studiocraft_engine::Recording {
        let samples: Vec<f32> = (0..frames).flat_map(|i| [i as f32 / 1e6, 0.5]).collect();
        let path = dir.join("Aufnahme.wav");
        studiocraft_engine::export::write_wav_f32_channels(&samples, 2, 48_000, &path).unwrap();
        studiocraft_engine::Recording {
            path,
            frames,
            channels: 2,
            sample_rate: 48_000,
            first_captured_at: None,
        }
    }

    fn recording_window() -> ProjectWindow {
        let mut w = ProjectWindow::new(Project::default(), None, String::new());
        for (name, ch) in [("Mikro", 0), ("Gitarre", 1)] {
            let t = w.project.add_track(TrackKind::Audio, name);
            w.project.tracks[t].record = true;
            w.project.tracks[t].input = studiocraft_project::TrackInput {
                first: ch,
                stereo: false,
            };
        }
        w.clear_history();
        w
    }

    #[test]
    fn recording_lands_on_armed_tracks_with_their_inputs() {
        let dir = temp_dir("rec-plain");
        let mut w = recording_window();
        let bar = w.bar_ticks();
        // Vier Sekunden ab Takt 2.
        let rec = two_channel_recording(&dir, 4 * 48_000);
        let n = w.place_recording(&rec, 96_000, 0).unwrap();
        assert_eq!(n, 2);
        assert!(!rec.path.exists(), "Rohdatei aufgeräumt");
        for track in &w.project.tracks {
            let ev = &track.events[0];
            assert_eq!((ev.start.0, ev.length), (bar, 2 * bar));
        }
        let mic = &w.clips[&w.project.tracks[0].events[0].clip.unwrap()].samples;
        let gtr = &w.clips[&w.project.tracks[1].events[0].clip.unwrap()].samples;
        assert_eq!(
            &mic[200..202],
            &[100.0 / 1e6, 100.0 / 1e6],
            "Kanal 1 mono auf beide Seiten"
        );
        assert_eq!(&gtr[200..202], &[0.5, 0.5], "Kanal 2");
        assert!(
            dir.join("Aufnahme - Mikro.wav").exists()
                && dir.join("Aufnahme - Gitarre.wav").exists()
        );
        w.undo();
        assert!(
            w.project.tracks.iter().all(|t| t.events.is_empty()),
            "Aufnahme ist ein Undo-Schritt"
        );
    }

    #[test]
    fn cycle_recording_makes_stacked_takes_last_on_top() {
        let dir = temp_dir("rec-cycle");
        let mut w = recording_window();
        w.project.tracks[1].record = false;
        let bar = w.bar_ticks();
        w.cycle = true;
        w.left_locator = bar;
        w.right_locator = 2 * bar; // Cycle Takt 2 bis 3 = 2 s bis 4 s
        // Start bei 0 s, 7 s Aufnahme: 4 s bis zum rechten Locator, dann 2 s und 1 s.
        let rec = two_channel_recording(&dir, 7 * 48_000);
        assert_eq!(w.place_recording(&rec, 0, 0).unwrap(), 3);
        let evs = &w.project.tracks[0].events;
        let spans: Vec<(u64, u64, u64)> = evs
            .iter()
            .map(|e| (e.start.0, e.length, e.offset))
            .collect();
        assert_eq!(
            spans,
            [
                (0, 2 * bar, 0),
                (bar, bar, 192_000),
                (bar, bar / 2, 288_000)
            ]
        );
        assert_eq!(
            evs[2].name, "Mikro 03",
            "jüngster Take liegt oben (zuletzt in der Liste)"
        );
    }

    #[test]
    fn punch_in_and_out_keep_only_the_locator_range() {
        let dir = temp_dir("rec-punch");
        let mut w = recording_window();
        w.project.tracks[1].record = false;
        let bar = w.bar_ticks();
        w.left_locator = bar;
        w.right_locator = 2 * bar;
        w.punch_in = true;
        w.punch_out = true;
        let rec = two_channel_recording(&dir, 8 * 48_000); // Takt 1 bis 5
        assert_eq!(w.place_recording(&rec, 0, 0).unwrap(), 1);
        let ev = &w.project.tracks[0].events[0];
        assert_eq!((ev.start.0, ev.length, ev.offset), (bar, bar, 96_000));
        // Ganz außerhalb: kein Take.
        let rec = two_channel_recording(&dir, 48_000);
        assert_eq!(w.place_recording(&rec, 5 * 96_000, 0).unwrap(), 0);
    }

    #[test]
    fn skipped_frames_before_project_start_are_dropped() {
        let dir = temp_dir("rec-skip");
        let mut w = recording_window();
        w.project.tracks[1].record = false;
        let rec = two_channel_recording(&dir, 48_000);
        assert_eq!(w.place_recording(&rec, 0, 480).unwrap(), 1);
        let ev = &w.project.tracks[0].events[0];
        assert_eq!((ev.start.0, ev.offset), (0, 480), "erste 10 ms fallen weg");
        assert_eq!(ev.length, w.project.tempo.ticks(48_000 - 480, 48_000));
    }

    #[test]
    fn record_without_armed_track_explains_why() {
        let (_core, ctl) = studiocraft_engine::engine(48_000, TempoMap::default());
        let mut w = ProjectWindow::new(Project::default(), Some(ctl), String::new());
        w.project.add_track(TrackKind::Audio, "A");
        w.start_recording();
        assert!(!w.is_recording());
        assert!(w.message.as_deref().unwrap().contains("aufnahmebereit"));
    }

    #[test]
    fn renders_recording_in_progress() {
        let (mut core, ctl) = studiocraft_engine::engine(48_000, TempoMap::default());
        shot("aufnahme", move |w| {
            w.project = Project::default();
            let bar = 4 * TICKS_PER_QUARTER;
            w.import_clip("Playback", None, tone_clip(8.0, 110.0), None)
                .unwrap();
            let id = w
                .import_clip(
                    "Alter Take",
                    None,
                    tone_clip(4.0, 330.0),
                    Some((usize::MAX, bar)),
                )
                .unwrap();
            let _ = id;
            w.project.tracks[1].record = true;
            w.project.tracks[1].name = "Gesang".into();
            w.transport = Some(ctl);
            w.send(Command::Locate(96_000));
            w.send(Command::Record);
            let mut buf = vec![0.0f32; 2 * 4800];
            for _ in 0..25 {
                core.process(&mut buf); // 2,5 s aufnehmen
            }
            w.recording = true;
            w.show_lower = false;
        });
    }

    /// Ein installiertes CLAP-Effekt-Plugin für Tests (ZamComp bevorzugt), sonst `None`.
    fn test_effect(w: &mut ProjectWindow) -> Option<studiocraft_plugin_host::PluginInfo> {
        let list = w.plugin_list().to_vec();
        list.iter()
            .find(|p| p.name.eq_ignore_ascii_case("ZamComp"))
            .or_else(|| {
                list.iter()
                    .find(|p| p.features.iter().any(|f| f == "audio-effect"))
            })
            .cloned()
    }

    #[test]
    fn insert_runs_in_engine_and_undo_removes_it() {
        let (mut core, ctl) = studiocraft_engine::engine(48_000, TempoMap::default());
        let mut w = ProjectWindow::new(Project::default(), Some(ctl), String::new());
        let Some(fx) = test_effect(&mut w) else {
            eprintln!("kein CLAP-Effekt installiert – übersprungen");
            return;
        };
        w.import_clip("Ton", None, tone_clip(2.0, 220.0), None)
            .unwrap();
        w.clear_history();
        let before = w.project.clone();
        w.project.tracks[0]
            .inserts
            .push(studiocraft_project::InsertSlot {
                plugin_id: fx.id.clone(),
                name: fx.name.clone(),
                bypass: false,
                params: Vec::new(),
                state: None,
            });
        w.checkpoint(before);
        w.sync_inserts();
        w.rebuild_song();
        assert_eq!(w.message, None);
        assert_eq!(w.plugins.sent[0].map, [Some(0)]);
        let params = w.plugins.params[&fx.id].clone();
        assert!(!params.is_empty(), "{} hat Parameter", fx.name);

        // Parameter ändern und abspielen: das Signal läuft durch das Plugin.
        let p = &params[0];
        w.project.tracks[0].inserts[0].set_param(p.id, (p.min + p.max) / 2.0);
        w.sync_inserts();
        w.send(Command::Play);
        let mut out = vec![0.0f32; 2 * 2048];
        for _ in 0..4 {
            core.process(&mut out);
        }
        assert!(out.iter().all(|s| s.is_finite()));
        assert!(out.iter().any(|s| s.abs() > 1e-4), "es kommt Signal an");

        // Rückgängig: Insert ist weg, die Instanz kommt zum Zerstören zurück.
        w.undo();
        w.sync_inserts();
        core.process(&mut out);
        if let Some(t) = &mut w.transport {
            t.collect_garbage();
        }
        assert!(w.project.tracks[0].inserts.is_empty());
        assert!(w.plugins.sent[0].slots.is_empty());
    }

    #[test]
    fn missing_plugin_is_reported_not_fatal() {
        let mut w = ProjectWindow::new(Project::default(), None, String::new());
        let t = w.project.add_track(TrackKind::Audio, "A");
        w.project.tracks[t]
            .inserts
            .push(studiocraft_project::InsertSlot {
                plugin_id: "gibt.es.nicht".into(),
                name: "Phantom".into(),
                bypass: false,
                params: Vec::new(),
                state: None,
            });
        w.sync_inserts();
        assert!(
            w.message
                .as_deref()
                .unwrap()
                .contains("Phantom: nicht installiert")
        );
        assert_eq!(w.plugins.sent[0].map, [None]);
    }

    #[test]
    fn renders_inserts_in_inspector() {
        shot("inserts", |w| {
            w.project = Project::default();
            w.import_clip("Gesang", None, tone_clip(8.0, 330.0), None)
                .unwrap();
            if let Some(fx) = test_effect(w) {
                w.project.tracks[0]
                    .inserts
                    .push(studiocraft_project::InsertSlot {
                        plugin_id: fx.id.clone(),
                        name: fx.name.clone(),
                        bypass: false,
                        params: Vec::new(),
                        state: None,
                    });
                w.sync_inserts();
            }
            w.show_lower = false;
        });
    }

    fn melody(w: &mut ProjectWindow, track: usize, start: u64) -> EventId {
        let q = TICKS_PER_QUARTER;
        let id = w.project.create_event(track, start, 8 * q).unwrap();
        let ev = w.project.event_mut(id).unwrap();
        for (i, pitch) in [60u8, 64, 67, 72, 67, 64, 62, 59].into_iter().enumerate() {
            ev.notes.push(studiocraft_project::Note {
                pitch,
                velocity: 100,
                start: (i as u64 * q) as i64,
                length: q - 120,
            });
        }
        id
    }

    #[test]
    fn instrument_track_plays_part_through_plugin() {
        let (mut core, ctl) = studiocraft_engine::engine(48_000, TempoMap::default());
        let mut w = ProjectWindow::new(Project::default(), Some(ctl), String::new());
        let Some(instr) = w.plugin_list().iter().find(|p| p.is_instrument()).cloned() else {
            eprintln!("kein CLAP-Instrument installiert – übersprungen");
            return;
        };
        w.add_instrument_track(Some(instr.clone()));
        assert_eq!(w.project.tracks[0].kind, TrackKind::Instrument);
        melody(&mut w, 0, 0);
        w.cycle = true;
        w.left_locator = 0;
        w.right_locator = 4 * TICKS_PER_QUARTER; // 2 s
        w.send_locators();
        w.send(Command::ToggleCycle);
        w.sync_inserts();
        w.rebuild_song();
        assert_eq!(w.message, None, "{}", instr.name);
        assert!(w.plugins.sent[0].instrument_ok);
        w.send(Command::Play);
        let mut out = vec![0.0f32; 2 * 4096];
        for _ in 0..40 {
            core.process(&mut out); // gut 3 s: einmal über den Cycle-Sprung
        }
        assert!(out.iter().all(|s| s.is_finite()));
        w.stop_transport();
        core.process(&mut out);
        // Zurück: Spur weg, Instrument wird im Hauptthread zerstört.
        w.undo();
        w.sync_inserts();
        core.process(&mut out);
        if let Some(t) = &mut w.transport {
            t.collect_garbage();
        }
        assert!(w.project.tracks.is_empty());
    }

    #[test]
    fn audio_track_shortcut_and_undo() {
        let mut w = ProjectWindow::new(Project::default(), None, String::new());
        w.add_audio_track();
        w.add_audio_track();
        assert_eq!(w.project.tracks[1].name, "Audio 02");
        w.undo();
        assert_eq!(w.project.tracks.len(), 1);
    }

    #[test]
    fn renders_instrument_track_with_part() {
        shot("instrument", |w| {
            w.project = Project::default();
            let instr = w.plugin_list().iter().find(|p| p.is_instrument()).cloned();
            w.add_instrument_track(instr);
            melody(w, 0, 4 * TICKS_PER_QUARTER);
            w.import_clip("Drums", None, tone_clip(8.0, 55.0), None)
                .unwrap();
            w.selected_track = Some(0);
            w.sync_inserts();
            w.show_lower = false;
        });
    }

    /// Bildschirmposition für (absoluter Tick, Tonhöhe) im Key-Editor.
    fn key_pos(h: &Harness<'_, ProjectWindow>, tick: u64, pitch: i32) -> egui::Pos2 {
        let w = h.state();
        let v = w.key_editor.view;
        let g = w.key_editor.grid_rect;
        let x = g.left() + ((tick as f64 - v.scroll_ticks) / TPQ) as f32 * v.px_per_quarter;
        let y = g.top() + (127 - pitch) as f32 * v.key_h - v.scroll_y + v.key_h / 2.0;
        egui::pos2(x, y)
    }

    fn mouse_drag(
        h: &mut Harness<'_, ProjectWindow>,
        a: egui::Pos2,
        b: egui::Pos2,
        modifiers: egui::Modifiers,
    ) {
        let button = |pos, pressed| egui::Event::PointerButton {
            pos,
            button: egui::PointerButton::Primary,
            pressed,
            modifiers,
        };
        h.event(egui::Event::PointerMoved(a));
        h.run_steps(1);
        h.event(button(a, true));
        h.run_steps(1);
        for i in 1..=5 {
            h.event(egui::Event::PointerMoved(a + (b - a) * (i as f32 / 5.0)));
            h.run_steps(1);
        }
        h.event(button(b, false));
        h.run_steps(2);
    }

    fn mouse_click(h: &mut Harness<'_, ProjectWindow>, p: egui::Pos2) {
        let button = |pressed| egui::Event::PointerButton {
            pos: p,
            button: egui::PointerButton::Primary,
            pressed,
            modifiers: egui::Modifiers::NONE,
        };
        h.event(egui::Event::PointerMoved(p));
        h.run_steps(1);
        h.event(button(true));
        h.event(button(false));
        h.run_steps(2);
    }

    #[test]
    fn key_editor_draw_move_resize_velocity_erase_undo() {
        let mut w = ProjectWindow::new(Project::default(), None, String::new());
        w.add_instrument_track(None);
        let q = TICKS_PER_QUARTER;
        let part = w.project.create_event(0, 0, 16 * q).unwrap();
        w.clear_history();
        w.open_key_editor(part);
        w.key_editor.view.scroll_y = (127.0 - 72.0) * w.key_editor.view.key_h;
        let mut h = window_harness(w);
        let notes =
            |h: &Harness<'_, ProjectWindow>| h.state().project.event(part).unwrap().notes.clone();

        // Zeichnen: Note C3 ab Zählzeit 2, auf zwei Zählzeiten aufgezogen.
        h.state_mut().tool = Tool::Draw;
        let a = key_pos(&h, q + 10, 60);
        let b = key_pos(&h, 3 * q - 10, 60);
        mouse_drag(&mut h, a, b, egui::Modifiers::NONE);
        assert_eq!(notes(&h).len(), 1);
        let n = notes(&h)[0];
        assert_eq!((n.pitch, n.start, n.length), (60, q as i64, 2 * q));

        // Verschieben: eine Zählzeit nach rechts, zwei Halbtöne hoch.
        h.state_mut().tool = Tool::Select;
        let a = key_pos(&h, 2 * q, 60);
        let b = key_pos(&h, 3 * q, 62);
        mouse_drag(&mut h, a, b, egui::Modifiers::NONE);
        let n = notes(&h)[0];
        assert_eq!((n.pitch, n.start), (62, 2 * q as i64));

        // Länge: rechten Rand von Zählzeit 5 auf 6 ziehen.
        let a = key_pos(&h, 4 * q, 62) - egui::vec2(2.0, 0.0);
        let b = key_pos(&h, 5 * q, 62);
        mouse_drag(&mut h, a, b, egui::Modifiers::NONE);
        assert_eq!(notes(&h)[0].length, 3 * q);

        // Velocity: halbe Höhe in der Velocity-Spur über der Note.
        let vel = h.state().key_editor.vel_rect;
        let x = key_pos(&h, 3 * q, 62).x;
        mouse_click(&mut h, egui::pos2(x, vel.center().y));
        let v = notes(&h)[0].velocity;
        assert!((55..=72).contains(&v), "Velocity etwa halb: {v}");

        // Löschen mit dem Radiergummi.
        h.state_mut().tool = Tool::Erase;
        let p = key_pos(&h, 3 * q, 62);
        mouse_click(&mut h, p);
        assert!(notes(&h).is_empty());

        // Rückgängig: Note wieder da, dann alles bis zum leeren Part.
        let w = h.state_mut();
        w.undo();
        assert_eq!(w.project.event(part).unwrap().notes.len(), 1);
        for _ in 0..4 {
            w.undo();
        }
        assert!(w.project.event(part).unwrap().notes.is_empty());
    }

    #[test]
    fn renders_key_editor() {
        shot("key-editor", |w| {
            w.project = Project::default();
            w.add_instrument_track(None);
            let part = melody(w, 0, 0);
            w.open_key_editor(part);
            w.key_editor.selection = vec![2, 3];
            w.lower = LowerZone::Editor;
            w.show_lower = true;
        });
    }

    fn rec(pos: u64, key: u8, on: bool) -> studiocraft_engine::midi::RecordedMidi {
        studiocraft_engine::midi::RecordedMidi {
            pos,
            key,
            velocity: 100,
            on,
        }
    }

    #[test]
    fn midi_take_becomes_part_with_notes() {
        let mut w = ProjectWindow::new(Project::default(), None, String::new());
        w.add_instrument_track(None);
        w.project.tracks[0].record = true;
        let q = TICKS_PER_QUARTER;
        let bar = w.bar_ticks();
        // Aufnahme ab Takt 2 (96 000 Frames): C, E; G noch gehalten beim Stopp.
        let take = [
            rec(96_000, 60, true),
            rec(120_000, 60, false),
            rec(120_000, 64, true),
            rec(144_000, 64, false),
            rec(144_000, 67, true),
        ];
        w.place_midi_take(0, &take, 96_000, 168_000);
        let ev = &w.project.tracks[0].events[0];
        assert_eq!((ev.start.0, ev.length), (bar, bar), "Part auf Taktgrenzen");
        let notes: Vec<(u8, i64, u64)> = ev
            .notes
            .iter()
            .map(|n| (n.pitch, n.start, n.length))
            .collect();
        assert_eq!(
            notes,
            [(60, 0, q), (64, q as i64, q), (67, 2 * q as i64, q)]
        );
        w.undo();
        assert!(w.project.tracks[0].events.is_empty());
    }

    #[test]
    fn midi_cycle_mixes_passes_and_punch_trims() {
        let mut w = ProjectWindow::new(Project::default(), None, String::new());
        w.add_instrument_track(None);
        w.project.tracks[0].record = true;
        let bar = w.bar_ticks();
        w.cycle = true;
        w.left_locator = 0;
        w.right_locator = bar; // 2 s = 96 000 Frames
        // Zwei Durchgänge: C im ersten, E im zweiten (Position wieder vorn).
        let take = [
            rec(10_000, 60, true),
            rec(20_000, 60, false),
            rec(30_000, 64, true),
            rec(40_000, 64, false),
        ];
        w.place_midi_take(0, &take, 0, 96_000);
        let ev = &w.project.tracks[0].events[0];
        assert_eq!((ev.start.0, ev.length), (0, bar), "Part = Cycle-Bereich");
        assert_eq!(ev.notes.len(), 2);
        // Punch-In ab Takt 2 ohne Cycle: Noten davor fallen weg.
        let mut w = ProjectWindow::new(Project::default(), None, String::new());
        w.add_instrument_track(None);
        w.project.tracks[0].record = true;
        w.left_locator = bar;
        w.right_locator = 2 * bar;
        w.punch_in = true;
        let take = [
            rec(0, 60, true),
            rec(10_000, 60, false),
            rec(100_000, 62, true),
            rec(110_000, 62, false),
        ];
        w.place_midi_take(0, &take, 0, 120_000);
        let ev = &w.project.tracks[0].events[0];
        assert_eq!(ev.start.0, bar);
        assert_eq!(ev.notes.iter().map(|n| n.pitch).collect::<Vec<_>>(), [62]);
    }

    #[test]
    fn keyboard_follows_armed_then_selected_instrument() {
        let mut w = ProjectWindow::new(Project::default(), None, String::new());
        w.add_audio_track();
        w.add_instrument_track(None);
        w.add_instrument_track(None);
        w.selected_track = Some(0);
        assert_eq!(w.midi_target(), None, "Audio-Spur ist kein Ziel");
        w.selected_track = Some(1);
        assert_eq!(w.midi_target(), Some(1));
        w.project.tracks[2].record = true;
        assert_eq!(w.midi_target(), Some(2), "aufnahmebereit geht vor");
    }

    #[test]
    fn plugin_state_survives_rebuild_and_save_open() {
        let tmp = temp_dir("plugin-state");
        let (mut core, ctl) = studiocraft_engine::engine(48_000, TempoMap::default());
        let mut w = ProjectWindow::new(Project::default(), Some(ctl), String::new());
        let Some(fx) = w
            .plugin_list()
            .iter()
            .find(|p| p.name == "ZamComp")
            .cloned()
        else {
            eprintln!("ZamComp nicht installiert – übersprungen");
            return;
        };
        let slot = |fx: &studiocraft_plugin_host::PluginInfo| studiocraft_project::InsertSlot {
            plugin_id: fx.id.clone(),
            name: fx.name.clone(),
            bypass: false,
            params: Vec::new(),
            state: None,
        };
        w.add_audio_track();
        w.project.tracks[0].inserts.push(slot(&fx));
        w.sync_inserts();
        // Parameter direkt an die Engine (wie aus einem Plugin-Fenster), dann verarbeiten.
        let p = w.plugins.params[&fx.id][0].clone();
        w.send(Command::SetParam(0, 0, p.id, p.max));
        core.process(&mut [0.0; 512]);
        let state_before = w.plugins.sent[0].handles[0]
            .as_ref()
            .unwrap()
            .save_state()
            .unwrap();

        // Zweiter Insert: Kette wird neu gebaut, der erste behält seinen Zustand.
        w.project.tracks[0].inserts.push(slot(&fx));
        w.sync_inserts();
        core.process(&mut [0.0; 512]);
        if let Some(t) = &mut w.transport {
            t.collect_garbage();
        }
        let first = w.plugins.sent[0].handles[0]
            .as_ref()
            .unwrap()
            .save_state()
            .unwrap();
        assert_eq!(first, state_before, "Einstellung nach Umbau erhalten");

        // Speichern und in einem neuen Fenster öffnen.
        let file = tmp.join("P/P.studiocraft");
        w.save_as(&file).unwrap();
        assert!(
            std::fs::read_to_string(&file)
                .unwrap()
                .contains("\"state\"")
        );
        let (_core2, ctl2) = studiocraft_engine::engine(48_000, TempoMap::default());
        let mut w2 = ProjectWindow::new(Project::default(), Some(ctl2), String::new());
        w2.open(&file).unwrap();
        w2.sync_inserts();
        let loaded = w2.plugins.sent[0].handles[0]
            .as_ref()
            .unwrap()
            .save_state()
            .unwrap();
        assert_eq!(loaded, state_before, "Zustand nach Öffnen identisch");
        assert_eq!(w2.message, None);
    }

    #[test]
    fn renders_project_window() {
        shot("projektfenster", |_| {});
    }

    #[test]
    fn renders_zoomed_with_cycle_and_selection() {
        shot("zoom-cycle-auswahl", |w| {
            w.view.zoom(3.0, None);
            w.cycle = true;
            w.left_locator = 4 * w.bar_ticks();
            w.right_locator = 8 * w.bar_ticks();
            w.cursor = 5 * w.bar_ticks() + w.beat_ticks();
            w.tool = Tool::Split;
            w.selected_track = Some(3);
            let id = w.project.tracks[3].events[1].id;
            w.selection = vec![id];
            w.project.tracks[1].mute = true;
            w.project.tracks[2].solo = true;
            w.project.tracks[0].record = true;
            w.view.scroll_ticks = (3 * w.bar_ticks()) as f64;
        });
    }
}
