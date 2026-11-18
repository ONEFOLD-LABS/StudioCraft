//! StudioCraft – Desktop-App (Prototyp des Projektfensters).
//!
//! Aufruf: `studiocraft [datei.wav] [--tracks <n>] [--audio-host <name>]`
//! Ohne Datei läuft der Transport mit Stille, damit Cursor und Cycle sichtbar sind.

use std::process::ExitCode;

use studiocraft_engine::{AudioClip, PreparedOutput, engine};
use studiocraft_project::Project;
use studiocraft_ui::ProjectWindow;

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
    let mut file = None;
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
                println!("Aufruf: studiocraft [datei.wav] [--tracks <n>] [--audio-host <name>]");
                return ExitCode::SUCCESS;
            }
            _ if !a.starts_with('-') && file.is_none() => file = Some(a),
            _ => {
                eprintln!("Unbekanntes Argument: {a}");
                return ExitCode::from(2);
            }
        }
    }

    let project = Project::demo(tracks);
    let clip = match &file {
        Some(f) => match AudioClip::load_wav(f) {
            Ok(c) => Some(c),
            Err(e) => {
                eprintln!("Fehler: {f}: {e}");
                return ExitCode::FAILURE;
            }
        },
        None => None,
    };
    let preferred = clip.as_ref().map_or(48_000, |c| c.sample_rate);

    let (transport, audio, status) = match PreparedOutput::open(host.as_deref(), preferred) {
        Ok(prepared) => {
            let info = prepared.info.clone();
            let clip = match clip {
                Some(c) => c.resampled(info.sample_rate),
                None => AudioClip {
                    sample_rate: info.sample_rate,
                    samples: Vec::new().into(),
                },
            };
            let (core, ctl) = engine(clip, project.tempo.clone());
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

    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_title("StudioCraft")
            .with_app_id("studiocraft")
            .with_inner_size([1400.0, 860.0]),
        ..Default::default()
    };
    let result = eframe::run_native(
        "StudioCraft",
        options,
        Box::new(move |_cc| {
            Ok(Box::new(App {
                window: ProjectWindow::new(project, transport, status),
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
