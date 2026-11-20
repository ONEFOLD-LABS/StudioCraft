//! Start im Browser und Audio nach der ersten Eingabe.
//!
//! Browser lassen Ton erst nach einer Geste des Nutzers zu: ein AudioContext, der vorher
//! entsteht, bleibt stumm. Deshalb startet die Ausgabe hier erst beim ersten Klick oder
//! Tastendruck im Fenster (aus Rust, ohne eigenes JavaScript); bis dahin läuft das Fenster
//! ohne Engine.

use eframe::wasm_bindgen::JsCast;
use studiocraft_engine::{AudioOutput, DeviceChoice, PreparedOutput, engine};
use studiocraft_project::Project;
use studiocraft_ui::ProjectWindow;

/// Id des Canvas in index.html.
const CANVAS_ID: &str = "studiocraft_canvas";

struct App {
    // Erst das Fenster, dann der Stream mit dem Kern (wie in der Desktop-App).
    window: ProjectWindow,
    audio: Option<AudioOutput>,
    /// Start der Ausgabe schon versucht (gelingt er nicht, bleibt es bei der Meldung).
    tried: bool,
}

impl App {
    fn new() -> Self {
        let window = ProjectWindow::new(
            Project::demo(8),
            None,
            "Kein Ton – zum Starten ins Fenster klicken".into(),
        );
        Self {
            window,
            audio: None,
            tried: false,
        }
    }

    /// Öffnet die Web-Audio-Ausgabe und hängt eine frische Engine an das Fenster.
    fn start_audio(&mut self) {
        let prepared = match PreparedOutput::open_with(&DeviceChoice::default()) {
            Ok(p) => p,
            Err(e) => return self.window.audio_switched(Err(format!("Kein Ton: {e}"))),
        };
        let info = prepared.info.clone();
        let (core, ctl) = engine(info.sample_rate, self.window.project.tempo.clone());
        match prepared.start(core) {
            Ok(out) => {
                self.audio = Some(out);
                self.window.set_output_channels(info.channels);
                self.window
                    .attach_transport(ctl, format!("{} · {} Hz", info.host, info.sample_rate));
            }
            Err(b) => {
                let (_core, e) = *b;
                self.window.audio_switched(Err(format!("Kein Ton: {e}")));
            }
        }
    }
}

impl eframe::App for App {
    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        if !self.tried
            && ui.input(|i| {
                i.events.iter().any(|e| {
                    matches!(
                        e,
                        egui::Event::PointerButton { pressed: true, .. }
                            | egui::Event::Key { pressed: true, .. }
                    )
                })
            })
        {
            self.tried = true;
            self.start_audio();
        }
        self.window.ui(ui);
    }
}

/// Sucht das Canvas und startet eframe darin.
pub fn start() {
    wasm_bindgen_futures::spawn_local(async {
        let canvas = web_sys::window()
            .and_then(|w| w.document())
            .and_then(|d| d.get_element_by_id(CANVAS_ID))
            .and_then(|e| e.dyn_into::<web_sys::HtmlCanvasElement>().ok());
        let Some(canvas) = canvas else {
            return;
        };
        let _ = eframe::WebRunner::new()
            .start(
                canvas,
                eframe::WebOptions::default(),
                Box::new(|_cc| Ok(Box::new(App::new()))),
            )
            .await;
    });
}
