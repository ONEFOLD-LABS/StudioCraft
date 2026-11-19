//! Oberfläche von StudioCraft (egui), aufgebaut nach `docs/spec.md`,
//! Abschnitte „Fenster-Architektur“, „Werkzeuge“, „Raster, Snap und Bearbeitung“.
//!
//! Prototyp-Stand: Projektfenster mit Werkzeugzeile, Inspector, Spurliste,
//! Lineal, Event-Anzeige und Transportleiste.
// Kein unwrap/expect/panic außerhalb von Tests (AGENTS.md „Robustness“, Issue #49).
#![cfg_attr(
    not(test),
    deny(
        clippy::unwrap_used,
        clippy::expect_used,
        clippy::panic,
        clippy::todo,
        clippy::unimplemented,
        clippy::unreachable
    )
)]

mod automation_panel;
mod automation_write;
mod autosave;
mod colors;
mod devices;
mod eq_ui;
mod fades_ui;
mod focus;
mod folders_ui;
mod icons;
mod infoline;
mod inserts;
mod io_ui;
mod keyeditor;
mod keymap;
pub mod keypad;
mod latency;
mod layouts_ui;
mod macros;
mod markers_ui;
mod menu;
mod midiout_ui;
mod mixer;
mod noteinfo;
mod panel_undo;
mod quantize;
mod recording;
mod rightzone;
mod routing;
mod sampleeditor;
mod session;
pub mod settings;
mod shortcuts;
mod snap;
mod tempo_ui;
pub mod theme;
mod timeline;
pub mod titlebar;
mod tools;
pub mod waveform;
mod windows;

use std::{
    collections::HashMap,
    path::{Path, PathBuf},
    sync::Arc,
};

use egui::{Color32, Rect, RichText, Ui};
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

/// Standardgrößen der Zonen in Punkten (gilt, solange im Projekt 0 steht).
const LEFT_DEFAULT: f32 = 200.0 + mixer::COLUMN_W;
const RIGHT_DEFAULT: f32 = 240.0;
const LOWER_DEFAULT: f32 = 300.0;

/// Gespeicherte Zonengröße oder, bei 0, der Standard.
fn layout_size(saved: f32, default: f32) -> f32 {
    if saved > 0.0 { saved } else { default }
}

/// Der Zustand des Projektfensters.
pub struct ProjectWindow {
    pub project: Project,
    transport: Option<EngineControl>,
    /// Text für die Statuszeile (Audio-Gerät oder Fehler).
    pub audio_status: String,
    tool: Tool,
    /// Zone mit dem Tastaturfokus (Fokus-Regel).
    focus: focus::Zone,
    snap: bool,
    snap_mode: tools::SnapMode,
    grid: Grid,
    /// „An Zoom anpassen“: musikalisches Raster beim Einzoomen verfeinern.
    grid_zoom: bool,
    /// Offene Farbpalette (Klick auf die Farbleiste im Spurkopf) und ihr Alter in Bildern.
    color_popup: Option<(colors::Target, egui::Pos2)>,
    color_popup_age: u8,
    /// Ausgabe-Thread für externe MIDI-Geräte (nur mit Engine).
    midi_out: Option<studiocraft_engine::midi_out::MidiOutThread>,
    /// Zuletzt gelesene MIDI-Ausgänge des Systems (Inspector einer MIDI-Spur).
    midi_out_ports: Option<Vec<String>>,
    /// Taste F: Projektfenster und Editoren folgen der Wiedergabe.
    follow_playback: bool,
    /// Eigene Fenster für Mixer und Editoren.
    windows: windows::Windows,
    /// Inspector-Spalte in der linken Zone (Alt+I); die Zone selbst ist `show_inspector`
    /// (Alt+L).
    show_inspector_column: bool,
    /// Zählt angewandte Layouts: Teil der Panel-Kennungen, damit gespeicherte Größen nach
    /// dem Laden oder Abrufen eines Layouts greifen.
    layout_gen: u32,
    /// Größe der linken, rechten und unteren Zone im letzten Bild.
    zone_sizes: [Option<f32>; 3],
    /// Eingabefeld für den Namen eines neuen Layouts (Menü Ansicht → Layouts).
    layout_name: String,
    /// Modus der Objektauswahl beim Ziehen am Rand (Rechtsklick auf das Werkzeug).
    resize_mode: tools::ResizeMode,
    /// Wiedergabe-Werkzeug bei gedrückter Maustaste (Spec „Werkzeuge“, Taste 9).
    audition: Option<timeline::Audition>,
    /// Werkzeugpalette am Mauszeiger (Rechtsklick in die Event-Anzeige) und ihr Alter in
    /// Bildern, damit der öffnende Klick sie nicht gleich wieder schließt.
    tool_palette: Option<egui::Pos2>,
    tool_palette_age: u8,
    /// Tastaturbelegung (Standard plus eigene Änderungen) und der Dialog dazu.
    keymap: keymap::Keymap,
    key_dialog: keymap::KeyDialog,
    /// Nummernblock-Drücke aus der App (ohne: Nummernblock wie Zahlenreihe).
    keypad: Option<keypad::KeypadTap>,
    /// Kurveneditor eines Fades (Doppelklick auf den Fade-Griff).
    fade_editor: Option<fades_ui::FadeEditor>,
    /// Crossfade-Editor (Doppelklick auf einen Fade, der zu einem Crossfade gehört).
    crossfade_editor: Option<fades_ui::CrossfadeEditor>,
    /// Dialog „Wiederholen“ (Strg+K).
    repeat_dialog: shortcuts::RepeatDialog,
    /// Sample-Editor in der unteren Zone.
    sample_editor: sampleeditor::SampleEditor,
    /// Globaler Quantisierungswert mit Swing und Stärke (Taste Q).
    quantize: studiocraft_project::Quantize,
    /// Automation schreiben: Modus, Ausdünnen, laufende Durchgänge.
    auto_writer: automation_write::AutomationWriter,
    /// Quantisierungs-Panel offen.
    show_quantize: bool,
    show_inspector: bool,
    /// Info-Zeile unter der Werkzeugzeile sichtbar.
    show_info: bool,
    /// Transportleiste sichtbar (F2).
    show_transport: bool,
    /// Statuszeile am unteren Fensterrand sichtbar.
    show_status: bool,
    clipboard: shortcuts::Clipboard,
    /// Rechte Zone (Medien, Plugins) sichtbar.
    show_right: bool,
    right: rightzone::RightZone,
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
    /// Peak-Hold je Spur und Kanal (Wert, Alter in Sekunden) und Übersteuerung
    /// (bis zum Klick); letzter Eintrag: Ausgangsbus.
    meter_hold: Vec<[(f32, f32); 2]>,
    clipped: Vec<bool>,
    /// Undo- und Redo-Verlauf (ganze Projektstände; Spec „Undo“).
    undo: Vec<Project>,
    redo: Vec<Project>,
    /// Schnappschuss zu Beginn einer Geste in Mixer, Inspector usw. (panel_undo.rs).
    panel_edit_before: Option<Project>,
    /// Lage der Zonen im letzten Bild (für den Schnappschuss beim Drücken).
    last_zones: focus::ZoneRects,
    /// In einem eigenen (nativen) Fenster ist gerade die Maus gedrückt.
    window_pointer_down: bool,
    /// Projektstand zu Beginn eines Ziehvorgangs.
    drag_before: Option<Project>,
    /// Lage der Event-Anzeige im letzten Frame (für Tests mit Mauseingaben).
    lanes_rect: egui::Rect,
    /// Projektdatei, unter der zuletzt gespeichert oder geöffnet wurde.
    project_path: Option<PathBuf>,
    /// Dialog „Audio-Anschlüsse“ offen, Stand vor einer laufenden Eingabe darin, Kanäle des
    /// Ausgabegeräts.
    io_dialog: bool,
    io_before: Option<Project>,
    output_channels: u16,
    /// Latenzausgleich (Issue #51).
    latency: latency::LatencyState,
    /// Automatische Sicherung und Wiederherstellung (Issue #52).
    autosave: autosave::Autosave,
    /// Zuletzt gespeicherter Stand, um ungesicherte Änderungen zu erkennen.
    saved: Option<Project>,
    /// Befehl, der auf die Rückfrage „Speichern?“ wartet.
    pending: Option<menu::Action>,
    /// Schließen ist bestätigt (keine weitere Rückfrage).
    allow_close: bool,
    last_title: String,
    /// Wer den Fensterrahmen zeichnet (siehe [`titlebar`]).
    window_frame: craft_window::policy::Frame,
    /// macOS: zuletzt an AppKit gemeldetes `movable` der Titelleiste.
    #[cfg(target_os = "macos")]
    mac_movable: Option<bool>,
    /// Audio-Host, mit dem auch der Eingang geöffnet wird.
    pub audio_host: Option<String>,
    input: Option<studiocraft_engine::AudioInput>,
    recorder: Option<studiocraft_engine::Recorder>,
    input_status: String,
    recording: bool,
    punch_in: bool,
    punch_out: bool,
    overruns_at_start: u64,
    /// Frames des Vorzählers der laufenden Aufnahme (0 = keiner).
    count_in_frames: u64,
    /// Stand vor einer Änderung im Menü „Metronom“ (ein Undo-Schritt je Geste).
    click_before: Option<Project>,
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
    /// Zuletzt gespielte, nicht aufgenommene Noten (nachträgliche Aufnahme).
    retro: std::collections::VecDeque<studiocraft_engine::midi::RecordedMidi>,
    /// Die laufende Aufnahme schreibt auch Audio.
    audio_recording: bool,
    metronome: bool,
    /// Bereichsauswahl (Werkzeug 2): erste/letzte Spur, Anfang/Ende in Ticks.
    range: Option<(usize, usize, u64, u64)>,
    /// Erscheinungsbild (Dunkel/Hell/System); installiert beim ersten Frame.
    theme: egui::ThemePreference,
    /// Stopp-Verhalten (Einstellungen): zurück zur Startposition, Ziel von zweimal Stopp.
    return_on_stop: bool,
    stop_twice: settings::StopTarget,
    /// Position, an der die Wiedergabe zuletzt begann.
    play_from: u64,
    /// Gewählte Audio-Geräte (Einstellungen) und MIDI-Eingänge an/aus.
    audio_choice: studiocraft_engine::DeviceChoice,
    midi_inputs_on: bool,
    device_dialog: devices::DeviceDialog,
    /// Gerätewechsel, den die App ausführen soll.
    device_request: Option<studiocraft_engine::DeviceChoice>,
    theme_installed: bool,
    /// Zuletzt an die Engine gemeldetes Tempo.
    sent_tempo: Option<studiocraft_project::TempoMap>,
    /// Projektstand vor einer laufenden Tempo-Eingabe (ein Undo-Schritt pro Eingabe).
    tempo_before: Option<Project>,
    /// Stand vor einer Eingabe im Inspector der Tempospur (ein Undo-Schritt je Eingabe).
    tempo_track_before: Option<Project>,
}

/// Inhalt der unteren Zone (Spec „Fenster-Architektur“).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum LowerZone {
    Mixer,
    Editor,
    SampleEditor,
}

impl ProjectWindow {
    pub fn new(project: Project, transport: Option<EngineControl>, audio_status: String) -> Self {
        let bar = project.tempo.signature.bar_ticks();
        let sample_rate = transport.as_ref().map_or(48_000, |t| t.sample_rate);
        let prefs = settings::Settings::load();
        let mut w = Self {
            project,
            transport,
            audio_status,
            tool: Tool::Select,
            focus: focus::Zone::Project,
            snap: true,
            snap_mode: tools::SnapMode::Grid,
            grid: Grid::Beat,
            grid_zoom: false,
            color_popup: None,
            color_popup_age: 0,
            midi_out: None,
            midi_out_ports: None,
            follow_playback: true,
            windows: windows::Windows::new(prefs.windows),
            show_inspector_column: true,
            layout_gen: 0,
            zone_sizes: [None; 3],
            layout_name: String::new(),
            resize_mode: tools::ResizeMode::default(),
            audition: None,
            tool_palette: None,
            tool_palette_age: 0,
            keymap: keymap::Keymap::default(),
            key_dialog: keymap::KeyDialog::default(),
            keypad: None,
            fade_editor: None,
            crossfade_editor: None,
            repeat_dialog: shortcuts::RepeatDialog::default(),
            sample_editor: sampleeditor::SampleEditor::default(),
            quantize: studiocraft_project::Quantize::default(),
            auto_writer: automation_write::AutomationWriter::default(),
            show_quantize: false,
            show_inspector: true,
            show_info: true,
            show_transport: true,
            show_status: true,
            clipboard: shortcuts::Clipboard::default(),
            show_right: false,
            right: rightzone::RightZone::default(),
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
            meter_hold: Vec::new(),
            clipped: Vec::new(),
            undo: Vec::new(),
            redo: Vec::new(),
            panel_edit_before: None,
            last_zones: focus::ZoneRects::default(),
            window_pointer_down: false,
            drag_before: None,
            lanes_rect: egui::Rect::NOTHING,
            project_path: None,
            io_dialog: false,
            io_before: None,
            output_channels: 2,
            latency: latency::LatencyState::default(),
            autosave: autosave::Autosave::new(prefs.autosave_minutes, prefs.autosave_keep),
            saved: None,
            pending: None,
            allow_close: false,
            last_title: String::new(),
            window_frame: craft_window::policy::Frame::Native,
            #[cfg(target_os = "macos")]
            mac_movable: None,
            audio_host: None,
            input: None,
            recorder: None,
            input_status: String::new(),
            recording: false,
            punch_in: false,
            punch_out: false,
            overruns_at_start: 0,
            count_in_frames: 0,
            click_before: None,
            plugins: inserts::PluginState::default(),
            lower: LowerZone::Mixer,
            key_editor: keyeditor::KeyEditor::default(),
            midi_inputs: None,
            midi_status: String::new(),
            midi_target_sent: None,
            midi_take: Vec::new(),
            retro: std::collections::VecDeque::new(),
            audio_recording: false,
            metronome: false,
            range: None,
            theme: prefs.theme,
            return_on_stop: prefs.return_on_stop,
            stop_twice: prefs.stop_twice,
            play_from: 0,
            audio_choice: prefs.audio.clone(),
            midi_inputs_on: prefs.midi_inputs,
            device_dialog: devices::DeviceDialog::default(),
            device_request: None,
            theme_installed: false,
            sent_tempo: None,
            tempo_before: None,
            tempo_track_before: None,
        };
        w.apply_layout(w.project.layout.clone());
        // MIDI-Ausgang: der Thread übernimmt den Puffer, den der Audio-Thread füllt.
        if let Some(rx) = w.transport.as_mut().and_then(|t| t.midi_out.take()) {
            match studiocraft_engine::midi_out::MidiOutThread::spawn(rx) {
                Ok(t) => w.midi_out = Some(t),
                Err(e) => w.message = Some(e),
            }
        }
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
            self.stop();
        } else {
            self.play();
        }
    }

    fn toggle_cycle(&mut self) {
        self.cycle = !self.cycle;
        self.send(Command::ToggleCycle);
    }

    /// Lädt eine WAV-Datei in den Pool und legt ein Event an. Ohne Ziel
    /// (Spur, Position) entsteht eine neue Audio-Spur ab Takt 1.
    pub fn import_audio(
        &mut self,
        path: &Path,
        target: Option<(usize, u64)>,
    ) -> Result<EventId, String> {
        let clip = AudioClip::load(path).map_err(|e| format!("{}: {e}", path.display()))?;
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
        // Geteilte Kopien: geänderten Inhalt an die ganze Gruppe weitergeben.
        self.project.sync_shared(&before);
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
            let keep = self.window_state();
            self.redo.push(std::mem::replace(&mut self.project, prev));
            self.restore_window_state(keep);
            self.after_history_step();
        }
    }

    pub fn redo(&mut self) {
        if let Some(next) = self.redo.pop() {
            let keep = self.window_state();
            self.undo.push(std::mem::replace(&mut self.project, next));
            self.restore_window_state(keep);
            self.after_history_step();
        }
    }

    /// Fensterlayout und benannte Layouts gehören nicht zum Undo-Verlauf: Undo und Redo
    /// lassen sie, wie sie gerade sind.
    fn window_state(
        &self,
    ) -> (
        studiocraft_project::WindowLayout,
        Vec<studiocraft_project::NamedLayout>,
    ) {
        (self.project.layout.clone(), self.project.layouts.clone())
    }

    fn restore_window_state(
        &mut self,
        (layout, layouts): (
            studiocraft_project::WindowLayout,
            Vec<studiocraft_project::NamedLayout>,
        ),
    ) {
        self.project.layout = layout;
        self.project.layouts = layouts;
    }

    /// Übernimmt `layout` in die Zonen (nach dem Laden oder Abrufen eines Layouts). Die
    /// Panels bekommen neue Kennungen, damit die gespeicherten Größen greifen.
    pub(crate) fn apply_layout(&mut self, layout: studiocraft_project::WindowLayout) {
        self.show_inspector = layout.left;
        self.show_inspector_column = layout.inspector;
        self.show_lower = layout.lower;
        self.show_right = layout.right;
        self.show_transport = layout.transport;
        self.project.layout = layout;
        self.layout_gen = self.layout_gen.wrapping_add(1);
    }

    /// Schreibt Sichtbarkeit und Größen der Zonen ins Projekt, wenn sie sich geändert haben.
    /// Eine Größe zählt nur, wenn sie sich bei gedrückter Maustaste ändert (Trenner ziehen):
    /// Zonen, die mit ihrem Inhalt wachsen, machen ein Projekt nicht „geändert“.
    fn capture_layout(&mut self, zones: &focus::ZoneRects, dragging: bool) {
        let measured = [
            zones
                .left
                .filter(|_| self.show_inspector_column)
                .map(|r| r.width()),
            zones.right.map(|r| r.width()),
            zones.lower.map(|r| r.height()),
        ];
        let old = self.project.layout.clone();
        let mut sizes = [old.left_width, old.right_width, old.lower_height];
        for (k, m) in measured.iter().enumerate() {
            if let (Some(m), Some(prev)) = (m, self.zone_sizes[k])
                && dragging
                && (m - prev).abs() > 0.5
            {
                sizes[k] = m.round();
            }
        }
        self.zone_sizes = measured;
        let new = studiocraft_project::WindowLayout {
            left: self.show_inspector,
            inspector: self.show_inspector_column,
            lower: self.show_lower,
            right: self.show_right,
            transport: self.show_transport,
            left_width: sizes[0],
            right_width: sizes[1],
            lower_height: sizes[2],
        };
        if new != old {
            self.project.layout = new;
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

    /// Erscheinungsbild wählen und für den nächsten Start merken.
    pub(crate) fn set_theme(&mut self, ctx: &egui::Context, pref: egui::ThemePreference) {
        self.theme = pref;
        theme::install(ctx, pref);
        self.save_settings();
    }

    /// Einstellungen für den nächsten Start merken (nur in der App, nicht in Tests: dort ist
    /// `persist` aus, wie bei der Tastaturbelegung).
    pub(crate) fn save_settings(&mut self) {
        if !self.key_dialog.persist {
            return;
        }
        let s = settings::Settings {
            theme: self.theme,
            return_on_stop: self.return_on_stop,
            stop_twice: self.stop_twice,
            audio: self.audio_choice.clone(),
            midi_inputs: self.midi_inputs_on,
            windows: self.windows.rects,
            autosave_minutes: self.autosave.minutes,
            autosave_keep: self.autosave.keep,
        };
        if let Err(e) = s.save() {
            self.message = Some(format!("Einstellung nicht gespeichert: {e}"));
        }
    }

    /// Wiedergabe ab dem Cursor; merkt die Startposition für das Stopp-Verhalten.
    pub(crate) fn play(&mut self) {
        self.send_programs();
        self.play_from = self.cursor;
        self.send(Command::Play);
    }

    /// Stopp (Spec „Stopp-Verhalten“): Läuft der Transport, hält er an und springt auf Wunsch
    /// zur Startposition zurück; steht er schon, springt Stopp zum Anfang bzw. linken Locator.
    pub(crate) fn stop(&mut self) {
        if self.is_playing() || self.recording {
            self.stop_transport();
            if self.return_on_stop {
                self.locate(self.play_from);
            }
        } else {
            let to = match self.stop_twice {
                settings::StopTarget::Start => 0,
                settings::StopTarget::LeftLocator => self.left_locator,
            };
            self.locate(to);
        }
    }

    fn toggle_metronome(&mut self) {
        self.metronome = !self.metronome;
        self.send(Command::SetMetronome(self.metronome));
    }

    /// Tempo und Taktart an die Engine geben, wenn sie sich geändert haben
    /// (auch nach Undo oder Öffnen eines Projekts).
    fn sync_tempo(&mut self) {
        if self.sent_tempo.as_ref() == Some(&self.project.tempo) {
            return;
        }
        let tempo = self.project.tempo.clone();
        self.sent_tempo = Some(tempo.clone());
        if let Some(t) = &mut self.transport {
            t.tempo = tempo.clone();
        }
        // Das Metronom bekommt die Tempo-Map mit dem neu gebauten Song (`dirty`).
        self.send_locators();
        self.dirty = true;
    }

    /// Tempo, Taktart und Metronom in der Transportleiste.
    fn tempo_controls(&mut self, ui: &mut Ui) {
        let mut tempo = self.project.tempo.clone();
        let mut editing = false;
        let mut changed = false;
        let l = ui.label("Tempo");
        if self.project.tempo.varies() {
            // Tempospur aktiv: das Tempo am Cursor, bearbeitet wird in der Tempospur.
            let bpm = self.project.tempo.bpm_at(self.cursor);
            ui.add(egui::Label::new(
                egui::RichText::new(format!("{bpm:.1}")).monospace(),
            ))
            .labelled_by(l.id)
            .on_hover_text("Tempo am Cursor (aus der Tempospur)");
        } else {
            let r = ui
                .add(
                    egui::DragValue::new(&mut tempo.bpm)
                        .range(20.0..=300.0)
                        .speed(0.2)
                        .fixed_decimals(1),
                )
                .labelled_by(l.id)
                .on_hover_text("Tempo in BPM (ziehen oder eintippen)");
            changed |= r.changed();
            editing |= r.dragged() || r.has_focus();
        }
        if ui
            .selectable_label(self.project.tempo.use_track, "Spur")
            .on_hover_text("Tempospur an/aus (aus: festes Tempo)")
            .clicked()
        {
            self.toggle_tempo_track();
        }
        let takt = ui.label("Takt");
        let r = ui
            .add(egui::DragValue::new(&mut tempo.signature.numerator).range(1..=16))
            .labelled_by(takt.id);
        changed |= r.changed();
        editing |= r.dragged() || r.has_focus();
        ui.label("/");
        egui::ComboBox::from_id_salt("taktart-nenner")
            .width(40.0)
            .selected_text(tempo.signature.denominator.to_string())
            .show_ui(ui, |ui| {
                for d in [2u32, 4, 8, 16] {
                    changed |= ui
                        .selectable_value(&mut tempo.signature.denominator, d, d.to_string())
                        .changed();
                }
            })
            .response
            .labelled_by(takt.id);
        if changed {
            // Immer vom Stand vor der Eingabe aus umrechnen: kein Rundungsfehler beim Ziehen.
            let base = self
                .tempo_before
                .get_or_insert_with(|| self.project.clone())
                .clone();
            let mut map = base.tempo.clone();
            map.bpm = tempo.bpm.clamp(20.0, 300.0);
            map.signature = tempo.signature;
            let mut project = base;
            project.set_tempo_map(map);
            self.project = project;
        }
        if !editing && let Some(before) = self.tempo_before.take() {
            self.checkpoint(before);
        }
        let accent = theme::tokens(ui.ctx()).accent;
        if icons::icon_button(
            ui,
            icons::Icon::Metronome,
            self.metronome,
            Some(accent),
            "Metronom (C)",
        )
        .clicked()
        {
            self.toggle_metronome();
        }
    }

    /// P: Locatoren auf die Bereichsauswahl bzw. die ausgewählten Events setzen.
    pub(crate) fn locators_to_selection(&mut self) {
        let span = match self.range {
            Some((_, _, a, b)) => Some((a, b)),
            None => {
                let evs: Vec<_> = self
                    .selection
                    .iter()
                    .filter_map(|&id| self.project.event(id))
                    .collect();
                let a = evs.iter().map(|e| e.start.0).min();
                let b = evs.iter().map(|e| e.end()).max();
                a.zip(b)
            }
        };
        if let Some((a, b)) = span.filter(|(a, b)| b > a) {
            self.left_locator = a;
            self.right_locator = b;
            self.send_locators();
        }
    }

    pub(crate) fn delete_selection(&mut self) {
        if let Some((r0, r1, a, b)) = self.range.take() {
            let before = self.project.clone();
            self.project.delete_range(r0..=r1, a, b);
            self.selection
                .retain(|&id| self.project.event(id).is_some());
            self.checkpoint(before);
            return;
        }
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
        if self.transport.is_none() {
            return;
        }
        let samples: HashMap<ClipId, Arc<[f32]>> = self
            .clips
            .iter()
            .map(|(&id, c)| (id, c.samples.clone()))
            .collect();
        let song = Song::build(&self.project, &samples, self.sample_rate);
        // Latenzausgleich passend zum neuen Song (nur bei Änderung neu angelegt).
        self.sync_constrain();
        self.sync_delays(&song);
        if let Some(out) = &self.midi_out {
            out.set_ports(song.midi_ports.clone());
        }
        self.send(Command::SetSong(Arc::new(song)));
    }

    fn sync_from_engine(&mut self) {
        if let Some(t) = &self.transport {
            self.cursor = t.position(t.status.cursor()).0;
        }
    }

    fn handle_keys(&mut self, ui: &Ui) {
        // Zuerst der Nummernblock: er nimmt seine Ereignisse, bevor die Tastenbelegung sie sieht.
        self.handle_keypad(ui);
        self.handle_window_keys(ui);
    }

    /// Tastenbelegung für die Eingaben von `ui`: das Hauptfenster und jedes eigene Fenster
    /// (dort kommen die Tasten in dessen eigener Eingabe an).
    pub(crate) fn handle_window_keys(&mut self, ui: &Ui) {
        if ui.ctx().egui_wants_keyboard_input() {
            return;
        }
        // `*` (ohne Nummernblock-Tap auch der Nummernblock): Aufnahme. egui hat dafür keine eigene Taste.
        let star = ui.input(|i| {
            i.events
                .iter()
                .any(|e| matches!(e, egui::Event::Text(t) if t == "*"))
        });
        if star {
            self.toggle_record();
        }
        self.dispatch_keys(ui);
    }

    /// Zeichnet das ganze Projektfenster.
    pub fn ui(&mut self, ui: &mut Ui) {
        if !self.theme_installed {
            theme::install(ui.ctx(), self.theme);
            self.theme_installed = true;
        }
        self.panel_edit_start(ui.ctx());
        self.sync_from_engine();
        if let Some(t) = &mut self.transport {
            t.collect_garbage();
        }
        self.sync_midi();
        // Automatische Sicherung nur in einer Sitzung der App (`begin_session`), nicht in Tests.
        if self.autosave.dir.is_some() {
            self.tick_autosave(std::time::Instant::now());
        }
        self.poll_latency();
        if self.idle_plugins() {
            // Plugin-Fenster brauchen ihre Timer auch ohne Mausbewegung.
            ui.ctx()
                .request_repaint_after(std::time::Duration::from_millis(15));
        }
        self.handle_keys(ui);
        let dt_s = ui.input(|i| i.unstable_dt);
        mixer::update_meters(self, dt_s);
        let dt = dt_s * 1000.0;
        self.frame_ms = if self.frame_ms == 0.0 {
            dt
        } else {
            self.frame_ms * 0.95 + dt * 0.05
        };

        self.resize_edges(ui.ctx());
        let mut top = egui::Panel::top("menue");
        if self.draws_title_bar() {
            top = top.exact_size(32.0);
        }
        top.show(ui, |ui| self.top_bar(ui));
        egui::Panel::top("werkzeugzeile").show(ui, |ui| self.toolbar(ui));
        if self.show_info {
            egui::Panel::top("info-zeile").show(ui, |ui| infoline::show(self, ui));
        }
        // Zuerst angelegt, also ganz unten: die Statuszeile unter der Transportleiste.
        if self.show_status {
            egui::Panel::bottom("statuszeile")
                .exact_size(22.0)
                .show(ui, |ui| self.status_line(ui));
        }
        if self.show_transport {
            egui::Panel::bottom("transportleiste").show(ui, |ui| self.transport_bar(ui));
        }
        let mut zones = focus::ZoneRects::default();
        if self.show_lower {
            let r = egui::Panel::bottom(("untere-zone", self.layout_gen))
                .resizable(true)
                .default_size(layout_size(self.project.layout.lower_height, LOWER_DEFAULT))
                .min_size(200.0)
                .show(ui, |ui| {
                    // Reiter am unteren Rand der Zone, Inhalt darüber (docs/ui-parity.md).
                    ui.with_layout(egui::Layout::bottom_up(egui::Align::LEFT), |ui| {
                        ui.horizontal(|ui| {
                            ui.selectable_value(&mut self.lower, LowerZone::Mixer, "Mixer");
                            ui.selectable_value(&mut self.lower, LowerZone::Editor, "Key-Editor");
                            ui.selectable_value(
                                &mut self.lower,
                                LowerZone::SampleEditor,
                                "Sample-Editor",
                            );
                            if self.lower == LowerZone::Editor
                                && let Some(ev) =
                                    self.key_editor.part.and_then(|p| self.project.event(p))
                            {
                                ui.separator();
                                ui.weak(format!("{} · {} Noten", ev.name, ev.notes.len()));
                            }
                        });
                        ui.separator();
                        ui.with_layout(egui::Layout::top_down(egui::Align::LEFT), |ui| {
                            let zone = self.lower;
                            if self.in_own_window(zone) {
                                self.own_window_placeholder(ui, zone);
                                return;
                            }
                            match zone {
                                LowerZone::Mixer => mixer::show(self, ui),
                                LowerZone::Editor => keyeditor::show(self, ui),
                                LowerZone::SampleEditor => sampleeditor::show(self, ui),
                            }
                        });
                    });
                });
            zones.lower = Some(r.response.rect);
        }
        if self.show_right {
            let r = egui::Panel::right(("rechte-zone", self.layout_gen))
                .resizable(true)
                .default_size(layout_size(self.project.layout.right_width, RIGHT_DEFAULT))
                .show(ui, |ui| rightzone::show(self, ui));
            zones.right = Some(r.response.rect);
        }
        if self.show_inspector {
            // Linke Zone in zwei Spalten: Kanalzug der gewählten Spur, daneben der
            // Inspector (docs/ui-parity.md, „Linke Zone“).
            // Ohne Inspector-Spalte bleibt nur der Kanalzug: schmale Zone mit eigener Kennung.
            let column = self.show_inspector_column;
            let (default, min) = if column {
                (
                    layout_size(self.project.layout.left_width, LEFT_DEFAULT),
                    LEFT_DEFAULT,
                )
            } else {
                (mixer::COLUMN_W + 8.0, mixer::COLUMN_W + 8.0)
            };
            let r = egui::Panel::left(("inspector", column, self.layout_gen))
                .resizable(column)
                .default_size(default)
                .min_size(min)
                .show(ui, |ui| {
                    let height = ui.available_height();
                    ui.horizontal_top(|ui| {
                        ui.vertical(|ui| {
                            ui.set_min_height(height);
                            mixer::channel_column(self, ui);
                        });
                        if column {
                            ui.separator();
                            ui.vertical(|ui| {
                                egui::ScrollArea::vertical().show(ui, |ui| self.inspector(ui));
                            });
                        }
                    });
                });
            zones.left = Some(r.response.rect);
        }
        let r = egui::CentralPanel::default().show(ui, |ui| timeline::show(self, ui));
        zones.project = Some(r.response.rect);
        let dragging = ui.input(|i| i.pointer.primary_down());
        self.capture_layout(&zones, dragging);
        self.update_focus(ui.ctx(), zones);
        self.last_zones = zones;
        quantize::panel(self, ui.ctx());
        self.color_popup(ui.ctx());
        self.tool_palette(ui.ctx());
        self.key_commands_dialog(ui.ctx());
        self.repeat_dialog(ui.ctx());
        self.io_dialog(ui.ctx());
        self.recovery_dialog(ui.ctx());
        self.fade_editor_dialog(ui.ctx());
        self.crossfade_editor_dialog(ui.ctx());
        self.device_dialog(ui.ctx());
        self.show_windows(ui.ctx());
        self.panel_edit_end(ui.ctx());

        self.sync_tempo();
        let playing = self.is_playing();
        self.automation_write(ui.ctx(), playing);
        if self.dirty {
            self.dirty = false;
            self.sync_monitor_input();
            self.sync_inserts();
            self.rebuild_song();
        }

        self.confirm_dialog(ui);
        self.sync_title_bar(ui.ctx());
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

    /// Werkzeugzeile (docs/ui-parity.md, „Werkzeugzeile“): links globale Zustände und die
    /// Transport-Kurzform, in der Mitte Werkzeuge und Snap, rechts die Zonen-Schalter.
    fn toolbar(&mut self, ui: &mut Ui) {
        use icons::{Icon, icon_button};
        let t = theme::tokens(ui.ctx());
        let row = ui.available_rect_before_wrap();
        ui.horizontal(|ui| {
            ui.spacing_mut().item_spacing.x = 2.0;
            self.global_states(ui);
            icons::group_gap(ui);
            // Transport-Kurzform
            if icon_button(ui, Icon::ToStart, false, None, "Zum Anfang (Pos1)").clicked() {
                self.locate(0);
            }
            if icon_button(ui, Icon::Stop, false, None, "Stopp").clicked() {
                self.stop();
            }
            let playing = self.is_playing();
            if icon_button(
                ui,
                Icon::Start,
                playing,
                Some(t.monitor),
                "Start (Leertaste)",
            )
            .clicked()
            {
                self.toggle_play();
            }
            if icon_button(
                ui,
                Icon::Record,
                self.recording,
                Some(t.rec),
                "Aufnahme (*)",
            )
            .clicked()
            {
                self.toggle_record();
            }
            if icon_button(ui, Icon::Cycle, self.cycle, Some(t.accent), "Cycle (/)").clicked() {
                self.toggle_cycle();
            }

            // Mitte: Werkzeuge und Snap, auf die Fensterbreite zentriert. Die Breite der
            // Gruppe stammt aus dem letzten Frame; ist kein Platz, folgt sie direkt links.
            let id = egui::Id::unique("werkzeugzeile-mitte");
            let width = ui.ctx().data(|d| d.get_temp::<f32>(id)).unwrap_or(0.0);
            let x = (row.center().x - width / 2.0).max(ui.cursor().left() + 12.0);
            ui.add_space((x - ui.cursor().left()).max(0.0));
            let start = ui.cursor().left();
            for tool in Tool::TOOLBAR {
                let mut hint = match tool.key_label() {
                    "" => tool.label().to_owned(),
                    key => format!("{} ({key})", tool.label()),
                };
                if !tool.implemented() {
                    hint.push_str(" – im Prototyp noch ohne Funktion");
                }
                let r = icon_button(ui, tool.icon(), self.tool == tool, None, &hint);
                if r.clicked() {
                    self.tool = tool;
                }
                // Untermenü am Werkzeugknopf: Modus für das Ziehen am Rand (Spec).
                if tool == Tool::Select {
                    r.context_menu(|ui| {
                        ui.label("Ziehen am Rand");
                        for mode in tools::ResizeMode::ALL {
                            if ui.radio(self.resize_mode == mode, mode.label()).clicked() {
                                self.resize_mode = mode;
                                self.tool = Tool::Select;
                                ui.close();
                            }
                        }
                    });
                }
            }
            icons::group_gap(ui);
            if icon_button(ui, Icon::Snap, self.snap, Some(t.accent), "Snap an/aus (J)").clicked() {
                self.snap = !self.snap;
            }
            snap::toolbar(self, ui);
            quantize::toolbar(self, ui);
            let used = ui.cursor().left() - start;
            if (used - width).abs() > 0.5 {
                ui.ctx().data_mut(|d| d.insert_temp(id, used));
                ui.ctx().request_repaint();
            }

            // Rechts: Zonen-Schalter, von rechts nach links gelegt.
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                if icon_button(
                    ui,
                    Icon::ZoneRight,
                    self.show_right,
                    Some(t.accent),
                    "Rechte Zone (Alt+R)",
                )
                .clicked()
                {
                    self.show_right = !self.show_right;
                }
                if icon_button(
                    ui,
                    Icon::ZoneLower,
                    self.show_lower,
                    Some(t.accent),
                    "Untere Zone",
                )
                .clicked()
                {
                    self.show_lower = !self.show_lower;
                }
                if icon_button(
                    ui,
                    Icon::ZoneLeft,
                    self.show_inspector,
                    Some(t.accent),
                    "Linke Zone (Alt+L)",
                )
                .clicked()
                {
                    self.show_inspector = !self.show_inspector;
                }
            });
        });
    }

    /// Globale Zustände links in der Werkzeugzeile: leuchten, sobald eine Spur gemutet, solo,
    /// auf Lesen oder auf Schreiben steht. Klick auf M, S oder W hebt den Zustand für alle
    /// Spuren auf; R schaltet Lesen für alle Kanäle ein bzw. aus.
    fn global_states(&mut self, ui: &mut Ui) {
        let t = theme::tokens(ui.ctx());
        let tracks = &self.project.tracks;
        let channel = |k: TrackKind| mixer::has_channel(k);
        let any_mute = tracks.iter().any(|t| t.mute);
        let any_solo = tracks.iter().any(|t| t.solo);
        let any_read = tracks.iter().any(|t| channel(t.kind) && t.read);
        let any_write = tracks.iter().any(|t| channel(t.kind) && t.write);
        let switches = [
            ("M", any_mute, t.mute, "Mute aufheben (alle Spuren)"),
            ("S", any_solo, t.solo, "Solo aufheben (alle Spuren)"),
            ("R", any_read, t.monitor, "Automation lesen (alle Kanäle)"),
            (
                "W",
                any_write,
                t.rec,
                "Automation schreiben aus (alle Kanäle)",
            ),
        ];
        let mut clicked = None;
        for (k, (label, on, color, name)) in switches.into_iter().enumerate() {
            let text = egui::RichText::new(label).color(if on { Color32::BLACK } else { t.text });
            let r = ui
                .add(
                    egui::Button::new(text)
                        .fill(if on { color } else { t.button })
                        .min_size(egui::vec2(20.0, 20.0)),
                )
                .on_hover_text(name);
            r.widget_info(|| egui::WidgetInfo::selected(egui::Role::Button, true, on, name));
            if r.clicked() {
                clicked = Some(k);
            }
        }
        let Some(k) = clicked else { return };
        let before = self.project.clone();
        for track in &mut self.project.tracks {
            match k {
                0 => track.mute = false,
                1 => track.solo = false,
                2 => {
                    if channel(track.kind) {
                        track.read = !any_read;
                    }
                }
                _ => track.write = false,
            }
        }
        self.checkpoint(before);
    }

    fn inspector(&mut self, ui: &mut Ui) {
        let Some(idx) = self
            .selected_track
            .filter(|&i| i < self.project.tracks.len())
        else {
            ui.weak("Keine Spur ausgewählt");
            return;
        };
        let track = &mut self.project.tracks[idx];
        // Kopf in Spurfarbe mit Name und Typ (docs/ui-parity.md, „Linke Zone“).
        let color = Color32::from_rgb(track.color[0], track.color[1], track.color[2]);
        let (head, _) =
            ui.allocate_exact_size(egui::vec2(ui.available_width(), 26.0), egui::Sense::hover());
        let luma = 0.299 * f32::from(color.r())
            + 0.587 * f32::from(color.g())
            + 0.114 * f32::from(color.b());
        let on_color = if luma > 140.0 {
            Color32::BLACK
        } else {
            Color32::WHITE
        };
        let painter = ui.painter();
        painter.rect_filled(head, 3.0, color);
        let kind_rect = painter.text(
            head.right_center() - egui::vec2(8.0, 0.0),
            egui::Align2::RIGHT_CENTER,
            track.kind.label(),
            egui::FontId::proportional(11.0),
            on_color,
        );
        // Langer Name endet vor dem Spurtyp.
        let name_clip =
            Rect::from_min_max(head.min, egui::pos2(kind_rect.left() - 6.0, head.max.y));
        painter.with_clip_rect(name_clip).text(
            head.left_center() + egui::vec2(8.0, 0.0),
            egui::Align2::LEFT_CENTER,
            &track.name,
            egui::FontId::proportional(14.0),
            on_color,
        );
        ui.add_space(4.0);
        ui.horizontal(|ui| {
            let label = ui.label("Name");
            let mut c = color;
            // Platz für Farbknopf und Abstände lassen, sonst wächst die Zone jedes Frame.
            let w = (ui.available_width() - 48.0).max(40.0);
            ui.add(egui::TextEdit::singleline(&mut track.name).desired_width(w))
                .labelled_by(label.id);
            if ui
                .color_edit_button_srgba(&mut c)
                .on_hover_text("Spurfarbe")
                .changed()
            {
                track.color = [c.r(), c.g(), c.b()];
            }
        });
        ui.horizontal(|ui| {
            let m = icons::state_toggle(ui, &mut track.mute, "M", theme::tokens(ui.ctx()).mute)
                .changed();
            let s = icons::state_toggle(ui, &mut track.solo, "S", theme::tokens(ui.ctx()).solo)
                .changed();
            self.dirty |= m || s;
            let t = theme::tokens(ui.ctx());
            if track.kind.holds_events()
                && icons::icon_button(
                    ui,
                    icons::Icon::Record,
                    track.record,
                    Some(t.rec),
                    "Aufnahme bereit",
                )
                .clicked()
            {
                track.record = !track.record;
            }
            if track.kind == TrackKind::Audio
                && icons::icon_button(
                    ui,
                    icons::Icon::Play,
                    track.monitor,
                    Some(t.monitor),
                    "Monitor",
                )
                .clicked()
            {
                track.monitor = !track.monitor;
                self.dirty = true;
            }
            if mixer::has_channel(track.kind) {
                icons::state_toggle(ui, &mut track.read, "R", t.monitor)
                    .on_hover_text("Automation lesen");
                icons::state_toggle(ui, &mut track.write, "W", t.rec)
                    .on_hover_text("Automation schreiben");
            }
        });
        if track.kind.holds_events() {
            ui.horizontal(|ui| {
                use studiocraft_project::TimeBase;
                let l = ui.label("Zeitbasis");
                let mut base = track.time_base();
                egui::ComboBox::from_id_salt("zeitbasis")
                    .selected_text(match base {
                        TimeBase::Musical => "Musikalisch",
                        TimeBase::Linear => "Linear",
                    })
                    .show_ui(ui, |ui| {
                        ui.selectable_value(&mut base, TimeBase::Musical, "Musikalisch");
                        ui.selectable_value(&mut base, TimeBase::Linear, "Linear");
                    })
                    .response
                    .labelled_by(l.id);
                if base != track.time_base() {
                    track.time_base = Some(base);
                }
                ui.checkbox(&mut track.show_lanes, "Lanes");
            });
        }
        // Marker- und Tempospur haben keinen Klang: keine Lautstärke, kein Panorama.
        if !matches!(track.kind, TrackKind::Marker | TrackKind::Tempo) {
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
            ui.horizontal(|ui| {
                let l = ui.label("Pan-Modus");
                let before = track.pan_mode;
                egui::ComboBox::from_id_salt(("pan-modus", idx))
                    .selected_text(track.pan_mode.label())
                    .show_ui(ui, |ui| {
                        for m in [
                            studiocraft_project::PanMode::Balance,
                            studiocraft_project::PanMode::Combined,
                        ] {
                            ui.selectable_value(&mut track.pan_mode, m, m.label());
                        }
                    })
                    .response
                    .labelled_by(l.id);
                self.dirty |= track.pan_mode != before;
            });
            ui.label(format!("{} Events", track.events.len()));
        }
        ui.add_space(4.0);
        // Sektionen mit Titelleiste (docs/ui-parity.md, „Linke Zone“): Routing zuerst,
        // dann Instrument, Inserts, Sends, EQ, Automation, Notizen.
        let kind = track.kind;
        let channel = crate::mixer::has_channel(kind);
        self.folder_ui(ui, idx);
        if kind == TrackKind::Midi {
            inspector_section(ui, "MIDI-Ausgang", true, |ui| self.midi_out_ui(ui, idx));
        }
        if kind == TrackKind::Audio || channel {
            inspector_section(ui, "Routing", true, |ui| {
                if kind == TrackKind::Audio {
                    self.input_ui(ui, idx);
                }
                if channel {
                    self.output_ui(ui, idx);
                }
            });
        }
        if kind == TrackKind::Marker {
            inspector_section(ui, "Marker", true, |ui| self.markers_ui(ui));
        }
        if kind == TrackKind::Tempo {
            inspector_section(ui, "Tempo", true, |ui| self.tempo_track_ui(ui));
        }
        if kind == TrackKind::Instrument {
            inspector_section(ui, "Instrument", true, |ui| self.instrument_ui(ui, idx));
        }
        if channel {
            inspector_section(ui, "Inserts", true, |ui| self.inserts_ui(ui, idx));
            inspector_section(ui, "Sends", true, |ui| self.sends_ui(ui, idx));
            inspector_section(ui, "EQ", false, |ui| self.eq_ui(ui, idx));
            inspector_section(ui, "Automation", false, |ui| self.automation_ui(ui, idx));
        }
        inspector_section(ui, "Notizen", false, |ui| {
            ui.weak("Kommt später");
        });
    }

    /// Transportleiste (docs/ui-parity.md, „Transportleiste“): Locatoren mit Punch und Cycle,
    /// Positionsanzeige, Transportknöpfe, Tempo/Takt/Metronom, rechts der Status.
    fn transport_bar(&mut self, ui: &mut Ui) {
        use icons::{Icon, icon_button};
        let t = theme::tokens(ui.ctx());
        ui.horizontal(|ui| {
            ui.spacing_mut().item_spacing.x = 2.0;
            let tempo = self.project.tempo.clone();
            let fmt = |ticks: u64| {
                let (b, be, s, _) = tempo.bars_beats(Position(ticks));
                format!("{b}.{be}.{s}")
            };
            // Locatoren, dazwischen Punch-In, Cycle, Punch-Out
            ui.label(RichText::new(format!("L {}", fmt(self.left_locator))).monospace())
                .on_hover_text("Linker Locator (Strg+Klick ins Lineal)");
            ui.add_space(4.0);
            if icon_button(
                ui,
                Icon::PunchIn,
                self.punch_in,
                Some(t.rec),
                "Punch-In (I)",
            )
            .clicked()
            {
                self.punch_in = !self.punch_in;
            }
            if icon_button(ui, Icon::Cycle, self.cycle, Some(t.accent), "Cycle (/)").clicked() {
                self.toggle_cycle();
            }
            if icon_button(
                ui,
                Icon::PunchOut,
                self.punch_out,
                Some(t.rec),
                "Punch-Out (O)",
            )
            .clicked()
            {
                self.punch_out = !self.punch_out;
            }
            ui.add_space(4.0);
            ui.label(RichText::new(format!("R {}", fmt(self.right_locator))).monospace())
                .on_hover_text("Rechter Locator (Alt+Klick ins Lineal)");
            icons::group_gap(ui);

            // Positionsanzeige
            let (bar, beat, six, tick) = tempo.bars_beats(Position(self.cursor));
            let secs = tempo.seconds(Position(self.cursor));
            egui::Frame::new()
                .fill(t.counter_bg)
                .corner_radius(3)
                .inner_margin(egui::Margin::symmetric(8, 1))
                .show(ui, |ui| {
                    ui.spacing_mut().item_spacing.x = 10.0;
                    ui.label(
                        RichText::new(format!("{bar}.{beat}.{six}.{tick:03}"))
                            .monospace()
                            .size(18.0)
                            .color(t.counter_text),
                    );
                    ui.label(
                        RichText::new(format!("{:02}:{:06.3}", (secs / 60.0) as u32, secs % 60.0))
                            .monospace()
                            .color(t.text_dim),
                    );
                });
            icons::group_gap(ui);

            // Transportknöpfe
            let bar_len = self.bar_ticks();
            if icon_button(ui, Icon::ToStart, false, None, "Zum Anfang (Pos1, Komma)").clicked() {
                self.locate(0);
            }
            if icon_button(ui, Icon::Rewind, false, None, "Zurückspulen um einen Takt").clicked() {
                self.locate(self.cursor.saturating_sub(bar_len));
            }
            if icon_button(ui, Icon::Forward, false, None, "Vorspulen um einen Takt").clicked() {
                self.locate(self.cursor.saturating_add(bar_len));
            }
            if icon_button(ui, Icon::Stop, false, None, "Stopp").clicked() {
                self.stop();
            }
            let playing = self.is_playing();
            if icon_button(
                ui,
                Icon::Start,
                playing,
                Some(t.monitor),
                "Start (Leertaste)",
            )
            .clicked()
            {
                self.toggle_play();
            }
            if icon_button(
                ui,
                Icon::Record,
                self.recording,
                Some(t.rec),
                "Aufnahme (*)",
            )
            .clicked()
            {
                self.toggle_record();
            }
            // Vorzähler: verbleibende Zählzeiten.
            if let Some(beats) = self.count_in_beats() {
                ui.colored_label(t.rec, format!("Vorzähler {beats}"));
            }
            icons::group_gap(ui);
            ui.spacing_mut().item_spacing.x = 6.0;
            // Automationsmodus für Spuren mit W; rot, solange geschrieben wird.
            let writing = self.writing_automation();
            let label = RichText::new("Auto");
            let l = ui.label(if writing { label.color(t.rec) } else { label });
            egui::ComboBox::from_id_salt("automationsmodus")
                .selected_text(self.auto_writer.mode.label())
                .width(84.0)
                .show_ui(ui, |ui| {
                    for m in automation_write::WriteMode::ALL {
                        ui.selectable_value(&mut self.auto_writer.mode, m, m.label());
                    }
                })
                .response
                .labelled_by(l.id)
                .on_hover_text("Automation schreiben bei W: Touch oder Auto-Latch");
            icons::group_gap(ui);
            self.tempo_controls(ui);

            // Ohne Statuszeile: Meldungen rechts in der Transportleiste, gekürzt auf den freien
            // Platz, damit sie nie über Tempo und Takt laufen.
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                if !self.show_status
                    && let Some(m) = &self.message
                    && ui.available_width() > 24.0
                {
                    let text = RichText::new(m.as_str()).color(ui.visuals().warn_fg_color);
                    ui.add(egui::Label::new(text).truncate())
                        .on_hover_text(m.as_str());
                }
            });
        });
    }

    /// Statuszeile (docs/ui-parity.md, Element 7): Audio-Ausgang, Eingang und MIDI links,
    /// Meldungen in der Mitte, Audio-Last und Bildzeit rechts. Lange Texte werden gekürzt,
    /// der volle Text steht im Tooltip.
    fn status_line(&mut self, ui: &mut Ui) {
        let t = theme::tokens(ui.ctx());
        let load = self.transport.as_ref().map(|e| e.status.load());
        ui.horizontal_centered(|ui| {
            ui.spacing_mut().item_spacing.x = 6.0;
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                ui.weak(format!("UI {:.1} ms", self.frame_ms))
                    .on_hover_text("Zeit pro Bild der Oberfläche");
                ui.separator();
                match load {
                    Some(l) => {
                        let pct = format!("Last {:.0} %", l * 100.0);
                        let text = if l >= 0.8 {
                            RichText::new(pct).color(t.rec)
                        } else {
                            RichText::new(pct).weak()
                        };
                        ui.label(text)
                            .on_hover_text("Audio-Last: Rechenzeit pro Block gegen seine Dauer");
                    }
                    None => {
                        ui.weak("Last –");
                    }
                }
                ui.separator();
                ui.with_layout(egui::Layout::left_to_right(egui::Align::Center), |ui| {
                    let mut parts = vec![self.audio_status.clone()];
                    if !self.input_status.is_empty() {
                        parts.push(format!("Eingang: {}", self.input_status));
                    }
                    if !self.midi_status.is_empty() {
                        parts.push(self.midi_status.clone());
                    }
                    let status = parts.join("  ·  ");
                    let text = match &self.message {
                        Some(m) => RichText::new(m.as_str()).color(ui.visuals().warn_fg_color),
                        None => RichText::new(status.as_str()).weak(),
                    };
                    let full = match &self.message {
                        Some(m) => format!("{m}\n{status}"),
                        None => status.clone(),
                    };
                    ui.add(egui::Label::new(text).truncate())
                        .on_hover_text(full);
                });
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

/// Ticks pro Viertel als `f64`, für Umrechnungen in Pixel.
pub(crate) const TPQ: f64 = TICKS_PER_QUARTER as f64;

/// So viele Schritte lassen sich rückgängig machen.
const UNDO_STEPS: usize = 200;

/// Aufklappbare Sektion im Inspector mit hinterlegter Titelleiste über die ganze Breite
/// (docs/ui-parity.md, „Linke Zone“). Klick auf die Leiste klappt auf und zu.
fn inspector_section(ui: &mut Ui, title: &str, open: bool, add: impl FnOnce(&mut Ui)) {
    use egui::collapsing_header::CollapsingState;
    let id = ui.make_persistent_id(("inspector-sektion", title));
    let mut state = CollapsingState::load_with_default_open(ui.ctx(), id, open);
    let (rect, resp) =
        ui.allocate_exact_size(egui::vec2(ui.available_width(), 20.0), egui::Sense::click());
    if resp.clicked() {
        state.toggle(ui);
    }
    let t = theme::tokens(ui.ctx());
    let fill = if resp.hovered() {
        t.button_hi
    } else {
        t.button
    };
    let painter = ui.painter();
    painter.rect_filled(rect, 3.0, fill);
    // Pfeil: nach rechts zu, nach unten offen.
    let c = rect.left_center() + egui::vec2(9.0, 0.0);
    let pts = if state.is_open() {
        vec![
            c + egui::vec2(-4.0, -2.5),
            c + egui::vec2(4.0, -2.5),
            c + egui::vec2(0.0, 3.0),
        ]
    } else {
        vec![
            c + egui::vec2(-2.5, -4.0),
            c + egui::vec2(3.0, 0.0),
            c + egui::vec2(-2.5, 4.0),
        ]
    };
    painter.add(egui::Shape::convex_polygon(pts, t.text, egui::Stroke::NONE));
    painter.text(
        rect.left_center() + egui::vec2(20.0, 0.0),
        egui::Align2::LEFT_CENTER,
        title,
        egui::FontId::proportional(12.0),
        t.text,
    );
    let is_open = state.is_open();
    resp.widget_info(|| egui::WidgetInfo::labeled(egui::Role::DisclosureTriangle, true, title));
    ui.ctx()
        .accesskit_node_builder(resp.id, |node| node.set_expanded(is_open));
    state.show_body_indented(&resp, ui, add);
    ui.add_space(2.0);
}

#[cfg(test)]
mod tests {
    use super::*;
    use egui::Key;
    use egui_kittest::{Harness, kittest::Queryable};
    use studiocraft_project::TempoMap;

    /// Rendert das Projektfenster headless. Mit `STUDIOCRAFT_SHOTS=<ordner>`
    /// werden die Bilder gespeichert, sonst wird nur geprüft, dass es rendert.
    fn shot(name: &str, setup: impl FnOnce(&mut ProjectWindow)) {
        shot_sized(name, egui::vec2(1400.0, 860.0), setup);
    }

    fn shot_sized(name: &str, size: egui::Vec2, setup: impl FnOnce(&mut ProjectWindow)) {
        let mut w = ProjectWindow::new(Project::demo(100), None, "kein Audio (Test)".into());
        setup(&mut w);
        let mut harness = Harness::builder()
            .with_size(size)
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
    fn shuffle_mode_swaps_events_by_dragging() {
        let mut p = Project::default();
        let t = p.add_track(TrackKind::Audio, "A");
        let mut w = ProjectWindow::new(p, None, String::new());
        let (bar, beat) = (w.bar_ticks(), w.beat_ticks());
        let a = w.project.create_event(t, 0, bar).unwrap();
        let b = w.project.create_event(t, bar, 2 * beat).unwrap();
        w.snap_mode = tools::SnapMode::Shuffle;
        w.view.scroll_ticks = 0.0;
        let mut h = window_harness(w);
        drag(&mut h, (bar / 2, 0.8), (bar / 2 + bar + beat, 0.8), 0.0);
        let order: Vec<(EventId, u64)> = h.state().project.tracks[t]
            .events
            .iter()
            .map(|e| (e.id, e.start.0))
            .collect();
        assert_eq!(order, [(b, 0), (a, 2 * beat)]);
        h.state_mut().undo();
        assert_eq!(h.state().project.event(a).map(|e| e.start.0), Some(0));
    }

    #[test]
    fn renders_snap_mode_menu() {
        shot("snap-modus", |w| {
            w.snap_mode = tools::SnapMode::EventsGridCursor;
            w.grid = Grid::Frames;
            w.grid_zoom = true;
        });
    }

    #[test]
    fn track_header_toggles_monitor_and_automation() {
        let (w, _) = one_event_window();
        let mut h = window_harness(w);
        let click_button = |h: &mut Harness<'_, ProjectWindow>, i: usize| {
            let w = h.state();
            let lanes = w.lanes_rect;
            let mid = lanes.top() + w.view.row_h / 2.0;
            // Two rows behind the colour strip, 21 px apart: M and S on top, the other
            // six (record, monitor, R, W, time base, lanes) below.
            let (y, col) = if i < 2 {
                (mid - 10.0, i)
            } else {
                (mid + 10.0, i - 2)
            };
            let x = lanes.left() - 290.0 + 38.0 + col as f32 * 21.0 + 9.0;
            let pos = egui::pos2(x, y);
            h.event(egui::Event::PointerMoved(pos));
            h.run_steps(1);
            for pressed in [true, false] {
                h.event(egui::Event::PointerButton {
                    pos,
                    button: egui::PointerButton::Primary,
                    pressed,
                    modifiers: egui::Modifiers::NONE,
                });
            }
            h.run_steps(2);
        };
        click_button(&mut h, 0);
        assert!(h.state().project.tracks[0].mute);
        click_button(&mut h, 1);
        assert!(h.state().project.tracks[0].solo);
        click_button(&mut h, 3);
        assert!(h.state().project.tracks[0].monitor);
        click_button(&mut h, 4);
        click_button(&mut h, 5);
        assert!(h.state().project.tracks[0].read);
        assert!(h.state().project.tracks[0].write);
        h.state_mut().undo();
        assert!(!h.state().project.tracks[0].write);
    }

    #[test]
    fn header_volume_slider_sets_the_track_volume() {
        let (w, _) = one_event_window();
        let mut h = window_harness(w);
        let press = |h: &mut Harness<'_, ProjectWindow>, pos: egui::Pos2, mods: egui::Modifiers| {
            h.event(egui::Event::ModifiersChanged(mods));
            h.event(egui::Event::PointerMoved(pos));
            h.run_steps(1);
            for pressed in [true, false] {
                h.event(egui::Event::PointerButton {
                    pos,
                    button: egui::PointerButton::Primary,
                    pressed,
                    modifiers: mods,
                });
            }
            h.run_steps(2);
            h.event(egui::Event::ModifiersChanged(egui::Modifiers::NONE));
            h.run_steps(1);
        };
        let (left, right, y) = {
            let w = h.state();
            let l = w.lanes_rect;
            // Slider: 64 px wide, 6 px from the right edge of the header, upper line.
            (
                l.left() - 6.0 - 64.0,
                l.left() - 6.0,
                l.top() + w.view.row_h / 2.0 - 10.0,
            )
        };
        press(&mut h, egui::pos2(left + 1.0, y), egui::Modifiers::NONE);
        let low = h.state().project.tracks[0].volume_db;
        assert!(low < -40.0, "left end is near silence, got {low}");
        press(&mut h, egui::pos2(right - 1.0, y), egui::Modifiers::NONE);
        let high = h.state().project.tracks[0].volume_db;
        assert!(high > 5.0, "right end is near the maximum, got {high}");
        press(&mut h, egui::pos2(left + 30.0, y), egui::Modifiers::COMMAND);
        assert_eq!(h.state().project.tracks[0].volume_db, 0.0);
        h.state_mut().undo();
        assert_eq!(h.state().project.tracks[0].volume_db, high);
        // Clicks on the slider do not select or toggle anything else.
        assert!(!h.state().project.tracks[0].mute);
    }

    #[test]
    fn renders_track_header_with_monitor_and_automation() {
        shot("spurkopf-monitor", |w| {
            w.project.tracks[0].monitor = true;
            w.project.tracks[0].read = true;
            w.project.tracks[1].write = true;
            w.project.tracks[2].record = true;
        });
    }

    #[test]
    fn clicking_a_zone_moves_the_focus() {
        let (mut w, _) = one_event_window();
        w.show_lower = true;
        w.lower = LowerZone::Editor;
        let mut h = window_harness(w);
        let press = |h: &mut Harness<'_, ProjectWindow>, pos: egui::Pos2| {
            h.event(egui::Event::PointerMoved(pos));
            h.run_steps(1);
            for pressed in [true, false] {
                h.event(egui::Event::PointerButton {
                    pos,
                    button: egui::PointerButton::Primary,
                    pressed,
                    modifiers: egui::Modifiers::NONE,
                });
            }
            h.run_steps(2);
        };
        assert_eq!(h.state().focus, focus::Zone::Project);
        // Lower zone: just above the tab row at the bottom of the window.
        let lanes = h.state().lanes_rect;
        let lower = egui::pos2(lanes.center().x, 780.0);
        press(&mut h, lower);
        assert_eq!(h.state().focus, focus::Zone::Lower);
        assert!(h.state().key_editor.focused);
        press(&mut h, egui::pos2(lanes.right() - 20.0, lanes.top() + 5.0));
        assert_eq!(h.state().focus, focus::Zone::Project);
        assert!(!h.state().key_editor.focused);
        press(&mut h, egui::pos2(20.0, lanes.top() + 200.0));
        assert_eq!(h.state().focus, focus::Zone::Left);
        // Hiding the focused zone hands the focus back to the event display.
        h.state_mut().show_inspector = false;
        h.run_steps(2);
        assert_eq!(h.state().focus, focus::Zone::Project);
    }

    #[test]
    fn renders_focus_frame_on_the_lower_zone() {
        shot("fokus-untere-zone", |w| {
            w.focus = focus::Zone::Lower;
        });
    }

    #[test]
    fn lanes_show_overlapping_takes_and_clicks_pick_the_lane() {
        let mut p = Project::default();
        let t = p.add_track(TrackKind::Audio, "Gesang");
        p.add_track(TrackKind::Audio, "Bass");
        let mut w = ProjectWindow::new(p, None, String::new());
        let bar = w.bar_ticks();
        let first = w.project.create_event(t, 0, 2 * bar).unwrap();
        let take = w.project.create_event(t, bar / 2, bar).unwrap();
        w.view.scroll_ticks = 0.0;
        let mut h = window_harness(w);
        let click_at = |h: &mut Harness<'_, ProjectWindow>, ticks: u64, y: f32| {
            let w = h.state();
            let r = w.lanes_rect;
            let x = r.left() + (ticks as f64 / TPQ) as f32 * w.view.px_per_quarter;
            let pos = egui::pos2(x, r.top() + y * w.view.row_h);
            h.event(egui::Event::PointerMoved(pos));
            h.run_steps(1);
            for pressed in [true, false] {
                h.event(egui::Event::PointerButton {
                    pos,
                    button: egui::PointerButton::Primary,
                    pressed,
                    modifiers: egui::Modifiers::NONE,
                });
            }
            h.run_steps(2);
        };
        // Folded: the later take covers the first event.
        click_at(&mut h, bar, 0.8);
        assert_eq!(h.state().selection, vec![take]);
        h.state_mut().project.tracks[t].show_lanes = true;
        h.run_steps(1);
        click_at(&mut h, bar, 0.8);
        assert_eq!(h.state().selection, vec![first]);
        click_at(&mut h, bar, 1.8);
        assert_eq!(h.state().selection, vec![take]);
        // The next track starts below both lanes.
        click_at(&mut h, bar, 2.5);
        assert_eq!(h.state().selected_track, Some(1));
    }

    #[test]
    fn renders_lanes_and_time_base() {
        shot("lanes-zeitbasis", |w| {
            w.project.tracks[0].show_lanes = true;
            let bar = w.bar_ticks();
            w.project.create_event(0, bar / 2, 2 * bar);
            w.project.tracks[3].time_base = Some(studiocraft_project::TimeBase::Linear);
        });
    }

    #[test]
    fn renders_fine_grid_lines() {
        shot("raster-1-16", |w| {
            w.grid = Grid::UseQuantize;
            w.view.zoom(4.0, None);
        });
        shot("raster-frames", |w| {
            w.grid = Grid::Frames;
            w.view.zoom(6.0, None);
        });
    }

    #[test]
    fn renders_narrow_window_with_long_status() {
        shot_sized("schmal", egui::vec2(1150.0, 760.0), |w| {
            w.audio_status = "pipewire · default_output · 48000 Hz · Block 256".into();
            w.midi_status = "MIDI: USB Audio Interface:USB Audio Interface MIDI 1 16:0".into();
        });
    }

    #[test]
    fn automation_lane_opens_from_the_arrow_and_points_are_edited_with_the_mouse() {
        use studiocraft_project::AutoParam;
        let (w, id) = one_event_window();
        let mut h = window_harness(w);
        let at = |h: &Harness<'_, ProjectWindow>, x: f32, rows: f32| {
            let w = h.state();
            egui::pos2(x, w.lanes_rect.top() + rows * w.view.row_h)
        };
        let press = |h: &mut Harness<'_, ProjectWindow>, from: egui::Pos2, to: egui::Pos2| {
            h.event(egui::Event::PointerMoved(from));
            h.run_steps(1);
            h.event(egui::Event::PointerButton {
                pos: from,
                button: egui::PointerButton::Primary,
                pressed: true,
                modifiers: egui::Modifiers::NONE,
            });
            h.run_steps(1);
            for i in 1..=4 {
                h.event(egui::Event::PointerMoved(
                    from + (to - from) * (i as f32 / 4.0),
                ));
                h.run_steps(1);
            }
            h.event(egui::Event::PointerButton {
                pos: to,
                button: egui::PointerButton::Primary,
                pressed: false,
                modifiers: egui::Modifiers::NONE,
            });
            h.run_steps(2);
        };
        // Arrow right of the colour strip, in the upper line of the header.
        let left = h.state().lanes_rect.left();
        let arrow = at(&h, left - 290.0 + 29.0, 0.3);
        press(&mut h, arrow, arrow);
        let lanes = &h.state().project.tracks[0].automation;
        assert_eq!(lanes.len(), 1);
        assert_eq!(lanes[0].param, AutoParam::Volume);
        // Click high up in the automation lane (second row): a point near the top of the range.
        let x = left + 200.0;
        let p = at(&h, x, 1.0 + 6.0 / 52.0);
        press(&mut h, p, p);
        let pts = h.state().project.tracks[0].automation[0].points.clone();
        assert_eq!(pts.len(), 1);
        assert!(pts[0].value > 3.0, "{}", pts[0].value);
        assert!(
            h.state().selection.is_empty(),
            "the event behind stays unselected"
        );
        // Drag the point down to the bottom: the lowest value.
        let to = at(&h, x, 1.0 + 46.0 / 52.0);
        press(&mut h, p, to);
        let pts = h.state().project.tracks[0].automation[0].points.clone();
        assert_eq!(pts.len(), 1);
        assert!(pts[0].value < -55.0, "{}", pts[0].value);
        // Erase tool removes it (click where the snapped point is); undo brings it back.
        h.state_mut().tool = Tool::Erase;
        let px = {
            let w = h.state();
            left + ((pts[0].tick as f64 - w.view.scroll_ticks) / TPQ) as f32 * w.view.px_per_quarter
        };
        let on_point = egui::pos2(px, to.y);
        press(&mut h, on_point, on_point);
        assert!(h.state().project.tracks[0].automation[0].points.is_empty());
        h.state_mut().undo();
        assert_eq!(h.state().project.tracks[0].automation[0].points.len(), 1);
        assert!(h.state().project.event(id).is_some());
    }

    #[test]
    fn renders_automation_lanes() {
        use studiocraft_project::AutoParam;
        shot("automation", |w| {
            let bar = w.bar_ticks();
            let t = &mut w.project.tracks[0];
            t.read = true;
            let vol = t.automation_lane_mut(AutoParam::Volume);
            for (k, v) in [(0, 0.0), (2, -20.0), (3, -20.0), (5, 3.0), (8, -6.0)] {
                vol.set_point(k * bar, v);
            }
            let pan = t.automation_lane_mut(AutoParam::Pan);
            pan.set_point(bar, -1.0);
            pan.set_point(6 * bar, 1.0);
            pan.muted = true;
            w.project.tracks[1]
                .automation_lane_mut(AutoParam::Mute)
                .visible = false;
        });
    }

    #[test]
    fn renders_integrated_title_bar() {
        shot("titelleiste", |w| {
            w.window_frame = titlebar::integrated_frame()
        });
    }

    #[test]
    fn renders_title_bar_with_desktop_button_layouts() {
        use craft_window::policy::{CaptionFamily, Captions, Frame};
        use craft_window::{Button, Layout};
        // GNOME with "close" on the left, Adwaita look.
        shot("titelleiste-gnome", |w| {
            w.window_frame = Frame::Integrated(Captions {
                family: CaptionFamily::Adwaita,
                layout: Layout {
                    left: vec![Button::Close],
                    right: Vec::new(),
                },
            });
        });
        shot("titelleiste-windows", |w| {
            w.window_frame = Frame::Integrated(Captions {
                family: CaptionFamily::Windows,
                layout: Layout::default(),
            });
        });
    }

    #[test]
    fn title_bar_buttons_send_window_commands() {
        let (mut w, _) = one_event_window();
        w.window_frame = titlebar::integrated_frame();
        let mut h = window_harness(w);
        h.get_by_label("Maximieren").click();
        h.run_steps(1);
        h.get_by_label("Minimieren").click();
        h.run_steps(1);
        h.get_by_label("Schließen").click();
        h.run_steps(1);
        // The menus stay reachable in the same bar.
        h.get_by_label("Datei");
    }

    #[test]
    fn comp_tool_picks_sections_of_takes_in_lanes() {
        let mut p = Project::default();
        let t = p.add_track(TrackKind::Audio, "Gesang");
        let mut w = ProjectWindow::new(p, None, String::new());
        let bar = w.bar_ticks();
        let take1 = w.project.create_event(t, 0, 4 * bar).unwrap();
        let take2 = w.project.create_event(t, 0, 4 * bar).unwrap();
        w.project.tracks[t].show_lanes = true;
        w.view.scroll_ticks = 0.0;
        w.tool = Tool::Comp;
        let mut h = window_harness(w);
        assert_eq!(h.state().project.event_at(t, bar + 1), Some(take2));
        // Drag across bars 2..3 of take 1 (top lane).
        drag(&mut h, (bar, 0.5), (2 * bar, 0.5), 0.0);
        let p = &h.state().project;
        let mid = p.event_at(t, bar + bar / 2).unwrap();
        assert_ne!(mid, take2);
        assert_eq!(
            p.event(mid).map(|e| (e.start.0, e.end())),
            Some((bar, 2 * bar))
        );
        assert_eq!(p.event_at(t, 3 * bar), Some(take2));
        assert_eq!(h.state().selection, vec![mid]);
        // Undo puts the takes back together.
        h.state_mut().undo();
        assert_eq!(h.state().project.tracks[t].events.len(), 2);
        assert!(h.state().project.event(take1).is_some());
    }

    #[test]
    fn renders_comped_lanes() {
        shot("comping", |w| {
            let bar = w.bar_ticks();
            w.project.tracks[0].events.clear();
            let a = w.project.create_event(0, 0, 8 * bar).unwrap();
            w.project.create_event(0, 0, 8 * bar).unwrap();
            w.project.create_event(0, bar, 6 * bar).unwrap();
            w.project.comp(a, 2 * bar, 4 * bar);
            w.project.tracks[0].show_lanes = true;
            w.tool = Tool::Comp;
        });
    }

    #[test]
    fn colour_strip_opens_the_palette_and_events_follow_the_track() {
        let (w, id) = one_event_window();
        let mut h = window_harness(w);
        let strip = {
            let w = h.state();
            egui::pos2(
                w.lanes_rect.left() - 290.0 + 2.0,
                w.lanes_rect.top() + w.view.row_h / 2.0,
            )
        };
        h.event(egui::Event::PointerMoved(strip));
        h.run_steps(1);
        for pressed in [true, false] {
            h.event(egui::Event::PointerButton {
                pos: strip,
                button: egui::PointerButton::Primary,
                pressed,
                modifiers: egui::Modifiers::NONE,
            });
        }
        h.run_steps(2);
        assert!(h.state().color_popup.is_some());
        h.get_by_label("Blau").click();
        h.run_steps(2);
        let blue = studiocraft_project::TRACK_COLORS[5];
        let w = h.state();
        assert_eq!(w.project.tracks[0].color, blue);
        assert!(w.color_popup.is_none());
        let ev = w.project.event(id).unwrap();
        assert_eq!(
            ev.color_on(&w.project.tracks[0]),
            blue,
            "the event follows its track"
        );
        // An own colour sticks; "Spurfarbe übernehmen" goes back to the track's.
        let w = h.state_mut();
        w.selection = vec![id];
        w.set_event_color(Some([1, 2, 3]));
        assert_eq!(w.project.event(id).unwrap().color, Some([1, 2, 3]));
        w.set_event_color(None);
        assert_eq!(w.project.event(id).unwrap().color, None);
        w.undo();
        assert_eq!(w.project.event(id).unwrap().color, Some([1, 2, 3]));
    }

    #[test]
    fn renders_colour_palette_and_event_colours() {
        shot("farben", |w| {
            let first = w.project.tracks[0].events[1].id;
            w.selection = vec![first];
            w.set_event_color(Some(studiocraft_project::TRACK_COLORS[3]));
            w.selection.clear();
            w.open_color_popup(colors::Target::Track(0), egui::pos2(260.0, 150.0));
        });
    }

    /// Testordner mit einer kurzen WAV-Datei `Loop.wav`.
    fn browser_dir(name: &str) -> std::path::PathBuf {
        let d = std::env::temp_dir().join(format!("studiocraft-ui-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(d.join("Takes")).unwrap();
        let clip = tone_clip(0.5, 220.0);
        let frames = clip.samples.len() / 2;
        let mut b = Vec::new();
        let data = (frames * 4) as u32;
        b.extend_from_slice(b"RIFF");
        b.extend_from_slice(&(36 + data).to_le_bytes());
        b.extend_from_slice(b"WAVEfmt ");
        b.extend_from_slice(&16u32.to_le_bytes());
        for v in [1u16, 2] {
            b.extend_from_slice(&v.to_le_bytes());
        }
        b.extend_from_slice(&48_000u32.to_le_bytes());
        b.extend_from_slice(&192_000u32.to_le_bytes());
        for v in [4u16, 16] {
            b.extend_from_slice(&v.to_le_bytes());
        }
        b.extend_from_slice(b"data");
        b.extend_from_slice(&data.to_le_bytes());
        for s in clip.samples.iter() {
            b.extend_from_slice(&((s * 20000.0) as i16).to_le_bytes());
        }
        std::fs::write(d.join("Loop.wav"), b).unwrap();
        d
    }

    #[test]
    fn files_drag_from_the_browser_into_the_event_display() {
        let d = browser_dir("drag");
        let mut p = Project::default();
        p.add_track(TrackKind::Audio, "Audio");
        let mut w = ProjectWindow::new(p, None, String::new());
        w.show_right = true;
        w.right.tab = rightzone::Tab::Files;
        rightzone::open_dir(&mut w, d.clone());
        let mut h = window_harness(w);
        let from = h.get_by_label("Loop.wav").rect().center();
        let to = {
            let w = h.state();
            egui::pos2(
                w.lanes_rect.left() + 200.0,
                w.lanes_rect.top() + w.view.row_h / 2.0,
            )
        };
        let button = |pos, pressed| egui::Event::PointerButton {
            pos,
            button: egui::PointerButton::Primary,
            pressed,
            modifiers: egui::Modifiers::NONE,
        };
        h.event(egui::Event::PointerMoved(from));
        h.run_steps(1);
        h.event(button(from, true));
        h.run_steps(1);
        for i in 1..=6 {
            h.event(egui::Event::PointerMoved(
                from + (to - from) * (i as f32 / 6.0),
            ));
            h.run_steps(1);
        }
        h.event(button(to, false));
        h.run_steps(2);
        let events = &h.state().project.tracks[0].events;
        assert_eq!(events.len(), 1, "dropped on the audio track");
        assert_eq!(h.state().project.pool.len(), 1);
        let _ = std::fs::remove_dir_all(&d);
    }

    #[test]
    fn renders_file_browser() {
        let d = browser_dir("shot");
        shot("dateibrowser", |w| {
            w.show_right = true;
            w.right.tab = rightzone::Tab::Files;
            rightzone::open_dir(w, d.clone());
            w.right.selected = Some(d.join("Loop.wav"));
        });
        let _ = std::fs::remove_dir_all(&d);
    }

    /// Jedes Sonderzeichen in einem Text der Oberfläche muss in der eingebauten Schrift sein,
    /// sonst bricht egui beim Zeichnen ab (so geschehen mit Pfeilen im Dateibrowser).
    #[test]
    fn every_ui_text_character_has_a_glyph() {
        fn sources(dir: &std::path::Path, out: &mut Vec<std::path::PathBuf>) {
            for e in std::fs::read_dir(dir).unwrap().flatten() {
                let p = e.path();
                if p.is_dir() {
                    sources(&p, out);
                } else if p.extension().is_some_and(|x| x == "rs") {
                    out.push(p);
                }
            }
        }
        let mut files = Vec::new();
        sources(
            &std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("src"),
            &mut files,
        );
        let mut chars = std::collections::BTreeSet::new();
        for f in files {
            let text = std::fs::read_to_string(&f).unwrap();
            // Nur Code vor dem Testmodul, ohne Kommentarzeilen: Texte, die angezeigt werden.
            let code = text.split("#[cfg(test)]").next().unwrap_or("");
            for line in code.lines().filter(|l| !l.trim_start().starts_with("//")) {
                let mut inside = false;
                for c in line.chars() {
                    if c == '"' {
                        inside = !inside;
                    } else if inside && !c.is_ascii() {
                        chars.insert(c);
                    }
                }
            }
        }
        let ctx = egui::Context::default();
        let mut out = ctx.run_ui(egui::RawInput::default(), |_| {});
        out.textures_delta.clear();
        let missing: Vec<char> = ctx.fonts_mut(|f| {
            chars
                .iter()
                .copied()
                .filter(|&c| !f.has_glyph(&egui::FontId::proportional(13.0), c))
                .collect()
        });
        assert!(missing.is_empty(), "no glyph for {missing:?}");
    }

    fn fake_plugin(id: &str, name: &str, instrument: bool) -> studiocraft_plugin_host::PluginInfo {
        studiocraft_plugin_host::PluginInfo {
            id: id.into(),
            name: name.into(),
            vendor: String::new(),
            version: "1".into(),
            features: vec![
                if instrument {
                    "instrument"
                } else {
                    "audio-effect"
                }
                .into(),
            ],
            path: std::path::PathBuf::from("/nicht/vorhanden.clap"),
        }
    }

    /// Zieht einen Eintrag der rechten Zone mit der Beschriftung `label` nach `to`.
    fn drag_label(h: &mut Harness<'_, ProjectWindow>, label: &str, to: egui::Pos2) {
        let from = h.get_by_label(label).rect().center();
        let button = |pos, pressed| egui::Event::PointerButton {
            pos,
            button: egui::PointerButton::Primary,
            pressed,
            modifiers: egui::Modifiers::NONE,
        };
        h.event(egui::Event::PointerMoved(from));
        h.run_steps(1);
        h.event(button(from, true));
        h.run_steps(1);
        for i in 1..=6 {
            h.event(egui::Event::PointerMoved(
                from + (to - from) * (i as f32 / 6.0),
            ));
            h.run_steps(1);
        }
        h.event(button(to, false));
        h.run_steps(2);
    }

    #[test]
    fn plugins_drag_from_the_right_zone_onto_tracks() {
        let mut p = Project::default();
        let audio = p.add_track(TrackKind::Audio, "Gesang");
        let mut w = ProjectWindow::new(p, None, String::new());
        w.plugins.list = Some(vec![
            fake_plugin("test.synth", "Synth", true),
            fake_plugin("test.hall", "Hall", false),
            fake_plugin("test.piano", "Piano", true),
        ]);
        w.show_right = true;
        w.right.tab = rightzone::Tab::Plugins;
        let mut h = window_harness(w);
        let row = |h: &Harness<'_, ProjectWindow>, r: f32| {
            let w = h.state();
            egui::pos2(
                w.lanes_rect.left() + 300.0,
                w.lanes_rect.top() + r * w.view.row_h,
            )
        };
        // Effect onto the audio track: a new insert.
        let to = row(&h, 0.5);
        drag_label(&mut h, "Hall", to);
        assert_eq!(h.state().project.tracks[audio].inserts.len(), 1);
        assert_eq!(
            h.state().project.tracks[audio].inserts[0].plugin_id,
            "test.hall"
        );
        // Instrument onto the empty area below: a new instrument track.
        let to = row(&h, 3.0);
        drag_label(&mut h, "Synth", to);
        let tracks = &h.state().project.tracks;
        assert_eq!(tracks.len(), 2);
        assert_eq!(tracks[1].kind, TrackKind::Instrument);
        assert_eq!(
            tracks[1].instrument.as_ref().map(|i| i.plugin_id.as_str()),
            Some("test.synth")
        );
        // Another instrument onto that instrument track replaces it.
        let to = row(&h, 1.5);
        drag_label(&mut h, "Piano", to);
        let tracks = &h.state().project.tracks;
        assert_eq!(tracks.len(), 2);
        assert_eq!(
            tracks[1].instrument.as_ref().map(|i| i.plugin_id.as_str()),
            Some("test.piano")
        );
        // An effect dropped on empty space does nothing.
        let to = row(&h, 4.0);
        drag_label(&mut h, "Hall", to);
        assert_eq!(h.state().project.tracks.len(), 2);
        assert_eq!(h.state().project.tracks[audio].inserts.len(), 1);
        h.state_mut().undo();
        assert_eq!(
            h.state().project.tracks[1]
                .instrument
                .as_ref()
                .map(|i| i.plugin_id.as_str()),
            Some("test.synth")
        );
    }

    #[test]
    fn renders_info_line_for_several_events() {
        shot("info-mehrfach", |w| {
            let ids: Vec<EventId> = w.project.tracks[0]
                .events
                .iter()
                .take(3)
                .map(|e| e.id)
                .collect();
            w.selection = ids;
        });
        shot("info-mehrfach-hell", |w| {
            let ids: Vec<EventId> = w.project.tracks[0]
                .events
                .iter()
                .take(3)
                .map(|e| e.id)
                .collect();
            w.selection = ids;
            w.theme = egui::ThemePreference::Light;
        });
    }

    #[test]
    fn status_line_shows_device_and_load_and_can_be_hidden() {
        let (mut w, _) = one_event_window();
        w.audio_status = "pipewire · default_output · 48000 Hz".into();
        w.message = Some("Datei fehlt".into());
        let mut h = window_harness(w);
        h.get_by_label("Last –");
        h.get_by_label("Datei fehlt");
        h.get_by_label("Ansicht").click();
        h.run_steps(2);
        h.get_by_label("Statuszeile").click();
        h.run_steps(2);
        assert!(!h.state().show_status);
        assert!(h.query_by_label("Last –").is_none());
        // Without the status line, messages show in the transport bar.
        h.get_by_label("Datei fehlt");
    }

    fn double_click_at(h: &mut Harness<'_, ProjectWindow>, at: egui::Pos2) {
        h.event(egui::Event::PointerMoved(at));
        h.run_steps(1);
        for _ in 0..2 {
            for pressed in [true, false] {
                h.event(egui::Event::PointerButton {
                    pos: at,
                    button: egui::PointerButton::Primary,
                    pressed,
                    modifiers: egui::Modifiers::NONE,
                });
                h.run_steps(1);
            }
        }
        h.run_steps(1);
    }

    #[test]
    fn render_fade_curves_volume_handle_and_curve_editor() {
        shot("fades", |w| {
            let bar = w.bar_ticks();
            let id = w
                .import_clip("Take", None, tone_clip(8.0, 220.0), Some((0, bar)))
                .unwrap();
            w.project.set_fades(id, 2 * bar, 3 * bar);
            let e = w.project.event_mut(id).unwrap();
            e.fade_in_curve = studiocraft_project::FadeCurve::SCurve;
            e.fade_out_curve = studiocraft_project::FadeCurve::Exponential;
            e.gain_db = 3.0;
            w.selection = vec![id];
            w.view.scroll_ticks = 0.0;
            w.view.px_per_quarter = 40.0;
            w.view.row_h = 90.0;
            w.open_fade_editor(id, fades_ui::Side::In);
        });
    }

    #[test]
    fn the_volume_handle_sets_event_gain_in_one_undo_step() {
        let (mut w, id) = one_event_window();
        w.view.scroll_ticks = 0.0;
        let mut h = window_harness(w);
        let (start, end) = {
            let e = h.state().project.event(id).unwrap();
            (e.start.0, e.end())
        };
        let top = screen(&h, (start + end) / 2, 0.0) + egui::vec2(0.0, 3.0);
        drag_with(
            &mut h,
            top,
            top - egui::vec2(0.0, 20.0),
            egui::Modifiers::NONE,
            egui::Modifiers::NONE,
        );
        let e = h.state().project.event(id).unwrap();
        assert_eq!(e.gain_db, 5.0, "20 px up at 0.25 dB per px");
        assert_eq!(e.start.0, start, "the handle does not move the event");
        h.state_mut().undo();
        assert_eq!(h.state().project.event(id).unwrap().gain_db, 0.0);
    }

    #[test]
    fn double_click_on_a_fade_handle_opens_the_curve_editor() {
        let (mut w, id) = one_event_window();
        w.view.scroll_ticks = 0.0;
        w.project.set_fades(id, 960, 0);
        w.clear_history();
        let mut h = window_harness(w);
        let start = h.state().project.event(id).unwrap().start.0;
        let handle = screen(&h, start + 960, 0.2);
        double_click_at(&mut h, handle);
        assert!(h.state().fade_editor.is_some(), "curve editor is open");
        assert!(
            !h.state().show_lower || h.state().lower != LowerZone::SampleEditor,
            "a fade double-click does not open the sample editor"
        );
        // The fade that was clicked comes first.
        h.get_all_by_label("S-Kurve").next().unwrap().click();
        h.run_steps(3);
        let e = h.state().project.event(id).unwrap();
        assert_eq!(e.fade_in_curve, studiocraft_project::FadeCurve::SCurve);
        assert_eq!(e.fade_out_curve, studiocraft_project::FadeCurve::Linear);
        assert_eq!(e.fade_in, 960, "the length stays");
        h.state_mut().undo();
        assert_eq!(
            h.state().project.event(id).unwrap().fade_in_curve,
            studiocraft_project::FadeCurve::Linear
        );
    }

    #[test]
    fn keypad_drives_the_transport_while_the_number_row_picks_tools() {
        use keypad::{KeypadKey, KeypadPress, KeypadTap};
        let (mut w, _) = one_event_window();
        let tap = KeypadTap::new();
        w.set_keypad(tap.clone());
        w.left_locator = 3840;
        w.right_locator = 7680;
        assert_eq!(w.project.add_marker(11520), 3, "first position marker is 3");
        let mut h = window_harness(w);
        // A keypad press arrives as an egui key (Num1) plus the tap entry from the app.
        let press = |h: &mut Harness<'_, ProjectWindow>, key: KeypadKey, egui_key: Option<Key>| {
            tap.push(KeypadPress {
                key,
                shift: false,
                repeat: false,
            });
            match egui_key {
                Some(k) => h.key_press(k),
                None => h.event(egui::Event::Text("*".into())),
            }
            h.run_steps(1);
        };
        h.key_press(Key::Num2);
        h.run_steps(1);
        assert_eq!(
            h.state().tool,
            Tool::Range,
            "number row 2 picks the range tool"
        );
        press(&mut h, KeypadKey::Digit(1), Some(Key::Num1));
        assert_eq!(h.state().tool, Tool::Range, "keypad 1 is no tool key");
        assert_eq!(h.state().cursor, 3840, "keypad 1 jumps to the left locator");
        press(&mut h, KeypadKey::Digit(2), Some(Key::Num2));
        assert_eq!(h.state().cursor, 7680);
        press(&mut h, KeypadKey::Digit(3), Some(Key::Num3));
        assert_eq!(h.state().cursor, 11520, "keypad 3 goes to marker 3");
        press(&mut h, KeypadKey::Subtract, Some(Key::Minus));
        assert_eq!(h.state().cursor, 11520 - h.state().bar_ticks());
        press(&mut h, KeypadKey::Add, Some(Key::Plus));
        assert_eq!(h.state().cursor, 11520);
        let cycle = h.state().cycle;
        press(&mut h, KeypadKey::Divide, Some(Key::Slash));
        assert_eq!(h.state().cycle, !cycle);
        press(&mut h, KeypadKey::Decimal, None);
        press(&mut h, KeypadKey::Digit(0), Some(Key::Num0));
        assert_eq!(
            h.state().cursor,
            0,
            "stop while stopped returns to the start"
        );
        assert_eq!(
            h.state().tool,
            Tool::Range,
            "no keypad key changed the tool"
        );
        // Main row 1 still selects.
        h.key_press(Key::Num1);
        h.run_steps(1);
        assert_eq!(h.state().tool, Tool::Select);
    }

    #[test]
    fn a_macro_built_in_the_dialog_runs_its_commands_from_key_and_menu() {
        use keymap::Cmd;
        let (w, _) = one_event_window();
        let mut h = window_harness(w);
        h.get_by_label("Bearbeiten").click();
        h.run_steps(2);
        h.get_by_label("Tastaturbefehle …").click();
        h.run_steps(2);
        // The section header (the command list also has a "Makros" category).
        h.get_all_by_label("Makros").next().unwrap().click();
        h.run_steps(2);
        h.get_by_label("Name von Makro 1").click();
        h.run_steps(1);
        h.get_by_label("Name von Makro 1")
            .type_text("Zoom und Snap");
        h.run_steps(1);
        for (category, step) in [
            ("Navigation", "Horizontal einzoomen"),
            ("Bearbeitung", "Snap an/aus"),
        ] {
            h.get_by_label("Befehl an Makro 1 anhängen").click();
            h.run_steps(2);
            // The last match is the entry in the open macro menu (the toolbar has buttons
            // with the same names).
            h.get_all_by_role_and_label(egui::accesskit::Role::Button, category)
                .last()
                .unwrap()
                .click();
            h.run_steps(2);
            h.get_all_by_role_and_label(egui::accesskit::Role::Button, step)
                .last()
                .unwrap()
                .click();
            h.run_steps(2);
        }
        let m = h.state().keymap.macro_slot(1).cloned().unwrap();
        assert_eq!(m.name, "Zoom und Snap");
        assert_eq!(m.steps, [Cmd::ZoomIn, Cmd::Snap]);
        // Bind F9 (a capture in the dialog is covered by the test below).
        let f9 = egui::KeyboardShortcut::new(egui::Modifiers::NONE, Key::F9);
        h.state_mut().keymap.assign(Cmd::Macro(1), f9);
        h.state_mut().key_dialog.open = false;
        h.run_steps(1);
        let (zoom, snap) = (h.state().view.px_per_quarter, h.state().snap);
        h.key_press(Key::F9);
        h.run_steps(1);
        assert!(h.state().view.px_per_quarter > zoom);
        assert_eq!(h.state().snap, !snap);
        // The Edit menu lists it under its name and runs it too.
        h.get_by_label("Bearbeiten").click();
        h.run_steps(2);
        h.get_by_label("Makros").click();
        h.run_steps(2);
        // Menu items carry their key in the label.
        h.get_by_label("Makro 1: Zoom und Snap F9").click();
        h.run_steps(2);
        assert_eq!(h.state().snap, snap);
    }

    #[test]
    fn ruler_upper_half_draws_a_cycle_and_lower_half_moves_the_cursor() {
        let (mut w, _) = one_event_window();
        w.view.scroll_ticks = 0.0;
        w.snap = false;
        let bar = w.bar_ticks();
        let mut h = window_harness(w);
        let at = |h: &Harness<'_, ProjectWindow>, ticks: u64, dy: f32| {
            egui::pos2(screen(h, ticks, 0.0).x, h.state().lanes_rect.top() - dy)
        };
        let none = egui::Modifiers::NONE;
        let (a, b) = (at(&h, 2 * bar, 22.0), at(&h, 4 * bar, 22.0));
        drag_with(&mut h, a, b, none, none);
        let w = h.state();
        assert_eq!((w.left_locator, w.right_locator), (2 * bar, 4 * bar));
        let (a, b) = (at(&h, bar, 7.0), at(&h, 3 * bar, 7.0));
        drag_with(&mut h, a, b, none, none);
        let w = h.state();
        assert_eq!(w.cursor, 3 * bar, "lower half scrubs the cursor");
        assert_eq!(
            (w.left_locator, w.right_locator),
            (2 * bar, 4 * bar),
            "locators unchanged"
        );
        // Ctrl/Alt + click still set the locators.
        let p = at(&h, 5 * bar, 7.0);
        h.event(egui::Event::PointerMoved(p));
        h.event(egui::Event::ModifiersChanged(egui::Modifiers::ALT));
        h.run_steps(1);
        for pressed in [true, false] {
            h.event(egui::Event::PointerButton {
                pos: p,
                button: egui::PointerButton::Primary,
                pressed,
                modifiers: egui::Modifiers::ALT,
            });
            h.run_steps(1);
        }
        h.event(egui::Event::ModifiersChanged(egui::Modifiers::NONE));
        h.run_steps(1);
        assert_eq!(h.state().right_locator, 5 * bar);
    }

    #[test]
    fn stop_returns_to_the_start_position_and_twice_jumps_to_start_or_left_locator() {
        let (mut core, ctl) = studiocraft_engine::engine(48_000, TempoMap::default());
        let mut w = ProjectWindow::new(Project::default(), Some(ctl), String::new());
        w.import_clip("A", None, tone_clip(4.0, 220.0), None)
            .unwrap();
        w.rebuild_song();
        let mut buf = vec![0.0f32; 1024];
        let mut run = |w: &mut ProjectWindow, core: &mut studiocraft_engine::EngineCore| {
            for _ in 0..20 {
                core.process(&mut buf);
            }
            w.sync_from_engine();
        };
        w.return_on_stop = true;
        w.locate(960);
        w.play();
        run(&mut w, &mut core);
        assert!(w.is_playing());
        w.stop();
        run(&mut w, &mut core);
        assert!(!w.is_playing());
        assert_eq!(w.cursor, 960, "back to where playback started");
        // Stopped: stop again goes to the start, or the left locator if chosen.
        w.stop();
        assert_eq!(w.cursor, 0);
        w.left_locator = 1920;
        w.stop_twice = settings::StopTarget::LeftLocator;
        w.stop();
        assert_eq!(w.cursor, 1920);
        // Without return-on-stop the cursor stays where playback stopped.
        w.return_on_stop = false;
        w.play();
        run(&mut w, &mut core);
        w.stop();
        run(&mut w, &mut core);
        assert!(w.cursor > 1920, "stays at the stop position: {}", w.cursor);
    }

    #[test]
    fn render_device_dialog() {
        shot("devices", |w| {
            w.open_device_dialog();
            w.device_dialog.choice.block_frames = Some(128);
        });
    }

    #[test]
    fn device_dialog_requests_a_switch_and_turns_midi_off() {
        let (w, _) = one_event_window();
        let mut h = window_harness(w);
        h.get_by_label("Transport").click();
        h.run_steps(2);
        h.get_by_label("Audio- und MIDI-Geräte …").click();
        h.run_steps(2);
        assert!(h.state().device_dialog.open);
        h.get_by_role_and_label(egui::accesskit::Role::ComboBox, "Blockgröße")
            .click();
        h.run_steps(2);
        let rate = h.state().sample_rate();
        let item = format!("128 ({:.1} ms)", devices::block_ms(128, rate));
        h.get_by_label(&item).click();
        h.run_steps(2);
        h.get_by_label("Übernehmen").click();
        h.run_steps(2);
        let req = h.state_mut().take_device_request().unwrap();
        assert_eq!(req.block_frames, Some(128));
        assert_eq!(h.state().audio_choice().block_frames, Some(128));
        assert!(h.state_mut().take_device_request().is_none(), "taken once");
        h.state_mut()
            .audio_switched(Ok("Test · Gerät · 48000 Hz · Block 128".into()));
        h.run_steps(2);
        h.get_by_label("Übernommen");
        assert!(h.state().audio_status.contains("Block 128"));
        h.get_by_label("MIDI-Eingänge").click();
        h.run_steps(2);
        assert!(!h.state().midi_inputs_on);
        assert_eq!(h.state().midi_status, "MIDI: aus");
    }

    #[test]
    fn render_crossfade_editor() {
        shot("crossfade", |w| {
            let bar = w.bar_ticks();
            let a = w
                .import_clip("Take 1", None, tone_clip(8.0, 220.0), Some((0, bar)))
                .unwrap();
            let clip = w.project.event(a).unwrap().clip.unwrap();
            let b = w.project.add_audio_event(0, 4 * bar, clip).unwrap();
            w.project.crossfade(a, b);
            w.project.event_mut(a).unwrap().fade_out_curve =
                studiocraft_project::FadeCurve::EqualPower;
            w.project.event_mut(b).unwrap().fade_in_curve =
                studiocraft_project::FadeCurve::EqualPower;
            w.view.scroll_ticks = 0.0;
            w.view.px_per_quarter = 40.0;
            w.view.row_h = 90.0;
            w.open_fade_editor(b, fades_ui::Side::In);
        });
    }

    #[test]
    fn render_tempo_track() {
        shot("tempo", |w| {
            let t = w.add_tempo_track();
            let bar = w.bar_ticks();
            let mut map = w.project.tempo.clone();
            map.set_event(2 * bar, 140.0, false);
            map.set_event(4 * bar, 90.0, true);
            map.set_event(6 * bar, 120.0, false);
            map.set_signature(
                5,
                studiocraft_project::TimeSignature {
                    numerator: 3,
                    denominator: 4,
                },
            );
            w.project.set_tempo_map(map);
            // Tempo track at the top, selected, so that the curve and the inspector show.
            let track = w.project.tracks.remove(t);
            w.project.tracks.insert(0, track);
            w.selected_track = Some(0);
            w.view.scroll_ticks = 0.0;
            w.cursor = 5 * bar;
        });
    }

    #[test]
    fn double_click_on_a_crossfade_opens_the_crossfade_editor() {
        let (mut w, a) = one_event_window();
        w.view.scroll_ticks = 0.0;
        let (start, len) = {
            let e = w.project.event(a).unwrap();
            (e.start.0, e.length)
        };
        // A second take overlapping the last bar of the first, then X.
        let clip = w.project.event(a).unwrap().clip.unwrap();
        let b = w
            .project
            .add_audio_event(0, start + len - 3840, clip)
            .unwrap();
        assert!(w.project.crossfade(a, b));
        w.clear_history();
        let mut h = window_harness(w);
        // The fade-in handle of the later event sits at the end of the overlap.
        let handle = screen(&h, start + len, 0.2);
        double_click_at(&mut h, handle);
        assert!(
            h.state().crossfade_editor.is_some(),
            "crossfade editor is open"
        );
        assert!(h.state().fade_editor.is_none());
        h.get_by_label("Gleiche Leistung").click();
        h.run_steps(3);
        let p = &h.state().project;
        let eq = studiocraft_project::FadeCurve::EqualPower;
        assert_eq!(p.event(a).unwrap().fade_out_curve, eq);
        assert_eq!(p.event(b).unwrap().fade_in_curve, eq);
        h.state_mut().undo();
        let p = &h.state().project;
        assert_eq!(
            p.event(b).unwrap().fade_in_curve,
            studiocraft_project::FadeCurve::Linear
        );
    }

    #[test]
    fn tempo_track_points_are_drawn_dragged_and_ramped() {
        let (mut w, id) = one_event_window();
        w.view.scroll_ticks = 0.0;
        let bar = w.bar_ticks();
        let audio_secs = w.project.tempo.seconds(w.project.event(id).unwrap().start);
        let mut h = window_harness(w);
        h.get_by_label("Projekt").click();
        h.run_steps(2);
        h.get_by_label("Tempospur hinzufügen").click();
        h.run_steps(2);
        let row = h.state().tempo_track().unwrap();
        assert!(h.state().project.tempo.use_track);
        // Fixed view, so that screen positions match the drawing.
        h.state_mut().view.scroll_ticks = 0.0;
        h.state_mut().view.px_per_quarter = 24.0;
        h.run_steps(2);
        // Pencil: a point at bar 3, at the height of the start tempo (middle of the lane).
        h.state_mut().tool = Tool::Draw;
        let at = screen(&h, 2 * bar, row as f32 + 0.5);
        h.event(egui::Event::PointerMoved(at));
        h.run_steps(1);
        for pressed in [true, false] {
            h.event(egui::Event::PointerButton {
                pos: at,
                button: egui::PointerButton::Primary,
                pressed,
                modifiers: egui::Modifiers::NONE,
            });
            h.run_steps(1);
        }
        let ev = h.state().project.tempo.events.clone();
        assert_eq!(ev.len(), 1);
        assert_eq!(ev[0].tick, 2 * bar);
        assert!((ev[0].bpm - 120.0).abs() < 1.0, "{}", ev[0].bpm);
        // Select tool: drag the point up; the tempo rises. The left zone grows once to the
        // width of the tempo table in the inspector: let the layout settle before measuring.
        h.state_mut().tool = Tool::Select;
        h.run_steps(4);
        let w = h.state();
        let p = egui::pos2(
            screen(&h, 2 * bar, 0.0).x,
            w.lanes_rect.top() + w.view.row_h * row as f32 + w.view.row_h / 2.0,
        );
        let none = egui::Modifiers::NONE;
        drag_with(&mut h, p, p - egui::vec2(0.0, 15.0), none, none);
        let bpm = h.state().project.tempo.events[0].bpm;
        assert!(bpm > 125.0, "{bpm}");
        // Audio stays at its time in seconds.
        let p = &h.state().project;
        let now = p.tempo.seconds(p.event(id).unwrap().start);
        assert!((now - audio_secs).abs() < 1e-3);
        // Inspector: ramp on.
        h.state_mut().selected_track = Some(row);
        h.run_steps(2);
        h.get_by_label("Rampe").click();
        h.run_steps(3);
        assert!(h.state().project.tempo.events[0].ramp);
        // The transport shows the tempo at the cursor and the track switch.
        h.get_by_label("Spur");
        h.state_mut().undo();
        assert!(
            !h.state().project.tempo.events[0].ramp,
            "ramp is one undo step"
        );
    }

    #[test]
    fn signature_changes_drive_snap_keypad_and_the_inspector() {
        use studiocraft_project::TimeSignature;
        let q = studiocraft_project::TICKS_PER_QUARTER;
        let (mut w, _) = one_event_window();
        // From bar 3 on 3/4: bar 4 starts at 11 quarters.
        w.project.tempo.set_signature(
            3,
            TimeSignature {
                numerator: 3,
                denominator: 4,
            },
        );
        w.snap = true;
        w.grid = tools::Grid::Bar;
        w.grid_zoom = false;
        assert_eq!(w.snap_ticks((11 * q) as f64 + 300.0), 11 * q);
        assert_eq!(
            w.snap_ticks((12 * q) as f64 + 600.0),
            14 * q,
            "next 3/4 bar"
        );
        w.grid = tools::Grid::Beat;
        assert_eq!(w.snap_ticks((11 * q) as f64 + 700.0), 12 * q);
        // Keypad minus/plus step along the real bar lines.
        w.cursor = 11 * q + 10;
        w.run_keypad(keypad::KeypadPress {
            key: keypad::KeypadKey::Subtract,
            shift: false,
            repeat: false,
        });
        assert_eq!(w.cursor, 11 * q, "to the start of this bar");
        w.run_keypad(keypad::KeypadPress {
            key: keypad::KeypadKey::Add,
            shift: false,
            repeat: false,
        });
        assert_eq!(w.cursor, 14 * q);
        // Inspector of the tempo track: a new change at the cursor's bar, one undo step.
        let t = w.add_tempo_track();
        w.selected_track = Some(t);
        w.clear_history();
        let mut h = window_harness(w);
        h.get_by_label("Taktwechsel am Cursor").click();
        h.run_steps(3);
        let bars: Vec<u64> = h
            .state()
            .project
            .tempo
            .signatures
            .iter()
            .map(|e| e.bar)
            .collect();
        assert_eq!(bars, [3, 5]);
        h.state_mut().undo();
        assert_eq!(h.state().project.tempo.signatures.len(), 1);
        // The position display uses the changed bars.
        assert_eq!(
            h.state()
                .project
                .tempo
                .format_position(studiocraft_project::Position(14 * q)),
            "5.1.1.000"
        );
    }

    #[test]
    fn the_metronome_menu_sets_the_precount_in_one_undo_step() {
        let (w, _) = one_event_window();
        let mut h = window_harness(w);
        h.get_by_label("Transport").click();
        h.run_steps(2);
        h.get_by_label("Metronom").click();
        h.run_steps(2);
        h.get_by_label("Vorzähler").click();
        h.run_steps(2);
        h.get_by_label("2 Takte").click();
        h.run_steps(3);
        assert_eq!(h.state().project.click.precount_bars, 2);
        h.state_mut().undo();
        assert_eq!(h.state().project.click.precount_bars, 0);
    }

    #[test]
    fn audio_connections_dialog_adds_busses_tracks_use_them() {
        let (mut w, _) = one_event_window();
        w.selected_track = Some(0);
        w.set_output_channels(4);
        let mut h = window_harness(w);
        h.key_press(Key::F4);
        h.run_steps(2);
        assert!(h.state().io_dialog, "F4 opens the dialog");
        h.get_by_label("+ Mono-Eingang").click();
        h.run_steps(2);
        h.get_by_label("+ Stereo-Ausgang").click();
        h.run_steps(2);
        let io = h.state().project.io.clone();
        assert_eq!(io.inputs.len(), 2);
        assert_eq!(io.inputs[1].name, "Eingang 3");
        assert_eq!(io.outputs.len(), 2);
        assert_eq!(io.outputs[1].ports_label(), "3/4");
        // The main mix cannot be removed.
        assert!(
            !h.state_mut()
                .project
                .remove_io_bus(true, studiocraft_project::MAIN_OUT)
        );
        // Inspector: the track takes the mono input and the new output bus.
        let (inp, out) = (io.inputs[1].id, io.outputs[1].id);
        h.state_mut().io_dialog = false;
        h.run_steps(1);
        h.get_by_label("Eingang").click();
        h.run_steps(2);
        h.get_by_label("Eingang 3 (3)").click();
        h.run_steps(2);
        assert_eq!(h.state().project.tracks[0].input_bus, Some(inp));
        assert_eq!(
            h.state().project.input_of(0),
            studiocraft_project::TrackInput {
                first: 2,
                stereo: false
            }
        );
        h.get_by_label("Ausgang").click();
        h.run_steps(2);
        h.get_all_by_label("Ausgang 3/4").last().unwrap().click();
        h.run_steps(2);
        assert_eq!(h.state().project.tracks[0].out_bus, Some(out));
        // The song routes the track to output bus 1 (behind the main mix).
        let w = h.state();
        let song = studiocraft_engine::song::Song::build(&w.project, &Default::default(), 48_000);
        assert_eq!(song.outputs, [(0, true), (2, true)]);
        assert!(
            song.tracks
                .iter()
                .all(|t| t.output == studiocraft_engine::song::Output::Hw(1))
        );
        // Undo: back to the main mix.
        h.state_mut().undo();
        assert_eq!(h.state().project.tracks[0].out_bus, None);
    }

    #[test]
    fn key_commands_can_be_reassigned_with_a_conflict_warning() {
        use keymap::Cmd;
        let (w, _) = one_event_window();
        let mut h = window_harness(w);
        h.get_by_label("Bearbeiten").click();
        h.run_steps(2);
        h.get_by_label("Tastaturbefehle …").click();
        h.run_steps(2);
        assert!(h.state().key_dialog.open);
        // Snap from J to K.
        h.state_mut().key_dialog.capturing = Some(Cmd::Snap);
        h.run_steps(1);
        h.key_press(Key::K);
        h.run_steps(2);
        let k = egui::KeyboardShortcut::new(egui::Modifiers::NONE, Key::K);
        assert_eq!(h.state().keymap.bindings(Cmd::Snap).last(), Some(&k));
        // K for the metronome now clashes: the dialog asks, "Ersetzen" moves it.
        h.state_mut().key_dialog.capturing = Some(Cmd::Metronome);
        h.run_steps(1);
        h.key_press(Key::K);
        h.run_steps(2);
        assert!(h.state().key_dialog.conflict.is_some());
        h.get_by_label("Ersetzen").click();
        h.run_steps(2);
        assert!(!h.state().keymap.bindings(Cmd::Snap).contains(&k));
        assert!(h.state().keymap.bindings(Cmd::Metronome).contains(&k));
        // Close the dialog: J still toggles snap (it kept J), K no longer does.
        h.state_mut().key_dialog.open = false;
        h.run_steps(1);
        let snap = h.state().snap;
        h.key_press(Key::J);
        h.run_steps(1);
        assert_ne!(h.state().snap, snap);
        h.key_press(Key::K);
        h.run_steps(1);
        assert_eq!(h.state().snap, !snap, "K is the metronome now, not snap");
    }

    #[test]
    fn renders_key_commands_dialog() {
        shot("tastaturbefehle", |w| {
            w.key_dialog.open = true;
            w.keymap.assign(
                keymap::Cmd::Metronome,
                egui::KeyboardShortcut::new(egui::Modifiers::NONE, Key::J),
            );
        });
    }

    #[test]
    fn x_crossfades_overlapping_events() {
        let mut p = Project::default();
        let t = p.add_track(TrackKind::Audio, "A");
        let a = p.create_event(t, 0, 3840).unwrap();
        let b = p.create_event(t, 2880, 3840).unwrap();
        let mut w = ProjectWindow::new(p, None, String::new());
        w.selection = vec![b];
        let mut h = window_harness(w);
        h.key_press(Key::X);
        h.run_steps(1);
        let p = &h.state().project;
        assert_eq!(p.event(a).map(|e| e.fade_out), Some(960));
        assert_eq!(p.event(b).map(|e| e.fade_in), Some(960));
        h.state_mut().undo();
        assert_eq!(h.state().project.event(b).map(|e| e.fade_in), Some(0));
    }

    #[test]
    fn renders_crossfade() {
        shot("crossfade", |w| {
            let bar = w.bar_ticks();
            w.project.tracks[0].events.clear();
            let a = w.project.create_event(0, 0, 3 * bar).unwrap();
            let b = w.project.create_event(0, 2 * bar, 3 * bar).unwrap();
            w.project.crossfade(a, b);
            w.project.tracks[0].show_lanes = true;
        });
    }

    #[test]
    fn eq_section_shapes_the_curve_and_saves_with_the_project() {
        let mut w = ProjectWindow::new(Project::demo(4), None, "kein Audio (Test)".into());
        w.selected_track = Some(0);
        w.project.tracks[0].eq.bands[1].gain = 6.0;
        w.project.tracks[0].eq.bands[0].kind = studiocraft_project::BandKind::LowCut;
        w.project.tracks[0].eq.bands[3].gain = -4.0;
        let mut h = Harness::builder()
            .with_size(egui::vec2(1400.0, 1000.0))
            .with_step_dt(1.0 / 60.0)
            .wgpu()
            .build_ui_state(|ui, w: &mut ProjectWindow| w.ui(ui), w);
        h.run_steps(2);
        h.get_by_label("EQ").click();
        h.run_steps(3);
        // The section is open: band 2's gain field is there, labelled "dB".
        assert!(h.query_all_by_label("dB").count() >= 4);
        if let (Ok(dir), Ok(img)) = (std::env::var("STUDIOCRAFT_SHOTS"), h.render()) {
            img.save(std::path::Path::new(&dir).join("eq.png")).unwrap();
        }
        h.get_by_label("EQ an").click();
        h.run_steps(2);
        assert!(!h.state().project.tracks[0].eq.on);
        let json = studiocraft_project::file::to_json(&h.state().project);
        let back = studiocraft_project::file::from_json(&json).unwrap();
        assert_eq!(back.tracks[0].eq, h.state().project.tracks[0].eq);
    }

    #[test]
    fn folder_tracks_hold_tracks_fold_away_and_mute_them() {
        let mut p = Project::default();
        let a = p.add_track(TrackKind::Audio, "Gesang");
        let b = p.add_track(TrackKind::Audio, "Gitarre");
        let mut w = ProjectWindow::new(p, None, String::new());
        w.selected_track = Some(b);
        let mut h = window_harness(w);
        h.get_by_label("Projekt").click();
        h.run_steps(2);
        h.get_by_label("Ordner-Spur hinzufügen").click();
        h.run_steps(2);
        let folder = 2;
        assert_eq!(h.state().project.tracks[folder].kind, TrackKind::Folder);
        assert_eq!(h.state().project.parent_of(b), Some(folder));
        // Display: Gesang, then the folder with Gitarre below it.
        let order: Vec<usize> = h
            .state()
            .project
            .display_order()
            .iter()
            .map(|r| r.0)
            .collect();
        assert_eq!(order, [a, folder, b]);
        // The arrow on the folder's header (second row, upper line, right of the colour
        // strip) folds it: Gitarre disappears.
        let arrow = {
            let w = h.state();
            egui::pos2(
                w.lanes_rect.left() - 290.0 + 29.0,
                w.lanes_rect.top() + 1.5 * w.view.row_h - 10.0,
            )
        };
        h.event(egui::Event::PointerMoved(arrow));
        h.run_steps(1);
        for pressed in [true, false] {
            h.event(egui::Event::PointerButton {
                pos: arrow,
                button: egui::PointerButton::Primary,
                pressed,
                modifiers: egui::Modifiers::NONE,
            });
        }
        h.run_steps(2);
        assert!(h.state().project.tracks[folder].folded);
        assert!(h.state().project.is_hidden(b));
        // Muting the folder mutes its track for playback.
        h.state_mut().project.tracks[folder].mute = true;
        assert!(!h.state().project.audible(b));
        assert!(h.state().project.audible(a));
        h.state_mut().undo();
        assert!(!h.state().project.tracks[folder].folded);
    }

    #[test]
    fn renders_folder_tracks() {
        shot("ordner", |w| {
            w.project = Project::demo(8);
            let f = w.project.add_track(TrackKind::Folder, "Rhythmus");
            for t in [1, 2, 3] {
                w.project.set_parent(t, Some(f));
            }
            let g = w.project.add_track(TrackKind::Folder, "Leer");
            w.project.tracks[g].folded = true;
            w.project.set_parent(5, Some(g));
        });
    }

    #[test]
    fn marker_track_shows_markers_and_cycle_markers_set_the_locators() {
        let (mut w, _) = one_event_window();
        let bar = w.bar_ticks();
        w.cursor = 2 * bar;
        w.insert_marker();
        let m = w.marker_track().expect("created on demand");
        w.left_locator = 4 * bar;
        w.right_locator = 6 * bar;
        w.cycle_marker_from_locators();
        assert_eq!(w.project.markers.len(), 2);
        w.left_locator = 0;
        w.right_locator = bar;
        w.view.scroll_ticks = 0.0;
        // Herausgezoomt, damit alle Klickziele (bis Takt 7) sicher sichtbar sind.
        w.view.px_per_quarter = 12.0;
        let mut h = window_harness(w);
        let row_y = |h: &Harness<'_, ProjectWindow>| {
            let w = h.state();
            w.lanes_rect.top() + (m as f32 + 0.8) * w.view.row_h
        };
        let x_of = |h: &Harness<'_, ProjectWindow>, t: u64| {
            let w = h.state();
            w.lanes_rect.left()
                + ((t as f64 - w.view.scroll_ticks) / TPQ) as f32 * w.view.px_per_quarter
        };
        let click = |h: &mut Harness<'_, ProjectWindow>, pos: egui::Pos2| {
            h.event(egui::Event::PointerMoved(pos));
            h.run_steps(1);
            for pressed in [true, false] {
                h.event(egui::Event::PointerButton {
                    pos,
                    button: egui::PointerButton::Primary,
                    pressed,
                    modifiers: egui::Modifiers::NONE,
                });
            }
            h.run_steps(2);
        };
        // A click into the cycle marker sets the locators to its range.
        let y = row_y(&h);
        let pos = egui::pos2(x_of(&h, 5 * bar), y);
        click(&mut h, pos);
        assert_eq!(
            (h.state().left_locator, h.state().right_locator),
            (4 * bar, 6 * bar)
        );
        // The erase tool removes the position marker at bar 3.
        h.state_mut().tool = Tool::Erase;
        let pos = egui::pos2(x_of(&h, 2 * bar) + 2.0, y);
        click(&mut h, pos);
        assert!(h.state().project.marker(3).is_none());
        assert_eq!(h.state().project.markers.len(), 1);
        // The draw tool adds one where there is none.
        h.state_mut().tool = Tool::Draw;
        let pos = egui::pos2(x_of(&h, 7 * bar), y);
        click(&mut h, pos);
        assert_eq!(h.state().project.marker(3).map(|m| m.start), Some(7 * bar));
    }

    #[test]
    fn insert_key_adds_a_marker_and_goto_commands_can_be_bound() {
        use keymap::Cmd;
        let (mut w, _) = one_event_window();
        w.cursor = 1920;
        let mut h = window_harness(w);
        h.key_press(Key::Insert);
        h.run_steps(1);
        assert_eq!(h.state().project.marker(3).map(|m| m.start), Some(1920));
        assert!(h.state().marker_track().is_some());
        // "Zu Marker 3" has no default key; bound to Shift+3 it jumps there.
        let key = egui::KeyboardShortcut::new(egui::Modifiers::SHIFT, Key::Num3);
        h.state_mut().keymap.assign(Cmd::GotoMarker(3), key);
        h.state_mut().cursor = 0;
        h.key_press_modifiers(egui::Modifiers::SHIFT, Key::Num3);
        h.run_steps(1);
        assert_eq!(h.state().cursor, 1920);
    }

    #[test]
    fn renders_marker_track() {
        shot("marker", |w| {
            w.project = Project::demo(6);
            let bar = w.bar_ticks();
            w.add_marker_track();
            for (i, at) in [1, 5, 9].into_iter().enumerate() {
                w.project.add_marker(at * bar);
                if let Some(m) = w.project.markers.last_mut() {
                    m.name = ["Intro", "Strophe", "Refrain"][i].into();
                }
            }
            w.project.add_cycle_marker(5 * bar, 9 * bar);
            w.selected_track = w.marker_track();
        });
    }

    /// Zieht von `from` nach `to` (Bildschirm) mit eigenen Modifiern für Drücken, Bewegen und
    /// Loslassen.
    fn drag_with(
        h: &mut Harness<'_, ProjectWindow>,
        from: egui::Pos2,
        to: egui::Pos2,
        press: egui::Modifiers,
        moving: egui::Modifiers,
    ) {
        let button = |pos, pressed, modifiers| egui::Event::PointerButton {
            pos,
            button: egui::PointerButton::Primary,
            pressed,
            modifiers,
        };
        h.event(egui::Event::PointerMoved(from));
        h.run_steps(1);
        h.event(egui::Event::ModifiersChanged(press));
        h.event(button(from, true, press));
        h.run_steps(1);
        // The drag starts with the press modifiers; `moving` applies from the second step.
        for i in 1..=6 {
            h.event(egui::Event::PointerMoved(
                from + (to - from) * (i as f32 / 6.0),
            ));
            h.run_steps(1);
            if i == 1 {
                h.event(egui::Event::ModifiersChanged(moving));
            }
        }
        h.event(button(to, false, moving));
        h.run_steps(2);
        h.event(egui::Event::ModifiersChanged(egui::Modifiers::NONE));
        h.run_steps(1);
    }

    fn two_part_window() -> (ProjectWindow, EventId) {
        let mut p = Project::default();
        let t = p.add_track(TrackKind::Instrument, "Synth");
        p.add_track(TrackKind::Instrument, "Bass");
        let a = p.create_event(t, 0, 3840).unwrap();
        p.event_mut(a).unwrap().notes = vec![studiocraft_project::Note {
            pitch: 60,
            velocity: 100,
            start: 0,
            length: 480,
        }];
        let mut w = ProjectWindow::new(p, None, String::new());
        w.view.scroll_ticks = 0.0;
        w.snap = false;
        (w, a)
    }

    fn screen(h: &Harness<'_, ProjectWindow>, ticks: u64, rows: f32) -> egui::Pos2 {
        let w = h.state();
        egui::pos2(
            w.lanes_rect.left()
                + ((ticks as f64 - w.view.scroll_ticks) / TPQ) as f32 * w.view.px_per_quarter,
            w.lanes_rect.top() + rows * w.view.row_h,
        )
    }

    #[test]
    fn ctrl_shift_drag_makes_a_shared_copy_that_follows_edits() {
        let (w, a) = two_part_window();
        let mut h = window_harness(w);
        let from = screen(&h, 1920, 0.8);
        let to = screen(&h, 1920 + 3840, 0.8);
        let cs = egui::Modifiers::COMMAND.plus(egui::Modifiers::SHIFT);
        drag_with(&mut h, from, to, cs, cs);
        let evs = &h.state().project.tracks[0].events;
        assert_eq!(evs.len(), 2, "copied, not moved");
        assert!(evs.iter().all(|e| e.shared.is_some()));
        let copy = evs.iter().find(|e| e.id != a).unwrap().id;
        // Editing the copy changes the original too (one undo step).
        let w = h.state_mut();
        let before = w.project.clone();
        w.project.event_mut(copy).unwrap().notes[0].pitch = 72;
        w.checkpoint(before);
        assert_eq!(w.project.event(a).unwrap().notes[0].pitch, 72);
    }

    #[test]
    fn ctrl_pressed_during_a_drag_locks_the_direction() {
        let (w, a) = two_part_window();
        let mut h = window_harness(w);
        // Diagonal drag: one bar right and one track down, Ctrl only while moving.
        let from = screen(&h, 1920, 0.8);
        let to = screen(&h, 1920 + 3840, 1.6);
        drag_with(
            &mut h,
            from,
            to,
            egui::Modifiers::NONE,
            egui::Modifiers::COMMAND,
        );
        let p = &h.state().project;
        assert_eq!(p.tracks[0].events.len(), 1, "moved, not copied");
        assert_eq!(p.find(a).map(|(t, _)| t), Some(0), "stayed on its track");
        assert_eq!(p.event(a).map(|e| e.start.0), Some(3840));
    }

    #[test]
    fn alt_drag_on_the_right_edge_repeats_and_ctrl_k_opens_the_dialog() {
        let (mut w, a) = two_part_window();
        w.selection = vec![a];
        let mut h = window_harness(w);
        let edge = screen(&h, 3840, 0.8) - egui::vec2(2.0, 0.0);
        let to = screen(&h, 3 * 3840, 0.8);
        drag_with(&mut h, edge, to, egui::Modifiers::ALT, egui::Modifiers::ALT);
        let starts: Vec<u64> = h.state().project.tracks[0]
            .events
            .iter()
            .map(|e| e.start.0)
            .collect();
        assert_eq!(starts, [0, 3840, 7680]);
        // Ctrl+K: repeat the selection twice as shared copies.
        h.key_press_modifiers(egui::Modifiers::COMMAND, Key::K);
        h.run_steps(2);
        assert!(h.state().repeat_dialog.open);
        h.state_mut().repeat_dialog.count = 2;
        h.state_mut().repeat_dialog.shared = true;
        h.get_by_label("Geteilte Kopien");
        h.get_all_by_label("Wiederholen").last().unwrap().click();
        h.run_steps(2);
        // The selection after the drag is both new copies: twice more is four shared copies.
        let evs = &h.state().project.tracks[0].events;
        assert_eq!(evs.len(), 7);
        assert!(!h.state().repeat_dialog.open);
        assert_eq!(evs.iter().filter(|e| e.shared.is_some()).count(), 6);
    }

    #[test]
    fn ctrl_arrows_nudge_events_by_the_grid() {
        let (mut w, id) = one_event_window();
        w.selection = vec![id];
        let ev = w.project.event(id).cloned().unwrap();
        let step = w.grid_ticks();
        let mut h = window_harness(w);
        h.key_press_modifiers(egui::Modifiers::COMMAND, Key::ArrowRight);
        h.run_steps(1);
        assert_eq!(
            h.state().project.event(id).map(|e| e.start.0),
            Some(ev.start.0 + step)
        );
        let cs = egui::Modifiers::COMMAND.plus(egui::Modifiers::SHIFT);
        h.key_press_modifiers(cs, Key::ArrowLeft);
        h.run_steps(1);
        assert_eq!(
            h.state().project.event(id).map(|e| e.length),
            Some(ev.length - step)
        );
        // Plain arrows still select the neighbouring event, not nudge.
        h.key_press(Key::ArrowRight);
        h.run_steps(1);
        assert_eq!(
            h.state().project.event(id).map(|e| e.start.0),
            Some(ev.start.0 + step)
        );
        h.state_mut().undo();
        h.state_mut().undo();
        assert_eq!(
            h.state().project.event(id).map(|e| (e.start.0, e.length)),
            Some((ev.start.0, ev.length))
        );
    }

    /// Instrument-Part mit zwei Noten, im Key-Editor geöffnet und fokussiert.
    fn key_editor_window() -> (ProjectWindow, EventId) {
        let mut p = Project::default();
        let t = p.add_track(TrackKind::Instrument, "Synth");
        let part = p.create_event(t, 0, 4 * 3840).unwrap();
        p.event_mut(part).unwrap().notes = vec![
            studiocraft_project::Note {
                pitch: 60,
                velocity: 100,
                start: 0,
                length: 480,
            },
            studiocraft_project::Note {
                pitch: 64,
                velocity: 100,
                start: 480,
                length: 480,
            },
        ];
        let mut w = ProjectWindow::new(p, None, String::new());
        w.open_key_editor(part);
        w.key_editor.focused = true;
        w.key_editor.selection = vec![0];
        (w, part)
    }

    #[test]
    fn key_editor_info_line_edits_notes_relatively() {
        let (mut w, part) = key_editor_window();
        w.key_editor.selection = vec![0, 1];
        let mut h = window_harness(w);
        h.run_steps(2);
        let enter = |h: &mut Harness<'_, ProjectWindow>, label: &str, text: &str| {
            h.get_by_label(label).focus();
            h.run_steps(1);
            h.key_press_modifiers(egui::Modifiers::COMMAND, Key::A);
            h.run_steps(1);
            h.get_by_label(label).type_text(text);
            h.run_steps(1);
            h.key_press(Key::Enter);
            h.run_steps(2);
        };
        // The first note is C3 (60); D3 moves both notes up two semitones.
        enter(&mut h, "Tonhöhe", "D3");
        let notes = h.state().project.event(part).unwrap().notes.clone();
        assert_eq!((notes[0].pitch, notes[1].pitch), (62, 66));
        // Velocity 80 lowers both by 20.
        enter(&mut h, "Velocity", "80");
        let notes = h.state().project.event(part).unwrap().notes.clone();
        assert_eq!((notes[0].velocity, notes[1].velocity), (80, 80));
        // Each change is one undo step.
        h.state_mut().undo();
        let notes = h.state().project.event(part).unwrap().notes.clone();
        assert_eq!(notes[0].velocity, 100);
        assert_eq!(notes[0].pitch, 62);
    }

    #[test]
    fn renders_key_editor_info_line() {
        shot("key-editor-info", |w| {
            let row = w
                .project
                .add_track(TrackKind::Instrument, String::from("Synth"));
            let part = w.project.create_event(row, 0, 4 * 3840).unwrap();
            w.project.event_mut(part).unwrap().notes = vec![studiocraft_project::Note {
                pitch: 60,
                velocity: 96,
                start: 960,
                length: 960,
            }];
            w.open_key_editor(part);
            w.key_editor.selection = vec![0];
        });
    }

    #[test]
    fn renders_key_editor_with_several_parts() {
        shot("key-editor-parts", |w| {
            let row = w
                .project
                .add_track(TrackKind::Instrument, String::from("Synth"));
            let mut ids = Vec::new();
            for (k, name) in ["Strophe", "Refrain"].iter().enumerate() {
                let id = w.project.create_event(row, k as u64 * 3840, 3840).unwrap();
                let ev = w.project.event_mut(id).unwrap();
                ev.name = (*name).into();
                ev.notes = (0..4)
                    .map(|i| studiocraft_project::Note {
                        pitch: 60 + 2 * i as u8 + 5 * k as u8,
                        velocity: 100,
                        start: i * 960,
                        length: 720,
                    })
                    .collect();
                ids.push(id);
            }
            w.open_key_editor_parts(&ids);
        });
    }

    #[test]
    fn several_parts_open_together_one_active() {
        let mut p = Project::default();
        let t = p.add_track(TrackKind::Instrument, "Synth");
        let a = p.create_event(t, 0, 3840).unwrap();
        let b = p.create_event(t, 3840, 3840).unwrap();
        p.event_mut(a).unwrap().name = "Strophe".into();
        p.event_mut(b).unwrap().name = "Refrain".into();
        for (id, pitch) in [(a, 60), (b, 67)] {
            p.event_mut(id).unwrap().notes = vec![studiocraft_project::Note {
                pitch,
                velocity: 100,
                start: 0,
                length: 960,
            }];
        }
        let mut w = ProjectWindow::new(p, None, String::new());
        w.selection = vec![a, b];
        w.open_key_editor_from_selection(b);
        assert_eq!(w.key_editor.part, Some(b), "the clicked part is active");
        assert_eq!(w.key_editor.open.len(), 2);
        let mut h = window_harness(w);
        h.run_steps(2);
        // Tabs switch the active part.
        h.get_by_label("Strophe").click();
        h.run_steps(2);
        assert_eq!(h.state().key_editor.part, Some(a));
        // A click on the dimmed note of the other part makes that part active and selects it.
        let pos = {
            let s = h.state();
            let g = s.key_editor.grid_rect;
            let v = s.key_editor.view;
            let x = g.left() + ((3840.0 + 480.0 - v.scroll_ticks) / TPQ) as f32 * v.px_per_quarter;
            let y = g.top() + (127 - 67) as f32 * v.key_h - v.scroll_y + v.key_h / 2.0;
            egui::pos2(x, y)
        };
        h.event(egui::Event::PointerMoved(pos));
        h.run_steps(1);
        for pressed in [true, false] {
            h.event(egui::Event::PointerButton {
                pos,
                button: egui::PointerButton::Primary,
                pressed,
                modifiers: egui::Modifiers::NONE,
            });
        }
        h.run_steps(2);
        assert_eq!(h.state().key_editor.part, Some(b));
        assert_eq!(h.state().key_editor.selection, vec![0]);
        // A single part opens alone again.
        h.state_mut().selection = vec![a];
        h.state_mut().open_key_editor_from_selection(a);
        assert_eq!(h.state().key_editor.open, vec![a]);
    }

    #[test]
    fn controller_lanes_draw_select_move_and_delete() {
        use studiocraft_project::CtrlKind;
        let (mut w, part) = key_editor_window();
        w.key_editor.selection.clear();
        w.add_ctrl_lane(CtrlKind::Cc(1));
        w.add_ctrl_lane(CtrlKind::Cc(1));
        assert_eq!(w.key_editor.lanes.len(), 1, "a kind only once");
        w.tool = Tool::Draw;
        w.snap = true;
        let mut h = window_harness(w);
        h.run_steps(2);
        // Screen point in the controller lane for a tick and a value 0..=1.
        let at = |h: &Harness<'_, ProjectWindow>, tick: f64, frac: f32| {
            let s = h.state();
            let g = s.key_editor.grid_rect;
            let v = s.key_editor.view;
            let top = s.key_editor.vel_rect.bottom() + 2.0;
            let lane_h = 62.0;
            let x = g.left() + ((tick - v.scroll_ticks) / TPQ) as f32 * v.px_per_quarter;
            egui::pos2(x, top + lane_h * (1.0 - frac))
        };
        let ctrls = |h: &Harness<'_, ProjectWindow>| {
            h.state().project.event(part).unwrap().controllers.clone()
        };
        // Draw a ramp over one bar: one point per grid step, rising.
        let (a, b) = (at(&h, 0.0, 0.2), at(&h, 4.0 * TPQ, 0.8));
        drag_with(&mut h, a, b, egui::Modifiers::NONE, egui::Modifiers::NONE);
        let drawn = ctrls(&h);
        assert!(drawn.len() >= 4, "{drawn:?}");
        assert!(drawn.iter().all(|c| c.kind == CtrlKind::Cc(1)));
        let first = drawn.iter().min_by_key(|c| c.start).unwrap();
        let last = drawn.iter().max_by_key(|c| c.start).unwrap();
        assert!(first.value < last.value);
        h.state_mut().undo();
        assert!(ctrls(&h).is_empty(), "one stroke, one undo step");
        h.state_mut().redo();
        // Select the first point and drag it up.
        h.state_mut().tool = Tool::Select;
        let p0 = at(&h, first.start as f64, f32::from(first.value) / 127.0);
        let up = p0 - egui::vec2(0.0, 20.0);
        drag_with(&mut h, p0, up, egui::Modifiers::NONE, egui::Modifiers::NONE);
        let moved = ctrls(&h)
            .into_iter()
            .find(|c| c.start == first.start)
            .unwrap();
        assert!(
            moved.value > first.value,
            "{} -> {}",
            first.value,
            moved.value
        );
        assert_eq!(h.state().key_editor.ctrl_sel.len(), 1);
        // Delete removes the selected point.
        let n = ctrls(&h).len();
        h.key_press(Key::Delete);
        h.run_steps(1);
        assert_eq!(ctrls(&h).len(), n - 1);
        // The eraser removes points under the pointer.
        h.state_mut().tool = Tool::Erase;
        let (a, b) = (at(&h, 0.0, 0.0), at(&h, 4.0 * TPQ, 0.0));
        drag_with(&mut h, a, b, egui::Modifiers::NONE, egui::Modifiers::NONE);
        assert!(ctrls(&h).is_empty());
    }

    #[test]
    fn renders_controller_lanes() {
        shot("controller-spuren", |w| {
            use studiocraft_project::{CtrlKind, CtrlPoint};
            let row = w
                .project
                .add_track(TrackKind::Instrument, String::from("Synth"));
            let part = w.project.create_event(row, 0, 4 * 3840).unwrap();
            let ev = w.project.event_mut(part).unwrap();
            ev.notes = vec![studiocraft_project::Note {
                pitch: 60,
                velocity: 100,
                start: 0,
                length: 3840,
            }];
            ev.controllers = (0..16)
                .map(|i| CtrlPoint {
                    kind: CtrlKind::PitchBend,
                    start: i * 960,
                    value: (8192.0 + 6000.0 * (i as f32 * 0.6).sin()) as u16,
                })
                .chain((0..8).map(|i| CtrlPoint {
                    kind: CtrlKind::Cc(1),
                    start: i * 1920,
                    value: (i * 16) as u16,
                }))
                .collect();
            w.open_key_editor(part);
            w.add_ctrl_lane(CtrlKind::PitchBend);
            w.add_ctrl_lane(CtrlKind::Cc(1));
        });
    }

    #[test]
    fn shift_arrows_transpose_by_an_octave_also_in_the_editor_window() {
        let (w, part) = key_editor_window();
        let mut h = window_harness(w);
        let pitch =
            |h: &Harness<'_, ProjectWindow>| h.state().project.event(part).unwrap().notes[0].pitch;
        h.key_press_modifiers(egui::Modifiers::SHIFT, Key::ArrowUp);
        h.run_steps(1);
        assert_eq!(pitch(&h), 72);
        h.key_press_modifiers(egui::Modifiers::SHIFT, Key::ArrowDown);
        h.key_press_modifiers(egui::Modifiers::SHIFT, Key::ArrowDown);
        h.run_steps(1);
        assert_eq!(pitch(&h), 48);
        // With the editor in its own window the arrows still reach the notes.
        h.state_mut().show_lower = false;
        h.state_mut().windows.editor = Some(windows::EditorWindow::Key);
        h.state_mut().key_editor.focused = true;
        h.key_press(Key::ArrowUp);
        h.run_steps(1);
        assert_eq!(pitch(&h), 49);
    }

    #[test]
    fn f_toggles_follow_and_feedback_can_be_switched_off() {
        let (mut w, _) = key_editor_window();
        assert!(w.follow_playback && w.key_editor.feedback);
        w.key_editor.feedback = false;
        w.note_feedback(60);
        assert_eq!(w.key_editor.preview, None, "no sound with feedback off");
        w.key_editor.feedback = true;
        w.note_feedback(60);
        assert_eq!(w.key_editor.preview, Some(60));
        w.key_editor.focused = false;
        let mut h = window_harness(w);
        h.key_press(Key::F);
        h.run_steps(1);
        assert!(!h.state().follow_playback);
        h.get_by_label("Folgen (F)").click();
        h.run_steps(1);
        assert!(h.state().follow_playback);
    }

    #[test]
    fn key_editor_copies_pastes_transposes_and_nudges_notes() {
        let mut p = Project::default();
        let t = p.add_track(TrackKind::Instrument, "Synth");
        let part = p.create_event(t, 0, 4 * 3840).unwrap();
        p.event_mut(part).unwrap().notes = vec![
            studiocraft_project::Note {
                pitch: 60,
                velocity: 100,
                start: 0,
                length: 480,
            },
            studiocraft_project::Note {
                pitch: 64,
                velocity: 100,
                start: 480,
                length: 480,
            },
        ];
        let mut w = ProjectWindow::new(p, None, String::new());
        w.open_key_editor(part);
        w.key_editor.focused = true;
        w.key_editor.selection = vec![0, 1];
        w.cursor = 3840;
        let mut h = window_harness(w);
        h.key_press_modifiers(egui::Modifiers::COMMAND, Key::C);
        h.run_steps(1);
        h.key_press_modifiers(egui::Modifiers::COMMAND, Key::V);
        h.run_steps(1);
        let notes = &h.state().project.event(part).unwrap().notes;
        assert_eq!(notes.len(), 4);
        assert_eq!((notes[2].start, notes[3].start), (3840, 4320));
        assert_eq!(
            h.state().key_editor.selection,
            vec![2, 3],
            "the pasted notes are selected"
        );
        h.key_press(Key::ArrowUp);
        h.run_steps(1);
        let notes = &h.state().project.event(part).unwrap().notes;
        assert_eq!(
            (notes[2].pitch, notes[3].pitch, notes[0].pitch),
            (61, 65, 60)
        );
        let step = h.state().grid_ticks() as i64;
        h.key_press_modifiers(egui::Modifiers::COMMAND, Key::ArrowRight);
        h.run_steps(1);
        assert_eq!(
            h.state().project.event(part).unwrap().notes[2].start,
            3840 + step
        );
        assert_eq!(
            h.state().project.tracks.len(),
            1,
            "arrows did not change track selection"
        );
    }

    #[test]
    fn sample_editor_processes_non_destructively_and_trims_to_the_selection() {
        let (mut w, id) = one_event_window();
        let old_clip = w.project.event(id).and_then(|e| e.clip).unwrap();
        let old_peak = {
            let s = &w.clip(old_clip).unwrap().samples;
            studiocraft_engine::offline::peak(s, 0..(s.len() / 2) as u64)
        };
        w.open_sample_editor(id);
        assert_eq!(w.lower, LowerZone::SampleEditor);
        w.process_clip(id, sampleeditor::Process::Normalize)
            .unwrap();
        let new_clip = w.project.event(id).and_then(|e| e.clip).unwrap();
        assert_ne!(new_clip, old_clip, "a new clip, the original stays");
        let s = &w.clip(new_clip).unwrap().samples;
        let p = studiocraft_engine::offline::peak(s, 0..(s.len() / 2) as u64);
        assert!((p - 1.0).abs() < 1e-4, "{p}");
        let s = &w.clip(old_clip).unwrap().samples;
        let p = studiocraft_engine::offline::peak(s, 0..(s.len() / 2) as u64);
        assert_eq!(p, old_peak);
        w.undo();
        assert_eq!(w.project.event(id).and_then(|e| e.clip), Some(old_clip));
        // Selection of one second from 0.5 s: the event follows, the audio stays in place.
        let ev = w.project.event(id).cloned().unwrap();
        w.sample_editor.selection = Some((24_000, 72_000));
        w.event_to_selection(id);
        let after = w.project.event(id).cloned().unwrap();
        assert_eq!(after.offset, 24_000);
        let tempo = &w.project.tempo;
        assert_eq!(after.start.0, ev.start.0 + tempo.ticks(24_000, 48_000));
        assert_eq!(after.length, tempo.ticks(48_000, 48_000));
    }

    #[test]
    fn double_click_on_an_audio_event_opens_the_sample_editor() {
        let (w, id) = one_event_window();
        let mut h = window_harness(w);
        let pos = {
            let w = h.state();
            let ev = w.project.event(id).unwrap();
            let t = ev.start.0 + ev.length / 2;
            egui::pos2(
                w.lanes_rect.left()
                    + ((t as f64 - w.view.scroll_ticks) / TPQ) as f32 * w.view.px_per_quarter,
                w.lanes_rect.top() + 0.8 * w.view.row_h,
            )
        };
        h.event(egui::Event::PointerMoved(pos));
        h.run_steps(1);
        for _ in 0..2 {
            for pressed in [true, false] {
                h.event(egui::Event::PointerButton {
                    pos,
                    button: egui::PointerButton::Primary,
                    pressed,
                    modifiers: egui::Modifiers::NONE,
                });
            }
            h.run_steps(1);
        }
        h.run_steps(2);
        assert!(h.state().show_lower);
        assert_eq!(h.state().lower, LowerZone::SampleEditor);
        assert_eq!(h.state().sample_editor.event, Some(id));
        h.get_by_label("Normalisieren");
        h.get_by_label("Event auf Auswahl");
    }

    #[test]
    fn renders_sample_editor() {
        shot("sample-editor", |w| {
            w.project = Project::default();
            let id = w
                .import_clip("Gesang", None, tone_clip(4.0, 220.0), Some((usize::MAX, 0)))
                .unwrap();
            w.open_sample_editor(id);
            w.sample_editor.selection = Some((48_000, 96_000));
        });
    }

    #[test]
    fn toolbar_symbols_pick_tools_and_toggle_zones() {
        let (w, _) = one_event_window();
        let mut h = window_harness(w);
        h.get_by_label("Trennen (3)").click();
        h.run_steps(2);
        assert_eq!(h.state().tool, Tool::Split);
        h.get_by_label("Snap an/aus (J)").click();
        h.run_steps(2);
        assert!(!h.state().snap);
        h.get_by_label("Linke Zone (Alt+L)").click();
        h.run_steps(2);
        assert!(!h.state().show_inspector);
        h.get_by_label("Untere Zone").click();
        h.run_steps(2);
        assert!(!h.state().show_lower);
    }

    #[test]
    fn global_state_switches_clear_mute_solo_and_write() {
        let (mut w, _) = one_event_window();
        w.project.add_track(TrackKind::Audio, String::from("Zwei"));
        w.project.tracks[0].mute = true;
        w.project.tracks[1].solo = true;
        w.project.tracks[1].write = true;
        let mut h = window_harness(w);
        h.get_by_label("Mute aufheben (alle Spuren)").click();
        h.get_by_label("Solo aufheben (alle Spuren)").click();
        h.get_by_label("Automation schreiben aus (alle Kanäle)")
            .click();
        h.run_steps(2);
        let tracks = &h.state().project.tracks;
        assert!(tracks.iter().all(|t| !t.mute && !t.solo && !t.write));
        // R switches reading on for every channel, a second click switches it off.
        h.get_by_label("Automation lesen (alle Kanäle)").click();
        h.run_steps(2);
        assert!(h.state().project.tracks.iter().all(|t| t.read));
        h.get_by_label("Automation lesen (alle Kanäle)").click();
        h.run_steps(2);
        assert!(h.state().project.tracks.iter().all(|t| !t.read));
        // Each click is one undo step.
        h.state_mut().undo();
        assert!(h.state().project.tracks.iter().all(|t| t.read));
    }

    #[test]
    fn toolbar_tools_sit_in_the_middle_of_the_window() {
        let (w, _) = one_event_window();
        let mut h = window_harness(w);
        h.run_steps(3);
        let split = h.get_by_label("Trennen (3)").rect();
        let right = h.get_by_label("Rechte Zone (Alt+R)").rect();
        // The tool group starts left of the centre and the zone switches sit at the right edge.
        assert!(
            split.center().x > 400.0 && split.center().x < 700.0,
            "{split:?}"
        );
        assert!(right.right() > 1350.0, "{right:?}");
    }

    /// Bildschirmposition für `ticks` und `rows` Zeilenhöhen unter dem Lineal.
    fn lane_pos(h: &Harness<'_, ProjectWindow>, ticks: u64, rows: f32) -> egui::Pos2 {
        let w = h.state();
        let r = w.lanes_rect;
        let x =
            r.left() + ((ticks as f64 - w.view.scroll_ticks) / TPQ) as f32 * w.view.px_per_quarter;
        egui::pos2(x, r.top() + rows * w.view.row_h)
    }

    /// Klick mit Maustaste `button` und gehaltenen `mods` an `pos`.
    fn click_with(
        h: &mut Harness<'_, ProjectWindow>,
        pos: egui::Pos2,
        mods: egui::Modifiers,
        button: egui::PointerButton,
    ) {
        h.event(egui::Event::ModifiersChanged(mods));
        h.event(egui::Event::PointerMoved(pos));
        h.run_steps(1);
        for pressed in [true, false] {
            h.event(egui::Event::PointerButton {
                pos,
                button,
                pressed,
                modifiers: mods,
            });
        }
        h.run_steps(2);
        h.event(egui::Event::ModifiersChanged(egui::Modifiers::NONE));
        h.run_steps(1);
    }

    #[test]
    fn split_tool_with_alt_cuts_equal_pieces() {
        let (mut w, id) = one_event_window();
        w.tool = Tool::Split;
        let start = w.project.event(id).map(|e| e.start.0).unwrap_or(0);
        let len = w.project.event(id).map(|e| e.length).unwrap_or(0);
        let bar = w.bar_ticks();
        let mut h = window_harness(w);
        let p = lane_pos(&h, start + bar, 0.5);
        click_with(
            &mut h,
            p,
            egui::Modifiers::ALT,
            egui::PointerButton::Primary,
        );
        let events = &h.state().project.tracks[0].events;
        assert_eq!(events.len() as u64, len.div_ceil(bar));
        assert!(
            events
                .iter()
                .take(events.len() - 1)
                .all(|e| e.length == bar)
        );
    }

    #[test]
    fn erase_and_mute_tools_work_as_strokes() {
        let (mut w, id) = one_event_window();
        let bar = w.bar_ticks();
        let start = w.project.event(id).map(|e| e.start.0).unwrap_or(0);
        w.project.split_equal(id, start + bar);
        assert!(w.project.tracks[0].events.len() >= 3);
        w.project.tracks[0].events[1].muted = true;
        w.tool = Tool::Mute;
        let mut h = window_harness(w);
        // Mute stroke over the first three events: the first one decides (it becomes muted),
        // the others follow, so the already muted second one stays muted.
        let (a, b) = (
            lane_pos(&h, start + bar / 2, 0.5),
            lane_pos(&h, start + 5 * bar / 2, 0.5),
        );
        drag_with(&mut h, a, b, egui::Modifiers::NONE, egui::Modifiers::NONE);
        let muted: Vec<bool> = h.state().project.tracks[0]
            .events
            .iter()
            .map(|e| e.muted)
            .collect();
        assert_eq!(&muted[..3], &[true, true, true]);
        h.state_mut().undo();
        assert!(
            !h.state().project.tracks[0].events[0].muted,
            "one stroke is one undo step"
        );
        // Erase stroke over the first two events.
        h.state_mut().tool = Tool::Erase;
        let before = h.state().project.tracks[0].events.len();
        let b = lane_pos(&h, start + 3 * bar / 2, 0.5);
        drag_with(&mut h, a, b, egui::Modifiers::NONE, egui::Modifiers::NONE);
        assert_eq!(h.state().project.tracks[0].events.len(), before - 2);
    }

    #[test]
    fn zoom_tool_rectangle_fills_the_view() {
        let (mut w, _) = one_event_window();
        w.tool = Tool::Zoom;
        let bar = w.bar_ticks();
        let mut h = window_harness(w);
        let width = h.state().lanes_rect.width();
        let (a, b) = (lane_pos(&h, 2 * bar, 0.2), lane_pos(&h, 4 * bar, 0.8));
        drag_with(&mut h, a, b, egui::Modifiers::NONE, egui::Modifiers::NONE);
        let v = h.state().view;
        assert!(
            (v.scroll_ticks - (2 * bar) as f64).abs() < TPQ / 4.0,
            "{}",
            v.scroll_ticks
        );
        let quarters = (2 * bar) as f32 / TPQ as f32;
        assert!(
            (v.px_per_quarter - width / quarters).abs() < 2.0,
            "{}",
            v.px_per_quarter
        );
    }

    #[test]
    fn range_tool_shift_click_extends_the_range() {
        let (mut w, _) = one_event_window();
        w.tool = Tool::Range;
        let bar = w.bar_ticks();
        w.range = Some((0, 0, bar, 2 * bar));
        let mut h = window_harness(w);
        let p = lane_pos(&h, 4 * bar, 0.5);
        click_with(
            &mut h,
            p,
            egui::Modifiers::SHIFT,
            egui::PointerButton::Primary,
        );
        assert_eq!(h.state().range, Some((0, 0, bar, 4 * bar)));
        // Without Shift a click clears it.
        click_with(
            &mut h,
            p,
            egui::Modifiers::NONE,
            egui::PointerButton::Primary,
        );
        assert_eq!(h.state().range, None);
    }

    #[test]
    fn renders_tool_palette() {
        shot("werkzeugpalette", |w| {
            w.tool_palette = Some(egui::pos2(700.0, 300.0));
            w.tool = Tool::Split;
        });
    }

    #[test]
    fn right_click_opens_the_tool_palette() {
        let (w, _) = one_event_window();
        let mut h = window_harness(w);
        let p = lane_pos(&h, 0, 2.0);
        click_with(
            &mut h,
            p,
            egui::Modifiers::NONE,
            egui::PointerButton::Secondary,
        );
        h.get_by_label("Trennen (Palette)").click();
        h.run_steps(2);
        assert_eq!(h.state().tool, Tool::Split);
        assert!(
            h.query_by_label("Trennen (Palette)").is_none(),
            "palette closes"
        );
    }

    #[test]
    fn play_tool_auditions_scrubs_and_returns_the_cursor() {
        let (mut w, _) = one_event_window();
        w.tool = Tool::Play;
        let bar = w.bar_ticks();
        let mut h = window_harness(w);
        let at = |h: &Harness<'_, ProjectWindow>, t: u64| {
            let w = h.state();
            let r = w.lanes_rect;
            let x =
                r.left() + ((t as f64 - w.view.scroll_ticks) / TPQ) as f32 * w.view.px_per_quarter;
            egui::pos2(x, r.top() + w.view.row_h / 2.0)
        };
        let button = |pos, pressed| egui::Event::PointerButton {
            pos,
            button: egui::PointerButton::Primary,
            pressed,
            modifiers: egui::Modifiers::NONE,
        };
        let (a, b) = (at(&h, 2 * bar), at(&h, 3 * bar));
        h.event(egui::Event::PointerMoved(a));
        h.run_steps(1);
        h.event(button(a, true));
        h.run_steps(1);
        let near = |x: u64, y: u64| x.abs_diff(y) < TPQ as u64 / 4;
        assert!(
            near(h.state().cursor, 2 * bar),
            "press plays from the mouse"
        );
        for i in 1..=4 {
            h.event(egui::Event::PointerMoved(a + (b - a) * (i as f32 / 4.0)));
            h.run_steps(1);
        }
        assert!(near(h.state().cursor, 3 * bar), "dragging scrubs");
        h.event(button(b, false));
        h.run_steps(2);
        assert_eq!(h.state().cursor, 0, "release returns the cursor");
        assert!(h.state().audition.is_none());
    }

    #[test]
    fn select_tool_menu_sets_the_resize_mode_and_stretch_scales_notes() {
        let (mut w, _) = one_event_window();
        let row = w.project.add_track(TrackKind::Midi, String::from("Noten"));
        let q = TPQ as u64;
        let id = w.project.create_event(row, 0, 4 * q).expect("part created");
        if let Some(ev) = w.project.event_mut(id) {
            ev.notes = vec![studiocraft_project::Note {
                pitch: 60,
                velocity: 100,
                start: 2 * q as i64,
                length: q,
            }];
        }
        w.snap = false;
        let mut h = window_harness(w);
        // Right-click on the select tool opens its menu; pick "stretch".
        let button = h.get_by_label("Auswahl (1)").rect().center();
        h.event(egui::Event::PointerMoved(button));
        h.run_steps(1);
        for pressed in [true, false] {
            h.event(egui::Event::PointerButton {
                pos: button,
                button: egui::PointerButton::Secondary,
                pressed,
                modifiers: egui::Modifiers::NONE,
            });
        }
        h.run_steps(2);
        h.get_by_label("Größe ändern, Inhalt strecken").click();
        h.run_steps(2);
        assert_eq!(h.state().resize_mode, tools::ResizeMode::Stretch);
        // Drag the right edge (lower half) from 4 to 8 quarters: the note doubles too.
        let edge = |h: &Harness<'_, ProjectWindow>, t: u64| {
            let w = h.state();
            let r = w.lanes_rect;
            let x =
                r.left() + ((t as f64 - w.view.scroll_ticks) / TPQ) as f32 * w.view.px_per_quarter;
            egui::pos2(x, r.top() + (row as f32 + 0.8) * w.view.row_h)
        };
        let (a, b) = (
            edge(&h, 4 * q) - egui::vec2(2.0, 0.0),
            edge(&h, 8 * q) - egui::vec2(2.0, 0.0),
        );
        drag_with(&mut h, a, b, egui::Modifiers::NONE, egui::Modifiers::NONE);
        let ev = h.state().project.event(id).cloned().expect("part exists");
        assert!(ev.length.abs_diff(8 * q) < q / 4, "{}", ev.length);
        assert!(
            ev.notes[0].start.abs_diff(4 * q as i64) < q / 4,
            "{:?}",
            ev.notes
        );
    }

    #[test]
    fn saved_layout_sets_zone_sizes_and_does_not_mark_the_project_modified() {
        let (w, _) = one_event_window();
        let mut p = w.project.clone();
        p.layout.right = true;
        p.layout.right_width = 400.0;
        p.layout.lower = false;
        // Opening a project goes through the same path as `new`: apply the layout, then
        // remember the state as saved.
        let w = ProjectWindow::new(p, None, String::new());
        let mut h = window_harness(w);
        h.run_steps(3);
        assert!(h.state().show_right && !h.state().show_lower);
        let right = h.get_by_label("Start").rect();
        let window_w = 1400.0;
        assert!(
            right.left() > window_w - 400.0 - 20.0,
            "right zone uses the saved width: {right:?}"
        );
        assert!(
            !h.state().is_modified(),
            "opening does not count as a change: {:?} vs {:?}",
            h.state().project.layout,
            h.state().saved.as_ref().map(|p| &p.layout)
        );
        // Hiding a zone is a change that gets saved with the project.
        h.state_mut().show_right = false;
        h.run_steps(2);
        assert!(!h.state().project.layout.right);
        assert!(h.state().is_modified());
        let json = studiocraft_project::file::to_json(&h.state().project);
        let back = studiocraft_project::file::from_json(&json).unwrap();
        assert_eq!(back.layout, h.state().project.layout);
    }

    #[test]
    fn dragging_a_zone_divider_saves_its_size() {
        let (mut w, _) = one_event_window();
        w.show_right = true;
        let mut h = window_harness(w);
        h.run_steps(2);
        // The divider sits on the left edge of the right zone (its tab row starts there).
        let edge = h.get_by_label("Start").rect().left() - 8.0;
        let y = 400.0;
        let (a, b) = (egui::pos2(edge, y), egui::pos2(edge - 100.0, y));
        drag_with(&mut h, a, b, egui::Modifiers::NONE, egui::Modifiers::NONE);
        let width = h.state().project.layout.right_width;
        assert!(width > 300.0, "divider drag widens the right zone: {width}");
    }

    #[test]
    fn named_layouts_are_saved_and_recalled_by_shortcut() {
        let (mut w, _) = one_event_window();
        w.show_right = true;
        w.show_lower = false;
        let mut h = window_harness(w);
        h.run_steps(2);
        h.state_mut().save_layout("Mischen");
        assert_eq!(h.state().project.layouts.len(), 1);
        // Change the zones, then recall slot 1 with Ctrl+Alt+1.
        h.state_mut().show_right = false;
        h.state_mut().show_lower = true;
        h.run_steps(2);
        h.key_press_modifiers(egui::Modifiers::COMMAND | egui::Modifiers::ALT, Key::Num1);
        h.run_steps(2);
        assert!(h.state().show_right && !h.state().show_lower);
        // An empty slot only reports it.
        h.key_press_modifiers(egui::Modifiers::COMMAND | egui::Modifiers::ALT, Key::Num5);
        h.run_steps(1);
        assert_eq!(
            h.state().message.as_deref(),
            Some("Layout 5 ist nicht belegt")
        );
        // Same name replaces; at most nine layouts.
        h.state_mut().save_layout("Mischen");
        for k in 0..12 {
            h.state_mut().save_layout(&format!("L{k}"));
        }
        assert_eq!(
            h.state().project.layouts.len(),
            studiocraft_project::layout::MAX_LAYOUTS
        );
        // Layouts survive saving and loading.
        let json = studiocraft_project::file::to_json(&h.state().project);
        let back = studiocraft_project::file::from_json(&json).unwrap();
        assert_eq!(back.layouts, h.state().project.layouts);
    }

    #[test]
    fn alt_l_toggles_the_left_zone_and_alt_i_the_inspector_column() {
        let (w, _) = one_event_window();
        let mut h = window_harness(w);
        h.key_press_modifiers(egui::Modifiers::ALT, Key::I);
        h.run_steps(2);
        assert!(h.state().show_inspector && !h.state().show_inspector_column);
        assert!(
            h.query_by_label("Name").is_none(),
            "inspector column hidden"
        );
        h.key_press_modifiers(egui::Modifiers::ALT, Key::L);
        h.run_steps(2);
        assert!(!h.state().show_inspector);
        // Alt+I on a hidden left zone opens it with the inspector.
        h.key_press_modifiers(egui::Modifiers::ALT, Key::I);
        h.run_steps(2);
        assert!(h.state().show_inspector && h.state().show_inspector_column);
    }

    #[test]
    fn undo_leaves_the_window_layout_alone() {
        let (mut w, id) = one_event_window();
        let before = w.project.clone();
        w.project.delete(id);
        w.checkpoint(before);
        w.project.layout.right = true;
        w.show_right = true;
        w.undo();
        assert!(w.project.event(id).is_some());
        assert!(w.project.layout.right && w.show_right);
    }

    #[test]
    fn f3_opens_the_mixer_in_its_own_window() {
        let (mut w, _) = one_event_window();
        w.show_lower = true;
        w.lower = LowerZone::Mixer;
        let mut h = window_harness(w);
        h.key_press(Key::F3);
        h.run_steps(3);
        assert!(h.state().windows.mixer);
        // The lower zone only points to the window; the mixer is drawn once, in the window.
        h.get_by_label("Läuft im eigenen Fenster.");
        assert!(h.query_by_label("In die untere Zone").is_some());
        h.get_by_label("In die untere Zone").click();
        h.run_steps(3);
        assert!(!h.state().windows.mixer);
        assert!(h.state().show_lower && h.state().lower == LowerZone::Mixer);
        assert!(h.query_by_label("Läuft im eigenen Fenster.").is_none());
    }

    #[test]
    fn ctrl_e_opens_the_editor_of_the_selected_event() {
        let (mut w, id) = one_event_window();
        w.show_lower = false;
        let mut h = window_harness(w);
        // Nothing selected: only a message.
        h.key_press_modifiers(egui::Modifiers::COMMAND, Key::E);
        h.run_steps(2);
        assert!(h.state().windows.editor.is_none());
        assert!(h.state().message.is_some());
        // An audio event opens the sample editor window; the lower zone stays as it was.
        h.state_mut().selection = vec![id];
        h.key_press_modifiers(egui::Modifiers::COMMAND, Key::E);
        h.run_steps(3);
        assert_eq!(
            h.state().windows.editor,
            Some(windows::EditorWindow::Sample)
        );
        assert_eq!(h.state().sample_editor.event, Some(id));
        assert!(!h.state().show_lower);
    }

    #[test]
    fn renders_mixer_window() {
        shot("mixer-fenster", |w| {
            w.toggle_mixer_window();
        });
    }

    #[test]
    fn dragging_track_headers_moves_tracks_into_and_out_of_folders() {
        let (mut w, _) = one_event_window();
        let folder = w
            .project
            .add_track(TrackKind::Folder, String::from("Ordner"));
        let second = w.project.add_track(TrackKind::Audio, String::from("Zwei"));
        let mut h = window_harness(w);
        // Header point of a track, `frac` of a row height below its top, right of the buttons.
        let head = |h: &Harness<'_, ProjectWindow>, track: usize, frac: f32| {
            let w = h.state();
            let shown: Vec<usize> = w.project.display_order().iter().map(|(t, _)| *t).collect();
            let d = shown.iter().position(|&t| t == track).unwrap();
            let r = w.lanes_rect;
            egui::pos2(r.left() - 70.0, r.top() + (d as f32 + frac) * w.view.row_h)
        };
        // "Zwei" onto the middle of the folder header: into the folder.
        let (a, b) = (head(&h, second, 0.75), head(&h, folder, 0.5));
        drag_with(&mut h, a, b, egui::Modifiers::NONE, egui::Modifiers::NONE);
        assert_eq!(h.state().project.parent_of(second), Some(folder));
        // "Zwei" onto the upper half of the first track: out of the folder, at the top.
        let (a, b) = (head(&h, second, 0.75), head(&h, 0, 0.1));
        drag_with(&mut h, a, b, egui::Modifiers::NONE, egui::Modifiers::NONE);
        assert_eq!(h.state().project.parent_of(second), None);
        let shown: Vec<usize> = h
            .state()
            .project
            .display_order()
            .iter()
            .map(|(t, _)| *t)
            .collect();
        assert_eq!(shown[0], second);
        h.state_mut().undo();
        assert_eq!(
            h.state().project.parent_of(second),
            Some(folder),
            "one move, one undo step"
        );
    }

    #[test]
    fn dragging_a_folder_row_moves_its_events() {
        let (mut w, id) = one_event_window();
        let folder = w
            .project
            .add_track(TrackKind::Folder, String::from("Ordner"));
        w.project.set_parent(0, Some(folder));
        let start = w.project.event(id).map(|e| e.start.0).unwrap_or(0);
        let bar = w.bar_ticks();
        let mut h = window_harness(w);
        let row_of = |h: &Harness<'_, ProjectWindow>, t: usize| {
            h.state()
                .project
                .display_order()
                .iter()
                .position(|(x, _)| *x == t)
                .unwrap() as f32
        };
        let y = row_of(&h, folder) + 0.5;
        let (a, b) = (lane_pos(&h, start, y), lane_pos(&h, start + bar, y));
        drag_with(&mut h, a, b, egui::Modifiers::NONE, egui::Modifiers::NONE);
        let now = h.state().project.event(id).map(|e| e.start.0).unwrap_or(0);
        assert_eq!(now, start + bar);
    }

    #[test]
    fn midi_track_inspector_sets_channel_and_program() {
        let mut p = Project::default();
        let t = p.add_track(TrackKind::Midi, "Gerät");
        p.tracks[t].midi_port = Some("Synth A".into());
        let mut w = ProjectWindow::new(p, None, String::new());
        w.selected_track = Some(t);
        // No real ports in the test: the saved one stays selectable as missing.
        w.midi_out_ports = Some(Vec::new());
        let mut h = window_harness(w);
        h.run_steps(2);
        h.get_by_label("MIDI-Ausgang");
        h.get_by_label("Programm").click();
        h.run_steps(2);
        assert_eq!(h.state().project.tracks[t].program, Some(0));
        h.state_mut().undo();
        assert_eq!(h.state().project.tracks[t].program, None, "one undo step");
        // Saved values are sanitized on load.
        let mut q = h.state().project.clone();
        q.tracks[t].midi_channel = 99;
        q.tracks[t].program = Some(200);
        q.sanitize();
        assert_eq!(
            (q.tracks[t].midi_channel, q.tracks[t].program),
            (16, Some(127))
        );
    }

    #[test]
    fn range_tool_scales_automation_inside_the_selection() {
        use studiocraft_project::AutoParam;
        let (mut w, _) = one_event_window();
        let bar = w.bar_ticks();
        {
            let lane = w.project.tracks[0].automation_lane_mut(AutoParam::Volume);
            lane.set_point(0, -20.0);
            lane.set_point(8 * bar, -20.0);
            lane.visible = true;
        }
        w.tool = Tool::Range;
        w.range = Some((0, 0, 2 * bar, 3 * bar));
        let mut h = window_harness(w);
        h.run_steps(2);
        // The automation lane is the second row of track 0.
        let a = lane_pos(&h, 2 * bar + bar / 2, 1.5);
        let b = a - egui::vec2(0.0, 15.0);
        drag_with(&mut h, a, b, egui::Modifiers::NONE, egui::Modifiers::NONE);
        let lane = h.state().project.tracks[0]
            .automation_lane(AutoParam::Volume)
            .unwrap()
            .clone();
        let inside = lane.value_at(2 * bar + bar / 2).unwrap();
        assert!(inside > -20.0, "dragging up raises the values: {inside}");
        assert_eq!(
            lane.value_at(bar),
            Some(-20.0),
            "before the range unchanged"
        );
        assert_eq!(
            lane.value_at(6 * bar),
            Some(-20.0),
            "after the range unchanged"
        );
        h.state_mut().undo();
        let lane = h.state().project.tracks[0]
            .automation_lane(AutoParam::Volume)
            .unwrap();
        assert_eq!(lane.points.len(), 2, "one gesture, one undo step");
    }

    #[test]
    fn mixer_and_inspector_edits_are_undoable_one_gesture_per_step() {
        let (mut w, _) = one_event_window();
        let name = w.project.tracks[0].name.clone();
        w.show_lower = true;
        w.lower = LowerZone::Mixer;
        w.show_inspector = false;
        let mut h = window_harness(w);
        h.run_steps(3);
        // A whole fader drag is one step.
        let fader = h.get_by_label(&format!("Lautstärke {name}")).rect();
        let (a, b) = (fader.center(), fader.center() + egui::vec2(0.0, 60.0));
        drag_with(&mut h, a, b, egui::Modifiers::NONE, egui::Modifiers::NONE);
        h.run_steps(2);
        let db = h.state().project.tracks[0].volume_db;
        assert!(db < -1.0, "fader moved: {db}");
        assert_eq!(h.state().undo.len(), 1, "one drag, one step");
        h.state_mut().undo();
        assert_eq!(h.state().project.tracks[0].volume_db, 0.0);
        // Mute in the mixer.
        h.get_by_label("M").click();
        h.run_steps(3);
        assert!(h.state().project.tracks[0].mute);
        assert_eq!(h.state().undo.len(), 1);
        h.state_mut().undo();
        assert!(!h.state().project.tracks[0].mute);
        // Typing a name in the inspector: one step after the field loses focus.
        h.state_mut().show_inspector = true;
        h.state_mut().selected_track = Some(0);
        h.run_steps(3);
        h.get_by_label("Name").focus();
        h.run_steps(1);
        h.key_press_modifiers(egui::Modifiers::COMMAND, Key::A);
        h.get_by_label("Name").type_text("Gesang");
        h.run_steps(1);
        h.key_press(Key::Enter);
        h.run_steps(3);
        assert_eq!(h.state().project.tracks[0].name, "Gesang");
        let steps = h.state().undo.len();
        h.state_mut().undo();
        assert_eq!(h.state().project.tracks[0].name, name);
        assert_eq!(h.state().undo.len(), steps - 1);
    }

    #[test]
    fn renders_mixer_solo_defeat_and_clip() {
        shot("mixer-solo-clip", |w| {
            w.show_lower = true;
            w.lower = LowerZone::Mixer;
            w.project.tracks[0].solo = true;
            w.project.tracks[1].solo_defeat = true;
            w.project.tracks[2].record = true;
            let n = w.project.tracks.len();
            w.meter_hold = vec![[(0.5, 0.0), (0.7, 0.0)]; n + 1];
            w.clipped = vec![false; n + 1];
            w.clipped[0] = true;
        });
    }

    #[test]
    fn mixer_solo_modes_clip_reset_and_strip_buttons() {
        let (mut w, _) = one_event_window();
        w.project.add_track(TrackKind::Audio, String::from("Zwei"));
        w.project.tracks[0].solo = true;
        w.show_lower = true;
        w.lower = LowerZone::Mixer;
        w.show_inspector = false;
        w.meter_hold = vec![[(0.0, 0.0); 2]; 3];
        w.clipped = vec![true, false, false];
        let mut h = window_harness(w);
        h.run_steps(3);
        let solo = |h: &Harness<'_, ProjectWindow>, k: usize| {
            let mut s: Vec<egui::Rect> = h.query_all_by_label("S").map(|n| n.rect()).collect();
            s.sort_by(|a, b| a.left().total_cmp(&b.left()));
            s[k].center()
        };
        // Alt+click on the second strip's S: exclusive solo.
        let p = solo(&h, 1);
        click_with(
            &mut h,
            p,
            egui::Modifiers::ALT,
            egui::PointerButton::Primary,
        );
        let tr = &h.state().project.tracks;
        assert!(!tr[0].solo && tr[1].solo);
        // Ctrl+click on the first strip's S: solo defeat, it stays audible.
        let p = solo(&h, 0);
        click_with(
            &mut h,
            p,
            egui::Modifiers::COMMAND,
            egui::PointerButton::Primary,
        );
        assert!(h.state().project.tracks[0].solo_defeat);
        assert!(h.state().project.audible(0));
        // The clip indicator resets on click.
        let mut clips: Vec<egui::Rect> = h
            .query_all_by_label("Übersteuerung zurücksetzen")
            .map(|n| n.rect())
            .collect();
        clips.sort_by(|a, b| a.left().total_cmp(&b.left()));
        click_with(
            &mut h,
            clips[0].center(),
            egui::Modifiers::NONE,
            egui::PointerButton::Primary,
        );
        assert!(!h.state().clipped[0]);
        // Record and monitor on the strip.
        let mut rec: Vec<egui::Rect> = h
            .query_all_by_label("Aufnahme bereit")
            .map(|n| n.rect())
            .collect();
        rec.sort_by(|a, b| a.left().total_cmp(&b.left()));
        click_with(
            &mut h,
            rec[0].center(),
            egui::Modifiers::NONE,
            egui::PointerButton::Primary,
        );
        assert!(h.state().project.tracks[0].record);
    }

    #[test]
    fn pan_mode_is_chosen_in_the_inspector_saved_and_undoable() {
        use studiocraft_project::PanMode;
        let (mut w, _) = one_event_window();
        w.selected_track = Some(0);
        w.show_lower = false;
        let mut h = window_harness(w);
        h.run_steps(2);
        h.get_by_label("Pan-Modus").click();
        h.run_steps(2);
        h.get_by_label("Stereo kombiniert").click();
        h.run_steps(3);
        assert_eq!(h.state().project.tracks[0].pan_mode, PanMode::Combined);
        let json = studiocraft_project::file::to_json(&h.state().project);
        let back = studiocraft_project::file::from_json(&json).unwrap();
        assert_eq!(back.tracks[0].pan_mode, PanMode::Combined);
        h.state_mut().undo();
        assert_eq!(h.state().project.tracks[0].pan_mode, PanMode::Balance);
    }

    #[test]
    fn info_line_edits_the_selected_event_by_keyboard() {
        let (mut w, id) = one_event_window();
        w.selection = vec![id];
        let mut h = window_harness(w);
        h.get_by_label("Länge").focus();
        h.run_steps(1);
        h.key_press_modifiers(egui::Modifiers::COMMAND, Key::A);
        h.run_steps(1);
        h.get_by_label("Länge").type_text("0.2");
        h.run_steps(1);
        h.key_press(Key::Enter);
        h.run_steps(2);
        let beat = h.state().beat_ticks();
        assert_eq!(
            h.state().project.event(id).map(|e| e.length),
            Some(2 * beat)
        );
        // Ziffern im Textfeld wählen kein Werkzeug.
        assert_eq!(h.state().tool, Tool::Select);
    }

    #[test]
    fn transport_symbols_wind_and_toggle_punch_and_click() {
        let (w, _) = one_event_window();
        let bar = w.bar_ticks();
        let mut h = window_harness(w);
        h.get_by_label("Vorspulen um einen Takt").click();
        h.run_steps(1);
        h.get_by_label("Vorspulen um einen Takt").click();
        h.run_steps(1);
        h.get_by_label("Zurückspulen um einen Takt").click();
        h.run_steps(1);
        assert_eq!(h.state().cursor, bar);
        h.get_by_label("Punch-In (I)").click();
        h.get_by_label("Punch-Out (O)").click();
        h.get_by_label("Metronom (C)").click();
        h.run_steps(1);
        let w = h.state();
        assert!(w.punch_in && w.punch_out && w.metronome);
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
    fn plugins_stay_alive_without_engine() {
        // Ohne Audio-Engine darf die Kette nicht verworfen werden, solange Griffe darauf zeigen.
        let mut w = ProjectWindow::new(Project::default(), None, String::new());
        let Some(fx) = test_effect(&mut w) else {
            return;
        };
        w.add_audio_track();
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
        let h = w.plugins.sent[0].handles[0].as_ref().unwrap();
        assert!(h.has_gui(), "Griff benutzbar");
        assert!(h.save_state().is_some(), "Instanz lebt noch");
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
    fn inspector_groups_input_and_output_under_routing() {
        let (mut w, _) = one_event_window();
        w.selected_track = Some(0);
        let mut h = window_harness(w);
        // Routing is open by default and holds input and output of the audio track.
        h.get_by_label("Routing");
        h.get_by_label("Eingang");
        h.get_by_label("Ausgang");
        // Clicking the bar closes the section; clicking again opens it.
        h.get_by_label("Routing").click();
        h.run_steps(20);
        assert!(h.query_by_label("Ausgang").is_none());
        h.get_by_label("Routing").click();
        h.run_steps(20);
        h.get_by_label("Ausgang");
        // Sections without content for the track type are left out.
        assert!(h.query_by_label("Instrument").is_none());
    }

    #[test]
    fn left_zone_shows_the_channel_strip_of_the_selected_track() {
        let (mut w, _) = one_event_window();
        let name = w.project.tracks[0].name.clone();
        w.project.add_track(TrackKind::Midi, String::from("Noten"));
        w.selected_track = Some(0);
        w.show_lower = false;
        let mut h = window_harness(w);
        let label = format!("Lautstärke {name}");
        assert_eq!(h.query_all_by_label(&label).count(), 1);
        // Drag the column's fader down: the track gets quieter.
        let rect = h.get_by_label(&label).rect();
        let (from, to) = (rect.center(), rect.center() + egui::vec2(0.0, 60.0));
        h.event(egui::Event::PointerMoved(from));
        h.run_steps(1);
        h.event(egui::Event::PointerButton {
            pos: from,
            button: egui::PointerButton::Primary,
            pressed: true,
            modifiers: egui::Modifiers::NONE,
        });
        h.run_steps(1);
        for i in 1..=4 {
            h.event(egui::Event::PointerMoved(
                from + (to - from) * (i as f32 / 4.0),
            ));
            h.run_steps(1);
        }
        h.event(egui::Event::PointerButton {
            pos: to,
            button: egui::PointerButton::Primary,
            pressed: false,
            modifiers: egui::Modifiers::NONE,
        });
        h.run_steps(2);
        let db = h.state().project.tracks[0].volume_db;
        assert!(db < -1.0, "fader moved down, got {db}");
        // A MIDI track has no channel: the column says so instead of showing a fader.
        h.state_mut().selected_track = Some(1);
        h.run_steps(2);
        assert_eq!(h.query_all_by_label(&label).count(), 0);
        h.get_by_label("Kein Kanal");
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
            recording: true,
            running: true,
            clock: pos,
            ctrl: None,
        }
    }

    fn played(
        pos: u64,
        clock: u64,
        key: u8,
        on: bool,
        running: bool,
    ) -> studiocraft_engine::midi::RecordedMidi {
        studiocraft_engine::midi::RecordedMidi {
            pos,
            key,
            velocity: 90,
            on,
            recording: false,
            running,
            clock,
            ctrl: None,
        }
    }

    #[test]
    fn retrospective_record_while_stopped_starts_at_the_cursor() {
        let mut w = ProjectWindow::new(Project::default(), None, String::new());
        let t = w.project.add_track(TrackKind::Instrument, "Synth");
        w.selected_track = Some(t);
        let bar = w.bar_ticks();
        w.cursor = 2 * bar;
        // While stopped the position stands still at 0; the clock runs: 0.5 s apart.
        w.retro.extend([
            played(0, 1_000, 60, true, false),
            played(0, 13_000, 60, false, false),
            played(0, 25_000, 64, true, false),
            played(0, 37_000, 64, false, false),
        ]);
        w.retrospective_record();
        let ev = w.project.tracks[t].events.last().expect("a part");
        let notes: Vec<(u64, u8)> = ev
            .notes
            .iter()
            .map(|n| ((ev.start.0 as i64 + n.start) as u64, n.pitch))
            .collect();
        // At 120 BPM, 24 000 frames are one quarter (960 ticks).
        assert_eq!(notes, [(2 * bar, 60), (2 * bar + 960, 64)]);
        assert!(w.retro.is_empty(), "taken");
        assert!(!w.undo.is_empty(), "one undo step");
    }

    #[test]
    fn retrospective_record_during_playback_keeps_positions_of_the_last_pass() {
        let mut w = ProjectWindow::new(Project::default(), None, String::new());
        let t = w.project.add_track(TrackKind::Instrument, "Synth");
        w.selected_track = Some(t);
        // Two cycle passes while playing: the jump back starts the second one.
        w.retro.extend([
            played(48_000, 0, 50, true, true),
            played(60_000, 0, 50, false, true),
            played(24_000, 0, 62, true, true),
            played(36_000, 0, 62, false, true),
        ]);
        w.retrospective_record();
        let ev = w.project.tracks[t].events.last().expect("a part");
        let pitches: Vec<u8> = ev.notes.iter().map(|n| n.pitch).collect();
        assert_eq!(pitches, [62], "only the last pass");
        assert_eq!((ev.start.0 as i64 + ev.notes[0].start) as u64, 960);
        // Nothing played: a message, no part.
        w.retrospective_record();
        assert_eq!(w.project.tracks[t].events.len(), 1);
        assert!(
            w.message
                .as_deref()
                .is_some_and(|m| m.contains("nichts gespielt"))
        );
    }

    #[test]
    fn the_midi_record_menu_sets_the_modes() {
        let (w, _) = one_event_window();
        let mut h = window_harness(w);
        h.get_by_label("Transport").click();
        h.run_steps(2);
        h.get_by_label("MIDI-Aufnahme").click();
        h.run_steps(2);
        h.get_by_label("Mischen").click();
        h.run_steps(2);
        assert_eq!(
            h.state().project.record.midi,
            studiocraft_project::MidiRecordMode::Merge
        );
        if h.query_by_label("Mix").is_none() {
            h.get_by_label("Transport").click();
            h.run_steps(2);
            h.get_by_label("MIDI-Aufnahme").click();
            h.run_steps(2);
        }
        h.get_by_label("Mix").click();
        h.run_steps(2);
        assert_eq!(
            h.state().project.record.cycle_midi,
            studiocraft_project::CycleRecordMode::Mix
        );
    }

    #[test]
    fn recorded_controllers_land_in_the_part_without_the_sustain_pedal() {
        let mut p = Project::default();
        let t = p.add_track(TrackKind::Instrument, "Synth");
        let mut w = ProjectWindow::new(p, None, String::new());
        let ctrl = |pos: u64, data: [u8; 3]| studiocraft_engine::midi::RecordedMidi {
            pos,
            key: 0,
            velocity: 0,
            on: false,
            recording: true,
            running: true,
            clock: pos,
            ctrl: Some(data),
        };
        // Half a second at 48 kHz = one quarter at 120 BPM.
        let take = [
            rec(0, 60, true),
            ctrl(24_000, [0xb0, 1, 90]),
            ctrl(48_000, [0xe0, 0, 64]),
            rec(96_000, 60, false),
        ];
        w.place_midi_take(t, &take, 0, 96_000);
        let ev = &w.project.tracks[t].events[0];
        assert_eq!(ev.notes.len(), 1);
        let c: Vec<(studiocraft_project::CtrlKind, i64, u16)> = ev
            .controllers
            .iter()
            .map(|c| (c.kind, c.start, c.value))
            .collect();
        assert_eq!(
            c,
            [
                (studiocraft_project::CtrlKind::Cc(1), 960, 90),
                (studiocraft_project::CtrlKind::PitchBend, 1920, 8192),
            ]
        );
        // Controllers alone also make a part.
        w.place_midi_take(t, &[ctrl(0, [0xd0, 50, 0])], 0, 48_000);
        assert_eq!(w.project.tracks[t].events.len(), 2);
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
    fn tempo_reaches_engine_and_is_undoable() {
        let (_core, ctl) = studiocraft_engine::engine(48_000, TempoMap::default());
        let mut w = ProjectWindow::new(Project::default(), Some(ctl), String::new());
        let bar = w.bar_ticks();
        let id = w
            .import_clip("Ton", None, tone_clip(2.0, 220.0), Some((usize::MAX, bar)))
            .unwrap();
        w.clear_history();
        let mut h = window_harness(w);
        // Wie eine Eingabe in der Transportleiste: ändern, dann loslassen.
        let st = h.state_mut();
        st.tempo_before = Some(st.project.clone());
        st.project.set_tempo(90.0);
        h.run_steps(3);
        let st = h.state_mut();
        assert_eq!(
            st.transport.as_ref().unwrap().tempo.bpm,
            90.0,
            "Engine kennt das neue Tempo"
        );
        let e = st.project.event(id).unwrap();
        assert_eq!(
            st.project.tempo.seconds(e.start),
            2.0,
            "Audio bleibt bei 2 s"
        );
        st.undo();
        assert_eq!(st.project.tempo.bpm, 120.0);
        h.run_steps(2);
        assert_eq!(
            h.state().transport.as_ref().unwrap().tempo.bpm,
            120.0,
            "Undo erreicht die Engine"
        );
    }

    #[test]
    fn renders_sends_and_fx_channel() {
        shot("sends", |w| {
            w.project = Project::default();
            w.import_clip("Gesang", None, tone_clip(8.0, 330.0), None)
                .unwrap();
            w.add_bus_track(TrackKind::Fx);
            w.add_bus_track(TrackKind::Group);
            let (fx, g) = (w.project.tracks[1].id, w.project.tracks[2].id);
            w.project.tracks[1].name = "Hall".into();
            w.project.tracks[0]
                .sends
                .push(studiocraft_project::SendSlot {
                    target: fx,
                    level_db: -8.0,
                    pre_fader: false,
                    on: true,
                });
            w.project.tracks[0].output = Some(g);
            if let Some(verb) = w
                .plugin_list()
                .iter()
                .find(|p| p.name == "ZamVerb")
                .cloned()
            {
                w.project.tracks[1]
                    .inserts
                    .push(studiocraft_project::InsertSlot {
                        plugin_id: verb.id,
                        name: verb.name,
                        bypass: false,
                        params: Vec::new(),
                        state: None,
                    });
            }
            w.selected_track = Some(0);
        });
    }

    #[test]
    fn output_menu_offers_no_loops() {
        let mut w = ProjectWindow::new(Project::default(), None, String::new());
        w.add_bus_track(TrackKind::Group);
        w.add_bus_track(TrackKind::Group);
        let (a, b) = (w.project.tracks[0].id, w.project.tracks[1].id);
        w.project.tracks[1].output = Some(a);
        assert!(!w.project.can_route(a, b), "A → B wäre eine Schleife");
        w.undo();
        assert_eq!(w.project.tracks.len(), 1);
    }

    #[test]
    fn range_tool_delete_and_locators() {
        let mut w = ProjectWindow::new(Project::default(), None, String::new());
        let bar = w.bar_ticks();
        w.import_clip("A", None, tone_clip(8.0, 110.0), None)
            .unwrap(); // Takt 1–5
        w.import_clip("B", None, tone_clip(8.0, 220.0), None)
            .unwrap();
        w.clear_history();
        let mut h = window_harness(w);
        h.state_mut().tool = Tool::Range;
        // Von Takt 2 auf Spur A bis Takt 3 auf Spur B ziehen.
        drag(&mut h, (bar, 0.5), (2 * bar, 1.5), 0.0);
        assert_eq!(h.state().range, Some((0, 1, bar, 2 * bar)));
        h.key_press(Key::P);
        h.run_steps(1);
        assert_eq!(
            (h.state().left_locator, h.state().right_locator),
            (bar, 2 * bar)
        );
        h.key_press(Key::Delete);
        h.run_steps(1);
        let w = h.state_mut();
        for (t, track) in w.project.tracks.iter().enumerate() {
            let spans: Vec<(u64, u64)> =
                track.events.iter().map(|e| (e.start.0, e.end())).collect();
            assert_eq!(spans, [(0, bar), (2 * bar, 4 * bar)], "Spur {t}");
        }
        assert_eq!(w.range, None);
        w.undo();
        assert!(w.project.tracks.iter().all(|t| t.events.len() == 1));
    }

    #[test]
    fn glue_tool_joins_parts() {
        let mut w = ProjectWindow::new(Project::default(), None, String::new());
        w.add_instrument_track(None);
        let q = TICKS_PER_QUARTER;
        w.project.create_event(0, 0, 2 * q).unwrap();
        w.project.create_event(0, 4 * q, 2 * q).unwrap();
        w.clear_history();
        let mut h = window_harness(w);
        h.state_mut().tool = Tool::Glue;
        let pos = {
            let w = h.state();
            let r = w.lanes_rect;
            egui::pos2(
                r.left() + w.view.px_per_quarter * 0.5,
                r.top() + w.view.row_h * 0.6,
            )
        };
        h.event(egui::Event::PointerMoved(pos));
        h.run_steps(1);
        let button = |pressed| egui::Event::PointerButton {
            pos,
            button: egui::PointerButton::Primary,
            pressed,
            modifiers: egui::Modifiers::NONE,
        };
        h.event(button(true));
        h.event(button(false));
        h.run_steps(2);
        let evs = &h.state().project.tracks[0].events;
        assert_eq!(evs.len(), 1);
        assert_eq!(evs[0].length, 6 * q);
    }

    #[test]
    fn renders_light_theme() {
        shot("hell", |w| {
            w.theme = egui::ThemePreference::Light;
            w.project = Project::default();
            w.import_clip("Gesang", None, tone_clip(8.0, 330.0), None)
                .unwrap();
            w.add_instrument_track(None);
            melody(w, 1, 4 * TICKS_PER_QUARTER);
            w.project.tracks[0].solo = true;
            w.project.tracks[1].mute = true;
            w.cycle = true;
            w.cursor = 3 * TICKS_PER_QUARTER;
            w.meter_levels = vec![[0.8, 0.7], [0.0, 0.0]];
        });
    }

    #[test]
    fn renders_right_zone_with_pool() {
        shot("rechte-zone", |w| {
            w.project = Project::default();
            w.import_clip("Bass", None, tone_clip(8.0, 55.0), None)
                .unwrap();
            w.import_clip("Gitarre", None, tone_clip(6.0, 196.0), None)
                .unwrap();
            w.show_right = true;
            w.right.tab = rightzone::Tab::Media;
        });
    }

    #[test]
    fn renders_right_zone_start_page() {
        shot("rechte-zone-start", |w| {
            w.show_right = true;
        });
    }

    #[test]
    fn start_page_tiles_open_the_filtered_plugin_list() {
        let (mut w, _) = one_event_window();
        w.show_right = true;
        let mut h = window_harness(w);
        assert_eq!(h.state().right.tab, rightzone::Tab::Home);
        h.get_by_label("Effekte").click();
        h.run_steps(2);
        assert_eq!(h.state().right.tab, rightzone::Tab::Plugins);
        assert_eq!(h.state().right.plugin_kind, rightzone::PluginKind::Effects);
        h.get_by_label("Start").click();
        h.run_steps(2);
        h.get_by_label("Instrumente").click();
        h.run_steps(2);
        assert_eq!(h.state().right.tab, rightzone::Tab::Plugins);
        assert_eq!(
            h.state().right.plugin_kind,
            rightzone::PluginKind::Instruments
        );
    }

    #[test]
    fn default_shortcuts_reach_their_commands() {
        let (w, id) = one_event_window();
        let mut h = window_harness(w);
        h.key_press_modifiers(egui::Modifiers::COMMAND, Key::A);
        h.run_steps(1);
        assert_eq!(h.state().selection, vec![id]);
        let track = h.state().selected_track.unwrap_or(0);
        h.key_press(Key::M);
        h.run_steps(1);
        assert!(h.state().project.tracks[track].mute);
        // Alt+X teilt am Cursor; X allein (Crossfade) teilt nicht.
        h.state_mut().cursor = h.state().project.event(id).map_or(0, |e| e.start.0 + 100);
        h.key_press_modifiers(egui::Modifiers::ALT, Key::X);
        h.run_steps(1);
        assert_eq!(h.state().project.tracks[track].events.len(), 2);
        h.key_press(Key::F2);
        h.run_steps(1);
        assert!(!h.state().show_transport);
    }

    #[test]
    fn q_quantizes_and_the_panel_opens_from_the_toolbar() {
        let (mut w, id) = one_event_window();
        let start = w.project.event(id).map_or(0, |e| e.start.0);
        w.project.move_event(id, 0, start + 100, false);
        w.selection = vec![id];
        let mut h = window_harness(w);
        h.key_press(Key::Q);
        h.run_steps(1);
        assert_eq!(h.state().project.event(id).map(|e| e.start.0), Some(start));
        h.get_by_label("Quantisierungs-Panel").click();
        h.run_steps(2);
        assert!(h.state().show_quantize);
        // Beschriftung und Regler tragen beide den Namen.
        assert!(h.query_all_by_label("Swing").count() >= 2);
        assert!(h.query_all_by_label("Stärke").count() >= 2);
    }

    #[test]
    fn renders_quantize_panel() {
        shot("quantisierung", |w| {
            w.show_quantize = true;
            w.grid = Grid::UseQuantize;
            w.quantize.swing = 30;
        });
    }

    #[test]
    fn alt_r_toggles_the_right_zone() {
        let (w, _) = one_event_window();
        let mut h = window_harness(w);
        h.key_press_modifiers(egui::Modifiers::ALT, Key::R);
        h.run_steps(2);
        assert!(h.state().show_right);
        h.get_by_label("Rechte Zone (Alt+R)").click();
        h.run_steps(2);
        assert!(!h.state().show_right);
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
