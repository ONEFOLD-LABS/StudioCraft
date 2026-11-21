//! StudioCraft – Desktop-App (Prototyp des Projektfensters).
//!
//! Aufruf: `studiocraft [datei.wav …] [--tracks <n>] [--audio-host <name>] [--control <port>]`
// Unter Windows ohne Konsolenfenster starten (die Kommandozeile ist studiocraft-cli).
#![cfg_attr(all(windows, not(test)), windows_subsystem = "windows")]
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

mod keypad;
#[cfg(target_os = "macos")]
mod macos_open;

use std::{path::Path, process::ExitCode, time::Duration};

use studiocraft_assistant::Assistant;
use studiocraft_automation::control::Control;
use studiocraft_engine::{
    AudioOutput, DeviceChoice, EngineCore, OutputInfo, PreparedOutput, engine,
};
use studiocraft_project::{Project, file};
use studiocraft_ui::ProjectWindow;

const USAGE: &str = "\
Aufruf: studiocraft [projekt.studiocraft | datei.wav …] [--tracks <n>] [--audio-host <name>]
                   [--control <port>]

  projekt.studiocraft  Projekt öffnen
  datei.wav     Audiodateien, jede auf eine eigene Audio-Spur ab Takt 1
                (ohne Dateien: Demo-Projekt mit --tracks Spuren, Standard 100)
  --audio-host  Audio-Host wählen, z. B. PipeWire, ALSA, JACK, WASAPI, CoreAudio
  --control     Steuerkanal für KI-Agenten auf 127.0.0.1:<port> öffnen
                (`studiocraft-cli mcp --connect`, siehe docs/mcp.md)

Weitere Dateien lassen sich ins Projektfenster ziehen.";

/// Reverse-DNS id: Wayland app id, X11 WM class and desktop file name.
const APP_ID: &str = "io.github.xxcrusaderxx.studiocraft";

/// Fenster-Icon „Welle“; macOS bekommt die eigene Variante mit Rand.
fn app_icon() -> egui::IconData {
    #[cfg(target_os = "macos")]
    let png: &[u8] = include_bytes!("../../../assets/app-icon/studiocraft-macos-512.png");
    #[cfg(not(target_os = "macos"))]
    let png: &[u8] =
        include_bytes!("../../../assets/app-icon/hicolor/256x256/apps/studiocraft.png");
    eframe::icon_data::from_png_bytes(png).unwrap_or_default()
}

/// Meldet dem Plugin-Host das Hauptfenster: Plugin-Fenster hängen unter X11 daran und
/// wählen unter Wayland ihren Weg danach (Issue #175).
fn report_main_window(cc: &eframe::CreationContext<'_>) {
    use raw_window_handle::{HasWindowHandle, RawWindowHandle};
    use studiocraft_plugin_host::MainWindow;
    let window = match cc.window_handle().map(|h| h.as_raw()) {
        // `c_ulong`: 64 bits on Linux, 32 on Windows, where the conversion is a no-op.
        #[allow(clippy::useless_conversion)]
        Ok(RawWindowHandle::Xlib(h)) => {
            u32::try_from(h.window).map_or(MainWindow::Other, MainWindow::X11)
        }
        Ok(RawWindowHandle::Xcb(h)) => MainWindow::X11(h.window.get()),
        Ok(RawWindowHandle::Wayland(_)) => MainWindow::Wayland,
        _ => MainWindow::Other,
    };
    studiocraft_plugin_host::set_main_window(window);
}

struct App {
    // Reihenfolge der Felder = Reihenfolge beim Aufräumen: erst der Steuerkanal (nimmt die
    // Token-Datei weg), dann das Fenster (es hält Plugin-Handles), dann der Kern mit den
    // Plugin-Instanzen.
    /// Steuerkanal für KI-Agenten (`--control`).
    control: Option<Control>,
    /// KI-Assistent (Ansicht → KI-Assistent).
    assistant: Assistant,
    window: ProjectWindow,
    /// Hält den Audio-Stream am Leben.
    audio: Option<AudioOutput>,
    /// Kern ohne Stream, wenn ein Gerätewechsel scheiterte; wird nie nebenbei zerstört.
    parked: Option<EngineCore>,
    /// Ein verschwundenes Gerät nur einmal melden.
    lost_reported: bool,
}

impl eframe::App for App {
    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        #[cfg(target_os = "macos")]
        self.open_from_finder(ui.ctx());
        // Anfragen von Agenten vor dem Zeichnen: ihre Änderungen sind im selben Bild zu sehen.
        if let Some(control) = &mut self.control {
            control.poll(&mut self.window, Some(ui.ctx()));
        }
        // Das Fenster mit der Leiste des KI-Assistenten am rechten Rand.
        self.assistant.window_ui(ui, &mut self.window);
        if let Some(choice) = self.window.take_device_request() {
            self.switch_audio(&choice);
        }
        if !self.lost_reported && self.audio.as_ref().is_some_and(AudioOutput::device_lost) {
            self.lost_reported = true;
            self.window.audio_lost();
        }
    }

    fn on_exit(&mut self) {
        // Token-Datei entfernen: ein Agent soll ein beendetes Fenster nicht mehr erreichen.
        self.control = None;
        // Sauber beendet: keine Wiederherstellung beim nächsten Start.
        self.window.end_session();
    }
}

/// Statuszeile für eine laufende Ausgabe.
fn status_line(info: &OutputInfo) -> String {
    let block = info
        .block_frames
        .map_or("Treiber".into(), |b| b.to_string());
    let mut line = format!(
        "{} · {} · {} Hz · Block {block}",
        info.host, info.device, info.sample_rate
    );
    if let Some(ms) = info.latency_ms() {
        line.push_str(&format!(" · Latenz {ms:.1} ms"));
    }
    // Das Gerät lehnte den gewählten Modus ab: sagen, warum ein anderer läuft.
    if let Some(note) = &info.note {
        line.push_str(&format!(" · Rückfall ({note})"));
    }
    line
}

impl App {
    /// macOS: Dateien, die der Finder geschickt hat (Doppelklick, „Öffnen mit“, auf das
    /// Dock-Symbol gezogen), wie Startparameter öffnen: Projekte öffnen, Audio importieren.
    #[cfg(target_os = "macos")]
    fn open_from_finder(&mut self, ctx: &egui::Context) {
        macos_open::set_context(ctx);
        for path in macos_open::take() {
            let result = if path.extension().is_some_and(|e| e == file::EXTENSION) {
                self.window.open(&path)
            } else {
                self.window.import_audio(&path, None).map(|_| ())
            };
            if let Err(e) = result {
                self.window.notify(format!("{}: {e}", path.display()));
            }
        }
    }

    /// Wechselt das Audiogerät ohne Neustart: der laufende Kern (Song, Plugins, Transport)
    /// zieht auf das neue Gerät um. Das geht nur mit derselben Abtastrate.
    fn switch_audio(&mut self, choice: &DeviceChoice) {
        let rate = self.window.sample_rate();
        let want = DeviceChoice {
            sample_rate: Some(rate),
            ..choice.clone()
        };
        let prepared = match PreparedOutput::open_with(&want) {
            Ok(p) => p,
            Err(e) => return self.window.audio_switched(Err(e)),
        };
        let info = prepared.info.clone();
        if info.sample_rate != rate {
            return self.window.audio_switched(Err(format!(
                "„{}“ läuft nicht mit {rate} Hz; die Wahl gilt nach einem Neustart",
                info.device
            )));
        }
        let core = match (self.parked.take(), self.audio.take()) {
            (Some(core), audio) => {
                self.audio = audio;
                core
            }
            (None, Some(out)) => match out.take_core(Duration::from_millis(500)) {
                Ok(core) => core,
                Err(b) => {
                    let (out, e) = *b;
                    self.audio = Some(out);
                    return self.window.audio_switched(Err(e));
                }
            },
            (None, None) => {
                return self.window.audio_switched(Err(
                    "Ohne Audio gestartet; die Wahl gilt nach einem Neustart".into(),
                ));
            }
        };
        match prepared.start(core) {
            Ok(out) => {
                self.audio = Some(out);
                self.lost_reported = false;
                let mut status = status_line(&info);
                if choice.sample_rate.is_some_and(|r| r != rate) {
                    status.push_str(" (neue Abtastrate nach Neustart)");
                }
                self.window.set_output_channels(info.channels);
                self.window.audio_switched(Ok(status));
            }
            Err(b) => {
                let (core, e) = *b;
                self.parked = Some(core);
                self.window
                    .audio_switched(Err(format!("Gerätewechsel fehlgeschlagen: {e}")));
            }
        }
    }
}

use eframe::egui;

fn main() -> ExitCode {
    // Kindprozess des Plugin-Scans: nur diese eine Datei prüfen, dann beenden.
    let raw: Vec<String> = std::env::args().collect();
    if let Some(code) = studiocraft_ui::run_plugin_scan_child(&raw) {
        return ExitCode::from(u8::try_from(code).unwrap_or(1));
    }
    // Plugin im eigenen Prozess (Bridge, Issue #146): dieser Prozess ist der Helfer.
    if let Some(code) = studiocraft_ui::run_plugin_bridge_child(&raw) {
        return ExitCode::from(u8::try_from(code).unwrap_or(1));
    }
    if let Ok(exe) = std::env::current_exe() {
        studiocraft_ui::enable_isolated_plugin_scan(exe);
    }
    let mut files = Vec::new();
    let mut tracks = 100usize;
    let mut host = None;
    let mut control_port: Option<u16> = None;
    let mut args = std::env::args().skip(1);
    while let Some(a) = args.next() {
        match a.as_str() {
            "--tracks" => match args.next().and_then(|v| v.parse().ok()) {
                Some(n) => tracks = n,
                None => {
                    eprintln!("--tracks erwartet eine Zahl");
                    return ExitCode::from(2);
                }
            },
            "--audio-host" => host = args.next(),
            "--control" => match args.next().and_then(|v| v.parse().ok()).filter(|p| *p != 0) {
                Some(p) => control_port = Some(p),
                None => {
                    eprintln!("--control erwartet eine Portnummer");
                    return ExitCode::from(2);
                }
            },
            "-V" | "--version" => {
                println!("studiocraft {}", env!("CARGO_PKG_VERSION"));
                return ExitCode::SUCCESS;
            }
            "-h" | "--help" => {
                println!("{USAGE}");
                return ExitCode::SUCCESS;
            }
            _ if !a.starts_with('-') => files.push(a),
            _ => {
                eprintln!("Unbekanntes Argument: {a}\n\n{USAGE}");
                return ExitCode::from(2);
            }
        }
    }

    // Mit Dateien: leeres Projekt, jede Datei auf eigener Spur. Ohne: Demo-Spuren.
    let project = if files.is_empty() {
        Project::demo(tracks)
    } else {
        Project::default()
    };

    // Audio-Geräte aus den Einstellungen; ein Host von der Kommandozeile hat Vorrang.
    let mut choice = studiocraft_ui::settings::Settings::load().audio;
    if host.is_some() {
        choice = DeviceChoice {
            host: host.clone(),
            ..DeviceChoice::default()
        };
    }
    let wanted = DeviceChoice {
        sample_rate: Some(choice.sample_rate.unwrap_or(48_000)),
        ..choice.clone()
    };
    let mut note = None;
    let prepared = PreparedOutput::open_with(&wanted).or_else(|e| {
        // Gespeichertes Gerät fehlt: Standardgerät, mit Hinweis.
        if wanted.output.is_none() {
            return Err(e);
        }
        note = Some(e);
        PreparedOutput::open_with(&DeviceChoice {
            output: None,
            ..wanted.clone()
        })
    });
    let mut out_channels = 2u16;
    let (transport, audio, status) = match prepared {
        Ok(prepared) => {
            let info = prepared.info.clone();
            let (core, ctl) = engine(info.sample_rate, project.tempo.clone());
            match prepared.start(core) {
                Ok(out) => {
                    out_channels = info.channels;
                    (Some(ctl), Some(out), status_line(&info))
                }
                // Beim Start gibt es noch keine Plugins: der Kern darf hier gehen.
                Err(b) => (None, None, format!("Kein Audio: {}", b.1)),
            }
        }
        Err(e) => (None, None, format!("Kein Audio: {e}")),
    };

    let mut window = ProjectWindow::new(project, transport, status);
    window.set_audio_choice(choice);
    window.set_output_channels(out_channels);
    if let Some(n) = note {
        window.audio_switched(Err(format!("{n}; Standardgerät verwendet")));
    }
    window.open_midi_inputs();
    // Automatische Sicherung; nach einem Absturz bietet ein Dialog die neueste Sicherung an.
    window.begin_session();
    for f in &files {
        let path = Path::new(f);
        let result = if path.extension().is_some_and(|e| e == file::EXTENSION) {
            window.open(path)
        } else {
            window.import_audio(path, None).map(|_| ())
        };
        // Fehler im Fenster zeigen: unter Windows gibt es keine Konsole (Doppelklick im
        // Explorer), das Programm soll nicht wortlos enden.
        if let Err(e) = result {
            eprintln!("Fehler: {e}");
            window.notify(format!("{f}: {e}"));
        }
    }
    window.clear_history();

    // Eigene Titelleiste: craft-window entscheidet pro Desktop, ob die
    // App sie zeichnet; STUDIOCRAFT_TITLEBAR=system behält den Rahmen des Systems.
    let frame = studiocraft_ui::titlebar::decide_frame();
    let builder = egui::ViewportBuilder::default().with_title("StudioCraft");
    let builder = studiocraft_ui::titlebar::apply_frame(builder, &frame);
    window.set_window_frame(frame);
    // Eigene Tastaturbelegung aus dem Einstellungsordner.
    window.load_keymap();
    // Nummernblock getrennt von der Zahlenreihe (eigene Ereignisschleife, siehe keypad.rs).
    let tap = studiocraft_ui::keypad::KeypadTap::new();
    window.set_keypad(tap.clone());
    let options = eframe::NativeOptions {
        viewport: builder
            // Wayland finds the window icon through the app id and the installed desktop file
            // of the same name (packaging/linux/io.github.xxcrusaderxx.studiocraft.desktop).
            .with_app_id(APP_ID)
            .with_inner_size([1400.0, 860.0])
            .with_icon(app_icon())
            .with_drag_and_drop(true),
        ..Default::default()
    };
    let event_loop =
        match winit::event_loop::EventLoop::<eframe::UserEvent>::with_user_event().build() {
            Ok(l) => l,
            Err(e) => {
                eprintln!("Fehler: {e}");
                return ExitCode::FAILURE;
            }
        };
    // macOS: Dokumente aus dem Finder empfangen (winits Delegate gibt es jetzt).
    #[cfg(target_os = "macos")]
    macos_open::install();
    let eframe_app = eframe::create_native(
        "StudioCraft",
        options,
        Box::new(move |cc| {
            let mut window = window;
            window.enable_assistant();
            report_main_window(cc);
            let control = control_port.and_then(|port| {
                let ctx = cc.egui_ctx.clone();
                match Control::start(port, move || ctx.request_repaint()) {
                    Ok(c) => Some(c),
                    Err(e) => {
                        eprintln!("Fehler: Steuerkanal: {e}");
                        window.notify(studiocraft_ui::i18n::trf(
                            "Steuerkanal: {fehler}",
                            &[("fehler", &e)],
                        ));
                        None
                    }
                }
            });
            Ok(Box::new(App {
                control,
                assistant: Assistant::new(),
                window,
                audio,
                parked: None,
                lost_reported: false,
            }))
        }),
        &event_loop,
    );
    let mut app = keypad::KeypadApp::new(eframe_app, tap);
    let result = event_loop.run_app(&mut app);
    match result {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => {
            eprintln!("Fehler: {e}");
            ExitCode::FAILURE
        }
    }
}
