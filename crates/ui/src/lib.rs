//! Oberfläche von StudioCraft (egui), aufgebaut nach `docs/spec.md`,
//! Abschnitte „Fenster-Architektur“, „Werkzeuge“, „Raster, Snap und Bearbeitung“.
//!
//! Prototyp-Stand: Projektfenster mit Werkzeugzeile, Inspector, Spurliste,
//! Lineal, Event-Anzeige und Transportleiste.

mod timeline;
mod tools;
pub mod waveform;

use std::{collections::HashMap, path::Path, sync::Arc};

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
        };
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
        let cmd = if self.is_playing() {
            Command::Stop
        } else {
            Command::Play
        };
        self.send(cmd);
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
        let (keys, alt) = ui.input(|i| {
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
            ] {
                if i.key_pressed(k) {
                    pressed.push(k);
                }
            }
            (pressed, i.modifiers.alt)
        });
        for k in keys {
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
                Key::Delete | Key::Backspace => {
                    self.dirty = true;
                    for id in std::mem::take(&mut self.selection) {
                        self.project.delete(id);
                    }
                }
                Key::Home | Key::Comma => self.locate(0),
                Key::I if alt => self.show_inspector = !self.show_inspector,
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
        self.handle_keys(ui);
        let dt = ui.input(|i| i.unstable_dt) * 1000.0;
        self.frame_ms = if self.frame_ms == 0.0 {
            dt
        } else {
            self.frame_ms * 0.95 + dt * 0.05
        };

        egui::Panel::top("werkzeugzeile").show(ui, |ui| self.toolbar(ui));
        egui::Panel::bottom("transportleiste").show(ui, |ui| self.transport_bar(ui));
        if self.show_inspector {
            egui::Panel::left("inspector")
                .resizable(true)
                .default_size(200.0)
                .show(ui, |ui| self.inspector(ui));
        }
        egui::CentralPanel::default().show(ui, |ui| timeline::show(self, ui));

        if self.dirty {
            self.dirty = false;
            self.rebuild_song();
        }

        if self.is_playing() {
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
        let track = &mut self.project.tracks[idx];
        ui.horizontal(|ui| {
            let label = ui.label("Name");
            ui.add(egui::TextEdit::singleline(&mut track.name))
                .labelled_by(label.id);
        });
        ui.label(format!("Typ: {}", track.kind.label()));
        ui.horizontal(|ui| {
            let m = ui.toggle_value(&mut track.mute, "M").changed();
            let s = ui.toggle_value(&mut track.solo, "S").changed();
            self.dirty |= m || s;
            if track.kind.holds_events() {
                ui.toggle_value(&mut track.record, "R");
            }
        });
        let mut c = Color32::from_rgb(track.color[0], track.color[1], track.color[2]);
        if ui.color_edit_button_srgba(&mut c).changed() {
            track.color = [c.r(), c.g(), c.b()];
        }
        ui.label(format!("{} Events", track.events.len()));
        ui.separator();
        for section in ["Eingang/Ausgang", "Inserts", "EQ", "Sends", "Notizen"] {
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
                self.send(Command::Stop);
            }
            let playing = self.is_playing();
            if ui
                .selectable_label(playing, "▶")
                .on_hover_text("Start/Stopp (Leertaste)")
                .clicked()
            {
                self.toggle_play();
            }
            if ui
                .selectable_label(self.cycle, "🔁")
                .on_hover_text("Cycle (/)")
                .clicked()
            {
                self.toggle_cycle();
            }
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

/// Ticks pro Viertel als `f64`, für Umrechnungen in Pixel.
pub(crate) const TPQ: f64 = TICKS_PER_QUARTER as f64;

#[cfg(test)]
mod tests {
    use super::*;
    use egui_kittest::Harness;
    use studiocraft_project::TempoMap;

    /// Rendert das Projektfenster headless. Mit `STUDIOCRAFT_SHOTS=<ordner>`
    /// werden die Bilder gespeichert, sonst wird nur geprüft, dass es rendert.
    fn shot(name: &str, setup: impl FnOnce(&mut ProjectWindow)) {
        let mut w = ProjectWindow::new(Project::demo(100), None, "kein Audio (Test)".into());
        setup(&mut w);
        let mut harness = Harness::builder()
            .with_size(egui::vec2(1400.0, 860.0))
            .wgpu()
            .build_ui(move |ui| w.ui(ui));
        harness.run();
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
            let art = if audio { "Audio-Spuren mit Wellenform" } else { "Spuren" };
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
