//! The AI assistant in the project window: a chat panel (Ansicht → KI-Assistent) where a
//! language model edits the open project with the same tools as the MCP server
//! (`studiocraft-automation`). Every edit is an undo step of its own, as from the menus.
//!
//! The model is any OpenAI-compatible chat-completions endpoint with tool calling ([`client`]):
//! a local server (Ollama, LM Studio) or a cloud service. Before the first request to a
//! service that is not on this computer, the person agrees to send their project data there
//! (once per host). Files are read and written only inside the project's folder.
//!
//! Requests run on a worker thread; the tools run on the main thread between frames, on the
//! window the person sees.
// No unwrap/expect/panic outside tests (AGENTS.md "Robustness").
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

pub mod client;
mod conversation;
mod settings;

use std::sync::mpsc::{Receiver, TryRecvError, channel};

use egui::{RichText, Ui};
use serde_json::Value;
use studiocraft_ui::ProjectWindow;
use studiocraft_ui::i18n::{tr, trf};
use studiocraft_ui::theme;

use client::Endpoint;
pub use conversation::{Conversation, Entry, Next};
use settings::Settings;

/// Longest message the input takes (characters).
const MAX_INPUT: usize = 20_000;

/// The assistant: its settings, the conversation and the request under way.
pub struct Assistant {
    settings: Settings,
    conversation: Conversation,
    input: String,
    /// The reply being waited for.
    pending: Option<Receiver<Result<Value, String>>>,
    /// The model list being fetched (`Modelle abfragen`).
    fetching: Option<Receiver<Result<Vec<String>, String>>>,
    models: Vec<String>,
    show_settings: bool,
    /// Waiting for consent to send to this host.
    consent: Option<String>,
    /// A message about the settings or the connection.
    status: Option<String>,
}

impl Default for Assistant {
    fn default() -> Self {
        Self::new()
    }
}

impl Assistant {
    pub fn new() -> Self {
        Self::with_settings(Settings::load())
    }

    fn with_settings(settings: Settings) -> Self {
        Self {
            show_settings: settings.model.trim().is_empty(),
            settings,
            conversation: Conversation::new(),
            input: String::new(),
            pending: None,
            fetching: None,
            models: Vec::new(),
            consent: None,
            status: None,
        }
    }

    /// Whether a reply is being waited for.
    pub fn busy(&self) -> bool {
        self.pending.is_some()
    }

    /// The project window with the panel at its right edge when wanted (Ansicht →
    /// KI-Assistent), below the menu and tool bars.
    pub fn window_ui(&mut self, ui: &mut Ui, window: &mut ProjectWindow) {
        window.ui_with(ui, &mut |ui, window| self.show(ui, window));
    }

    /// Draw the panel at the right edge of `ui` when the window wants it.
    fn show(&mut self, ui: &mut Ui, window: &mut ProjectWindow) {
        // Replies are taken also while the panel is closed: their tools still run.
        self.poll(ui.ctx(), window);
        if !window.assistant_open() {
            return;
        }
        egui::Panel::right("ki-assistent")
            .resizable(true)
            .default_size(340.0)
            .min_size(260.0)
            .show(ui, |ui| self.panel(ui, window));
    }

    fn endpoint(&self) -> Endpoint {
        let key = self.settings.key.trim();
        Endpoint {
            url: self.settings.url.trim().to_owned(),
            model: self.settings.model.trim().to_owned(),
            key: (!key.is_empty()).then(|| key.to_owned()),
        }
    }

    fn poll(&mut self, ctx: &egui::Context, window: &mut ProjectWindow) {
        if let Some(rx) = &self.pending {
            match rx.try_recv() {
                Err(TryRecvError::Empty) => {}
                Err(TryRecvError::Disconnected) => {
                    self.pending = None;
                    self.conversation
                        .failed(tr("Die Anfrage ist ohne Antwort abgebrochen."));
                }
                Ok(Err(e)) => {
                    self.pending = None;
                    self.conversation.failed(&e);
                }
                Ok(Ok(message)) => {
                    self.pending = None;
                    let folder = window
                        .project_path()
                        .and_then(|p| p.parent())
                        .map(std::path::Path::to_path_buf);
                    if self.conversation.reply(&message, window, folder.as_deref()) == Next::Ask {
                        self.send(ctx);
                    }
                }
            }
        }
        if let Some(rx) = &self.fetching {
            match rx.try_recv() {
                Err(TryRecvError::Empty) => {}
                Err(TryRecvError::Disconnected) => self.fetching = None,
                Ok(r) => {
                    self.fetching = None;
                    match r {
                        Ok(models) if models.is_empty() => {
                            self.status = Some(tr("Der Endpunkt nennt keine Modelle.").into());
                        }
                        Ok(models) => {
                            self.status =
                                Some(trf("{n} Modelle gefunden.", &[("n", &models.len())]));
                            self.models = models;
                        }
                        Err(e) => self.status = Some(e),
                    }
                }
            }
        }
    }

    /// Ask the model, on a worker thread.
    fn send(&mut self, ctx: &egui::Context) {
        let endpoint = self.endpoint();
        let messages = self.conversation.messages().to_vec();
        let tools = Conversation::tools();
        let (tx, rx) = channel();
        let ctx = ctx.clone();
        let spawned = std::thread::Builder::new()
            .name("ki-assistent".into())
            .spawn(move || {
                let _ = tx.send(client::chat(&endpoint, &messages, &tools));
                ctx.request_repaint();
            });
        match spawned {
            Ok(_) => self.pending = Some(rx),
            Err(e) => self.conversation.failed(&e.to_string()),
        }
    }

    /// Send what the person typed, once the endpoint is set and, for a service elsewhere,
    /// agreed to.
    fn submit(&mut self, ctx: &egui::Context) {
        let text = self.input.trim().to_owned();
        if text.is_empty() || self.busy() {
            return;
        }
        let endpoint = self.endpoint();
        if let Err(e) = endpoint.check() {
            self.status = Some(e);
            self.show_settings = true;
            return;
        }
        if !endpoint.is_local() {
            let host = client::host(&endpoint.url).unwrap_or_default();
            if !self.settings.consented.contains(&host) {
                self.consent = Some(host);
                return;
            }
        }
        if let Err(e) = self.settings.save() {
            self.status = Some(e);
        }
        self.conversation.user(&text);
        self.input.clear();
        self.send(ctx);
    }

    fn panel(&mut self, ui: &mut Ui, window: &mut ProjectWindow) {
        let t = theme::tokens(ui.ctx());
        ui.horizontal(|ui| {
            ui.heading(tr("KI-Assistent"));
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                if ui.button("×").on_hover_text(tr("Schließen")).clicked() {
                    window.set_assistant_open(false);
                }
                if ui
                    .selectable_label(self.show_settings, tr("Verbindung"))
                    .clicked()
                {
                    self.show_settings = !self.show_settings;
                }
                if ui
                    .add_enabled(!self.busy(), egui::Button::new(tr("Neu")))
                    .on_hover_text(tr("Neues Gespräch beginnen"))
                    .clicked()
                {
                    self.conversation = Conversation::new();
                }
            });
        });
        let endpoint = self.endpoint();
        let place = if endpoint.is_local() {
            tr("Lokal: nichts verlässt diesen Rechner.").to_owned()
        } else {
            match client::host(&endpoint.url) {
                Some(host) => trf("Cloud: Daten gehen an {host}.", &[("host", &host)]),
                None => String::new(),
            }
        };
        ui.label(RichText::new(place).small().color(t.text_dim));
        if self.show_settings {
            self.settings_ui(ui);
        }
        if let Some(status) = &self.status {
            ui.label(RichText::new(status).small().color(t.text_dim));
        }
        ui.separator();
        if let Some(host) = self.consent.clone() {
            self.consent_ui(ui, &host);
        }
        // The input stays at the bottom; the conversation fills the rest.
        egui::Panel::bottom("ki-eingabe")
            .resizable(false)
            .show(ui, |ui| self.input_ui(ui));
        egui::ScrollArea::vertical()
            .auto_shrink([false, false])
            .stick_to_bottom(true)
            .show(ui, |ui| self.entries_ui(ui));
    }

    fn settings_ui(&mut self, ui: &mut Ui) {
        egui::Grid::new("ki-verbindung")
            .num_columns(2)
            .spacing([8.0, 4.0])
            .show(ui, |ui| {
                let l = ui.label(tr("Adresse"));
                ui.add(
                    egui::TextEdit::singleline(&mut self.settings.url)
                        .hint_text(settings::DEFAULT_URL)
                        .desired_width(f32::INFINITY),
                )
                .labelled_by(l.id);
                ui.end_row();
                let l = ui.label(tr("Modell"));
                ui.add(
                    egui::TextEdit::singleline(&mut self.settings.model)
                        .hint_text("qwen2.5:7b")
                        .desired_width(f32::INFINITY),
                )
                .labelled_by(l.id);
                ui.end_row();
                let l = ui.label(tr("API-Schlüssel"));
                ui.add(
                    egui::TextEdit::singleline(&mut self.settings.key)
                        .password(true)
                        .hint_text(tr("leer für lokale Server"))
                        .desired_width(f32::INFINITY),
                )
                .labelled_by(l.id);
                ui.end_row();
            });
        ui.checkbox(&mut self.settings.remember_key, tr("Schlüssel speichern"))
            .on_hover_text(tr("Im Einstellungsordner, nur für dich lesbar"));
        ui.horizontal(|ui| {
            if ui
                .add_enabled(
                    self.fetching.is_none(),
                    egui::Button::new(tr("Modelle abfragen")),
                )
                .clicked()
            {
                self.fetch_models(ui.ctx());
            }
            if ui.button(tr("Sichern")).clicked() {
                self.status = Some(match self.settings.save() {
                    Ok(()) => tr("Verbindung gesichert.").into(),
                    Err(e) => e,
                });
            }
        });
        if !self.models.is_empty() {
            let mut chosen = self.settings.model.clone();
            egui::ComboBox::from_label(tr("Modell wählen"))
                .selected_text(chosen.clone())
                .show_ui(ui, |ui| {
                    for m in &self.models {
                        ui.selectable_value(&mut chosen, m.clone(), m);
                    }
                });
            self.settings.model = chosen;
        }
    }

    fn fetch_models(&mut self, ctx: &egui::Context) {
        let endpoint = self.endpoint();
        let (tx, rx) = channel();
        let ctx = ctx.clone();
        let spawned = std::thread::Builder::new()
            .name("ki-modelle".into())
            .spawn(move || {
                let _ = tx.send(client::models(&endpoint));
                ctx.request_repaint();
            });
        match spawned {
            Ok(_) => {
                self.fetching = Some(rx);
                self.status = Some(tr("Frage Modelle ab …").into());
            }
            Err(e) => self.status = Some(e.to_string()),
        }
    }

    fn consent_ui(&mut self, ui: &mut Ui, host: &str) {
        let t = theme::tokens(ui.ctx());
        egui::Frame::group(ui.style())
            .fill(t.panel_bg2)
            .show(ui, |ui| {
                ui.label(trf(
                    "Deine Nachrichten und Projektdaten (Spurnamen, Noten, Tempo) gehen an {host}. \
                     Der Anbieter kann sie speichern.",
                    &[("host", &host)],
                ));
                ui.horizontal(|ui| {
                    if ui.button(tr("Erlauben")).clicked() {
                        self.settings.consented.push(host.to_owned());
                        self.consent = None;
                        self.submit(ui.ctx());
                    }
                    if ui.button(tr("Abbrechen")).clicked() {
                        self.consent = None;
                    }
                });
            });
        ui.separator();
    }

    fn input_ui(&mut self, ui: &mut Ui) {
        ui.add_space(4.0);
        if self.busy() {
            ui.horizontal(|ui| {
                ui.spinner();
                ui.label(tr("Denkt nach …"));
                if ui.button(tr("Abbrechen")).clicked() {
                    // The worker's answer is dropped when it comes.
                    self.pending = None;
                    self.conversation.failed(tr("Abgebrochen."));
                }
            });
        }
        let l = ui.label(RichText::new(tr("Nachricht")).small());
        let edit = ui
            .add(
                egui::TextEdit::multiline(&mut self.input)
                    .hint_text(tr(
                        "Zum Beispiel: Lege eine Bass-Spur mit einem Part über 4 Takte an",
                    ))
                    .desired_rows(3)
                    .char_limit(MAX_INPUT)
                    .desired_width(f32::INFINITY),
            )
            .labelled_by(l.id);
        // Enter sends, Shift+Enter makes a new line.
        let enter =
            edit.has_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter) && !i.modifiers.shift);
        if enter {
            // The text field has already taken the line break.
            if self.input.ends_with('\n') {
                self.input.pop();
            }
        }
        let send = ui
            .add_enabled(
                !self.busy() && !self.input.trim().is_empty(),
                egui::Button::new(tr("Senden")),
            )
            .clicked();
        if enter || send {
            self.submit(ui.ctx());
        }
        ui.add_space(4.0);
    }

    fn entries_ui(&self, ui: &mut Ui) {
        let t = theme::tokens(ui.ctx());
        if self.conversation.entries().is_empty() {
            ui.label(
                RichText::new(tr(
                    "Sag dem Assistenten, was er im Projekt tun soll. Jede Änderung lässt sich mit Strg+Z zurücknehmen.",
                ))
                .color(t.text_dim),
            );
        }
        for entry in self.conversation.entries() {
            match entry {
                Entry::User(text) => {
                    ui.add_space(6.0);
                    ui.label(RichText::new(tr("Du")).strong().color(t.text));
                    ui.label(text);
                }
                Entry::Model(text) => {
                    ui.add_space(6.0);
                    ui.label(RichText::new(tr("Assistent")).strong().color(t.accent));
                    ui.label(text);
                }
                Entry::Tool { name, args, error } => {
                    let line = match error {
                        None => format!("• {name}  {args}"),
                        Some(e) => format!("! {name}  {args} – {e}"),
                    };
                    let color = if error.is_some() {
                        t.meter_red
                    } else {
                        t.text_dim
                    };
                    ui.label(RichText::new(line).small().color(color));
                }
                Entry::Error(e) => {
                    ui.label(
                        RichText::new(trf("Fehler: {fehler}", &[("fehler", e)])).color(t.meter_red),
                    );
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use egui_kittest::Harness;
    use serde_json::json;
    use studiocraft_project::Project;

    use super::*;

    /// A server that answers each request with the next of `replies` (chat completions) and
    /// returns the request bodies it saw.
    fn fake_model(replies: Vec<Value>) -> (String, std::thread::JoinHandle<Vec<Value>>) {
        use std::io::{BufRead, BufReader, Read, Write};
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let url = format!(
            "http://127.0.0.1:{}/v1",
            listener.local_addr().unwrap().port()
        );
        let t = std::thread::spawn(move || {
            let mut seen = Vec::new();
            for reply in replies {
                let (stream, _) = listener.accept().unwrap();
                let mut reader = BufReader::new(stream);
                let mut length = 0;
                loop {
                    let mut line = String::new();
                    reader.read_line(&mut line).unwrap();
                    if let Some(v) = line.to_ascii_lowercase().strip_prefix("content-length:") {
                        length = v.trim().parse().unwrap();
                    }
                    if line == "\r\n" {
                        break;
                    }
                }
                let mut body = vec![0; length];
                reader.read_exact(&mut body).unwrap();
                seen.push(serde_json::from_slice(&body).unwrap());
                let text =
                    json!({"choices": [{"finish_reason": "stop", "message": reply}]}).to_string();
                write!(
                    reader.into_inner(),
                    "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{text}",
                    text.len()
                )
                .unwrap();
            }
            seen
        });
        (url, t)
    }

    /// Frames of the panel until the assistant is idle again.
    fn run_until_idle(a: &mut Assistant, window: &mut ProjectWindow, ctx: &egui::Context) {
        for _ in 0..500 {
            let mut out = ctx.run_ui(egui::RawInput::default(), |ui| a.poll(ui.ctx(), window));
            out.textures_delta.clear();
            if !a.busy() {
                return;
            }
            std::thread::sleep(std::time::Duration::from_millis(5));
        }
        panic!("no reply");
    }

    #[test]
    fn a_message_runs_the_models_tools_and_shows_its_answer() {
        let (url, server) = fake_model(vec![
            json!({"role": "assistant", "content": "", "tool_calls": [
                {"id": "c1", "type": "function", "function": {"name": "add_track", "arguments": r#"{"kind": "midi", "name": "Bass"}"#}},
            ]}),
            json!({"role": "assistant", "content": "Spur Bass angelegt."}),
        ]);
        let mut window = ProjectWindow::new(Project::default(), None, String::new());
        let before = window.project.tracks.len();
        let mut a = Assistant::with_settings(Settings {
            url,
            model: "test".into(),
            ..Settings::default()
        });
        let ctx = egui::Context::default();
        a.input = "Lege eine Bass-Spur an".into();
        a.conversation.user(&a.input.clone());
        a.input.clear();
        a.send(&ctx);
        run_until_idle(&mut a, &mut window, &ctx);

        assert_eq!(window.project.tracks.len(), before + 1);
        assert_eq!(
            a.conversation.entries().last(),
            Some(&Entry::Model("Spur Bass angelegt.".into()))
        );
        let seen = server.join().unwrap();
        assert_eq!(seen.len(), 2);
        assert_eq!(seen[0]["model"], "test");
        assert!(seen[0]["tools"].as_array().unwrap().len() > 10);
        // The second request carries the tool's result.
        let last = seen[1]["messages"]
            .as_array()
            .unwrap()
            .last()
            .unwrap()
            .clone();
        assert_eq!(last["role"], "tool");
        assert_eq!(last["tool_call_id"], "c1");
    }

    /// Against a real model: `STUDIOCRAFT_AI_TEST_URL` (default Ollama) and
    /// `STUDIOCRAFT_AI_TEST_MODEL`, run with `cargo test -p studiocraft-assistant -- --ignored`.
    #[test]
    #[ignore = "needs a model server"]
    fn a_real_model_edits_the_project() {
        let model = std::env::var("STUDIOCRAFT_AI_TEST_MODEL").expect("STUDIOCRAFT_AI_TEST_MODEL");
        let url = std::env::var("STUDIOCRAFT_AI_TEST_URL")
            .unwrap_or_else(|_| settings::DEFAULT_URL.into());
        let mut window = ProjectWindow::new(Project::default(), None, String::new());
        let mut a = Assistant::with_settings(Settings {
            url,
            model,
            ..Settings::default()
        });
        let ctx = egui::Context::default();
        a.conversation.user(
            "Lege eine MIDI-Spur namens Bass an. Erzeuge darauf ab Takt 1 einen Part über 2 Takte \
             mit vier Viertelnoten C2 (Tonhöhe 36). Stelle das Tempo auf 100 BPM.",
        );
        a.send(&ctx);
        for _ in 0..12_000 {
            let mut out = ctx.run_ui(egui::RawInput::default(), |ui| {
                a.poll(ui.ctx(), &mut window)
            });
            out.textures_delta.clear();
            if !a.busy() {
                break;
            }
            std::thread::sleep(std::time::Duration::from_millis(50));
        }
        for e in a.conversation.entries() {
            eprintln!("{e:?}");
        }
        let p =
            studiocraft_automation::execute(&mut window, "inspect_project", &json!({})).unwrap();
        eprintln!("{}", serde_json::to_string_pretty(&p).unwrap());
        let bass = p["tracks"]
            .as_array()
            .unwrap()
            .iter()
            .find(|t| t["name"] == "Bass");
        assert!(bass.is_some(), "no Bass track");
        assert_eq!(p["tempo"]["bpm"], 100.0);
    }

    #[test]
    fn a_cloud_endpoint_asks_for_consent_first() {
        let mut a = Assistant::with_settings(Settings {
            url: "https://api.example.com/v1".into(),
            model: "m".into(),
            ..Settings::default()
        });
        let ctx = egui::Context::default();
        a.input = "Hallo".into();
        a.submit(&ctx);
        assert_eq!(a.consent.as_deref(), Some("api.example.com"));
        assert!(!a.busy());
        assert!(a.conversation.entries().is_empty());
        // Plain http to another computer is refused outright.
        a.consent = None;
        a.settings.url = "http://api.example.com/v1".into();
        a.submit(&ctx);
        assert!(a.consent.is_none() && !a.busy() && a.status.is_some());
    }

    /// The panel beside the project window, after a short exchange with a tool call and an
    /// error. With `STUDIOCRAFT_SHOTS=<folder>` the picture is saved (dark and light).
    #[test]
    fn the_panel_renders_and_every_widget_is_labelled() {
        for dark in [true, false] {
            let mut window = ProjectWindow::new(Project::demo(8), None, String::new());
            studiocraft_ui::i18n::set_language(studiocraft_ui::i18n::Language::German);
            window.enable_assistant();
            window.set_assistant_open(true);
            let mut a = Assistant::with_settings(Settings::default());
            a.show_settings = true;
            a.conversation
                .user("Lege eine Bass-Spur an und mach sie leiser.");
            a.conversation.reply(
                &json!({"tool_calls": [
                    {"id": "1", "function": {"name": "add_track", "arguments": r#"{"kind": "midi", "name": "Bass"}"#}},
                    {"id": "2", "function": {"name": "set_track", "arguments": r#"{"track": "Bass", "volume_db": -6}"#}},
                    {"id": "3", "function": {"name": "split_event", "arguments": r#"{"event": 99999, "at": 0}"#}},
                ]}),
                &mut window,
                None,
            );
            a.conversation.reply(
                &json!({"content": "Ich habe die Spur „Bass“ angelegt und auf −6 dB gestellt."}),
                &mut window,
                None,
            );
            a.conversation
                .failed("Endpunkt nicht erreichbar (läuft der Server?).");
            let mut harness = Harness::builder()
                .with_size(egui::vec2(1400.0, 860.0))
                .wgpu()
                .build_ui(move |ui| {
                    ui.ctx().set_theme(if dark {
                        egui::Theme::Dark
                    } else {
                        egui::Theme::Light
                    });
                    a.window_ui(ui, &mut window);
                });
            harness.run_steps(4);
            harness.check_accessibility();
            let Ok(img) = harness.render() else {
                eprintln!("kein GPU-Renderer verfügbar, Bild übersprungen");
                continue;
            };
            if let Ok(dir) = std::env::var("STUDIOCRAFT_SHOTS") {
                let name = if dark {
                    "assistant-dark"
                } else {
                    "assistant-light"
                };
                img.save(std::path::Path::new(&dir).join(format!("{name}.png")))
                    .unwrap();
            }
        }
    }

    /// Every `tr("…")`/`trf("…"` text of this crate has an English line in the UI catalogue
    /// (`crates/ui/i18n/en.tsv`).
    #[test]
    fn every_text_has_an_english_entry() {
        let dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("src");
        let mut missing = Vec::new();
        for e in std::fs::read_dir(dir).unwrap().flatten() {
            let file = std::fs::read_to_string(e.path()).unwrap();
            // Tests are not shown to anyone.
            let text = file.split("#[cfg(test)]").next().unwrap_or("");
            for start in ["tr(", "trf("] {
                let mut rest = text;
                while let Some(i) = rest.find(start) {
                    let before = rest[..i].chars().last();
                    rest = &rest[i + start.len()..];
                    if before.is_some_and(|c| c.is_alphanumeric() || c == '_') {
                        continue;
                    }
                    let Some(lit) = rest.trim_start().strip_prefix('"') else {
                        continue;
                    };
                    let Some(end) = lit.find('"') else { break };
                    // Strings continued with `\` at the line end are joined as Rust does.
                    let key = join_continued(&lit[..end]);
                    if !studiocraft_ui::i18n::has_english(&key) {
                        missing.push(key);
                    }
                }
            }
        }
        missing.sort();
        missing.dedup();
        assert!(
            missing.is_empty(),
            "fehlt in crates/ui/i18n/en.tsv:\n{}",
            missing.join("\n")
        );
    }

    fn join_continued(s: &str) -> String {
        let mut out = String::new();
        let mut lines = s.split("\\\n");
        if let Some(first) = lines.next() {
            out.push_str(first);
        }
        for l in lines {
            out.push_str(l.trim_start());
        }
        out
    }
}
