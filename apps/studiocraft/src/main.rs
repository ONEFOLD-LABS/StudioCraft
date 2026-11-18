//! StudioCraft – Desktop-App (Prototyp des Projektfensters).
//!
//! Aufruf: `studiocraft [datei.wav …] [--tracks <n>] [--audio-host <name>]`

use std::{path::Path, process::ExitCode};

use studiocraft_engine::{PreparedOutput, engine};
use studiocraft_project::{Project, file};
use studiocraft_ui::ProjectWindow;

const USAGE: &str = "\
Aufruf: studiocraft [projekt.studiocraft | datei.wav …] [--tracks <n>] [--audio-host <name>]

  projekt.studiocraft  Projekt öffnen
  datei.wav     Audiodateien, jede auf eine eigene Audio-Spur ab Takt 1
                (ohne Dateien: Demo-Projekt mit --tracks Spuren, Standard 100)
  --audio-host  Audio-Host wählen, z. B. PipeWire, ALSA, JACK, WASAPI, CoreAudio

Weitere Dateien lassen sich ins Projektfenster ziehen.";

struct App {
    window: ProjectWindow,
    /// Hält den Audio-Stream am Leben.
    _audio: Option<studiocraft_engine::AudioOutput>,
}

impl eframe::App for App {
    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        self.window.ui(ui);
    }
}

use eframe::egui;

fn main() -> ExitCode {
    let mut files = Vec::new();
    let mut tracks = 100usize;
    let mut host = None;
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

    let (transport, audio, status) = match PreparedOutput::open(host.as_deref(), 48_000) {
        Ok(prepared) => {
            let info = prepared.info.clone();
            let (core, ctl) = engine(info.sample_rate, project.tempo.clone());
            match prepared.start(core) {
                Ok(out) => {
                    let block = info
                        .block_frames
                        .map_or("Treiber".into(), |b| b.to_string());
                    let status = format!(
                        "{} · {} · {} Hz · Block {block}",
                        info.host, info.device, info.sample_rate
                    );
                    (Some(ctl), Some(out), status)
                }
                Err(e) => (None, None, format!("Kein Audio: {e}")),
            }
        }
        Err(e) => (None, None, format!("Kein Audio: {e}")),
    };

    let mut window = ProjectWindow::new(project, transport, status);
    for f in &files {
        let path = Path::new(f);
        let result = if path.extension().is_some_and(|e| e == file::EXTENSION) {
            window.open(path)
        } else {
            window.import_audio(path, None).map(|_| ())
        };
        if let Err(e) = result {
            eprintln!("Fehler: {e}");
            return ExitCode::FAILURE;
        }
    }
    window.clear_history();

    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_title("StudioCraft")
            .with_app_id("studiocraft")
            .with_inner_size([1400.0, 860.0])
            .with_drag_and_drop(true),
        ..Default::default()
    };
    let result = eframe::run_native(
        "StudioCraft",
        options,
        Box::new(move |_cc| {
            Ok(Box::new(App {
                window,
                _audio: audio,
            }))
        }),
    );
    match result {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => {
            eprintln!("Fehler: {e}");
            ExitCode::FAILURE
        }
    }
}
