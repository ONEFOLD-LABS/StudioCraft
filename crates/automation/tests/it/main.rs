//! Drives the MCP server through JSON-RPC lines only, as an agent would.

// A test program with its own main (harness = false): it may unwrap and panic like any test
// (Cargo.toml lints, clippy.toml).
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use std::path::{Path, PathBuf};

use serde_json::{Value, json};
use studiocraft_automation::{Roots, Studio, connect, server};

struct Agent {
    server: craft_mcp_server::Server<Studio>,
    next: u64,
}

impl Agent {
    fn new(read: &Path, write: &Path) -> Self {
        let roots = Roots::new(&[read.to_path_buf()], &[write.to_path_buf()]).unwrap();
        Self::with(server(roots))
    }

    /// An agent on the app's control channel at `addr`.
    fn connected(read: &Path, write: &Path, addr: &str, token: &str) -> Self {
        let roots = Roots::new(&[read.to_path_buf()], &[write.to_path_buf()]).unwrap();
        Self::with(connect(roots, addr, Some(token)))
    }

    fn with(server: craft_mcp_server::Server<Studio>) -> Self {
        let mut a = Self { server, next: 1 };
        a.request("initialize", json!({"protocolVersion": "2025-06-18", "capabilities": {}, "clientInfo": {"name": "test", "version": "1"}}));
        a.server
            .handle_line(r#"{"jsonrpc":"2.0","method":"notifications/initialized"}"#);
        a
    }

    fn request(&mut self, method: &str, params: Value) -> Value {
        let id = self.next;
        self.next += 1;
        let line = json!({"jsonrpc": "2.0", "id": id, "method": method, "params": params});
        let reply = self.server.handle_line(&line.to_string()).unwrap();
        let reply: Value = serde_json::from_str(&reply).unwrap();
        assert_eq!(reply["id"], id);
        reply["result"].clone()
    }

    /// A tool call: `Ok(json)` on success, `Err(text)` for an error result.
    fn call(&mut self, name: &str, args: Value) -> Result<Value, String> {
        let r = self.request("tools/call", json!({"name": name, "arguments": args}));
        let text = r["content"][0]["text"].as_str().unwrap().to_owned();
        if r["isError"] == true {
            Err(text)
        } else {
            Ok(serde_json::from_str(&text).unwrap())
        }
    }

    fn ok(&mut self, name: &str, args: Value) -> Value {
        self.call(name, args)
            .unwrap_or_else(|e| panic!("{name} failed: {e}"))
    }
}

fn temp(name: &str) -> PathBuf {
    let d = std::env::temp_dir().join(format!("studiocraft-mcp-{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&d);
    std::fs::create_dir_all(&d).unwrap();
    d
}

/// A one-second 16-bit stereo sine at 48 kHz as WAV bytes.
fn sine_wav() -> Vec<u8> {
    let rate = 48_000u32;
    let frames = rate as usize;
    let mut data = Vec::with_capacity(frames * 4);
    for i in 0..frames {
        let s = (f64::from(i as u32) * 440.0 * std::f64::consts::TAU / f64::from(rate)).sin();
        let v = ((s * 0.5) * f64::from(i16::MAX)) as i16;
        data.extend_from_slice(&v.to_le_bytes());
        data.extend_from_slice(&v.to_le_bytes());
    }
    let mut w = Vec::new();
    w.extend_from_slice(b"RIFF");
    w.extend_from_slice(&(36 + data.len() as u32).to_le_bytes());
    w.extend_from_slice(b"WAVEfmt ");
    w.extend_from_slice(&16u32.to_le_bytes());
    w.extend_from_slice(&1u16.to_le_bytes());
    w.extend_from_slice(&2u16.to_le_bytes());
    w.extend_from_slice(&rate.to_le_bytes());
    w.extend_from_slice(&(rate * 4).to_le_bytes());
    w.extend_from_slice(&4u16.to_le_bytes());
    w.extend_from_slice(&16u16.to_le_bytes());
    w.extend_from_slice(b"data");
    w.extend_from_slice(&(data.len() as u32).to_le_bytes());
    w.extend_from_slice(&data);
    w
}

#[test]
fn an_agent_builds_renders_saves_and_reopens_a_project() {
    let dir = temp("session");
    let input = dir.join("in");
    let out = dir.join("out");
    std::fs::create_dir_all(&input).unwrap();
    std::fs::create_dir_all(&out).unwrap();
    std::fs::write(input.join("tone.wav"), sine_wav()).unwrap();
    let mut a = Agent::new(&dir, &out);

    let tools = a.request("tools/list", json!({}));
    assert!(tools["tools"].as_array().unwrap().len() > 10);

    // Audio on a new track, a MIDI part with notes on another.
    let ev = a.ok("import_audio", json!({"path": input.join("tone.wav")}));
    assert_eq!(ev["track"], 0);
    a.ok("add_track", json!({"kind": "instrument", "name": "Keys"}));
    let part = a.ok(
        "create_part",
        json!({"track": "Keys", "start": "2.1.1.0", "length": "1.0.0.0"}),
    );
    let part_id = part["id"].clone();
    a.ok(
        "add_notes",
        json!({"event": part_id, "notes": [
            {"pitch": 60, "start": 0, "length": "0.1.0.0"},
            {"pitch": 67, "start": "0.2.0.0", "length": 480, "velocity": 80},
        ]}),
    );
    let notes = a.ok("get_notes", json!({"event": part_id}));
    assert_eq!(notes["notes"].as_array().unwrap().len(), 2);

    // Mixdown: the tone is audible, a quieter fader makes it quieter.
    a.ok(
        "set_locators",
        json!({"left": "1.1.1.0", "right": "2.1.1.0"}),
    );
    let loud = a.ok("export_mixdown", json!({"path": out.join("loud.wav")}));
    a.ok("set_track", json!({"track": 0, "volume_db": -12.0}));
    let quiet = a.ok("export_mixdown", json!({"path": out.join("quiet.wav")}));
    let (loud, quiet) = (
        loud["peak_dbfs"].as_f64().unwrap(),
        quiet["peak_dbfs"].as_f64().unwrap(),
    );
    assert!(loud > -20.0, "tone should be audible: {loud}");
    assert!(
        quiet < loud - 6.0,
        "fader should lower the mix: {loud} → {quiet}"
    );
    // An existing file is kept unless the agent asks to replace it.
    assert!(
        a.call("export_mixdown", json!({"path": out.join("quiet.wav")}))
            .is_err()
    );
    assert!(
        a.call(
            "export_mixdown",
            json!({"path": out.join("quiet.wav"), "replace": true})
        )
        .is_ok()
    );

    // Undo takes back the fader move, redo brings it again.
    a.ok("undo", json!({}));
    let p = a.ok("inspect_project", json!({}));
    assert!(p["tracks"][0]["volume_db"].as_f64().unwrap() > -1.0);
    a.ok("redo", json!({}));

    // Save, start over, reopen: the project comes back with its audio.
    let saved = a.ok("save_project", json!({"path": out.join("Song")}));
    assert!(
        saved["saved"]
            .as_str()
            .unwrap()
            .ends_with("Song.studiocraft")
    );
    let before = a.ok("inspect_project", json!({}));
    a.ok("new_project", json!({}));
    assert_eq!(
        a.ok("inspect_project", json!({}))["tracks"]
            .as_array()
            .unwrap()
            .len(),
        0
    );
    let reopened = a.ok(
        "open_project",
        json!({"path": out.join("Song.studiocraft")}),
    );
    assert_eq!(reopened["tracks"], before["tracks"]);
    assert!(
        reopened["message"].is_null(),
        "audio should load: {}",
        reopened["message"]
    );
}

#[test]
fn paths_outside_the_roots_are_refused() {
    let dir = temp("confined");
    let read = dir.join("read");
    let write = dir.join("write");
    let other = dir.join("other");
    for d in [&read, &write, &other] {
        std::fs::create_dir_all(d).unwrap();
    }
    std::fs::write(other.join("tone.wav"), sine_wav()).unwrap();
    let mut a = Agent::new(&read, &write);

    let e = a
        .call("import_audio", json!({"path": other.join("tone.wav")}))
        .unwrap_err();
    assert!(e.contains("outside the read root"), "{e}");
    let e = a
        .call("save_project", json!({"path": read.join("x")}))
        .unwrap_err();
    assert!(e.contains("outside the write root"), "{e}");
    let e = a
        .call("export_midi", json!({"path": write.join("../other/x.mid")}))
        .unwrap_err();
    assert!(e.contains("outside the write root"), "{e}");
    assert!(!other.join("x.mid").exists());

    // A project inside the read root that names audio outside it is refused as a whole.
    let mut outside = Agent::new(&dir, &dir);
    outside.ok("import_audio", json!({"path": other.join("tone.wav")}));
    outside.ok("save_project", json!({"path": other.join("Song")}));
    let project = std::fs::read_to_string(other.join("Song.studiocraft")).unwrap();
    // Audio already in the project folder is referenced in place, as `tone.wav`.
    let escaped = project.replace("\"tone.wav\"", "\"../other/tone.wav\"");
    assert_ne!(escaped, project);
    std::fs::write(read.join("Song.studiocraft"), escaped).unwrap();
    let e = a
        .call(
            "open_project",
            json!({"path": read.join("Song.studiocraft")}),
        )
        .unwrap_err();
    assert!(e.contains("outside the read root"), "{e}");
}

#[test]
fn hostile_arguments_come_back_as_tool_errors() {
    let dir = temp("hostile");
    let mut a = Agent::new(&dir, &dir);
    a.ok("add_track", json!({"kind": "midi"}));
    let part = a.ok(
        "create_part",
        json!({"track": 0, "start": 0, "length": 3840}),
    )["id"]
        .clone();
    for (tool, args) in [
        ("add_track", json!({"kind": "tempo"})),
        ("create_part", json!({"track": 5, "start": 0, "length": 1})),
        (
            "create_part",
            json!({"track": 0, "start": "0.0", "length": 1}),
        ),
        ("create_part", json!({"track": 0, "start": -4, "length": 1})),
        (
            "add_notes",
            json!({"event": part, "notes": [{"pitch": 200, "start": 0, "length": 1}]}),
        ),
        (
            "add_notes",
            json!({"event": part, "notes": [{"pitch": 60, "start": 0, "length": 0}]}),
        ),
        ("add_notes", json!({"event": 999, "notes": []})),
        ("split_event", json!({"event": part, "at": 999_999})),
        ("set_tempo", json!({"numerator": 4})),
        ("move_event", json!({"event": part, "track": "nobody"})),
        (
            "open_project",
            json!({"path": dir.join("missing.studiocraft")}),
        ),
        ("no_such_tool", json!({})),
    ] {
        assert!(
            a.call(tool, args.clone()).is_err(),
            "{tool} {args} should fail"
        );
    }
    // Nothing above changed the project: the part is still empty and in place.
    let p = a.ok("inspect_project", json!({}));
    assert_eq!(p["tracks"][0]["events"][0]["notes"], 0);
    assert_eq!(p["tracks"].as_array().unwrap().len(), 1);
}

mod connected {
    //! `--connect`: the tools run in an app's window, reached through its control channel.

    use std::net::TcpListener;
    use std::sync::Arc;
    use std::sync::atomic::{AtomicBool, Ordering};
    use std::sync::mpsc;
    use std::time::Duration;

    use studiocraft_automation::control::Control;
    use studiocraft_project::Project;
    use studiocraft_ui::ProjectWindow;

    use super::*;

    const TOKEN: &str = "0123456789abcdef0123456789abcdef0123456789abcdef";

    /// An app with a headless window serving the control channel on its own thread, as the
    /// desktop app does between frames. `setup` runs on the window first. Stops when dropped.
    struct App {
        addr: String,
        stop: Arc<AtomicBool>,
        thread: Option<std::thread::JoinHandle<()>>,
    }

    impl App {
        fn start(setup: impl FnOnce(&mut ProjectWindow) + Send + 'static) -> Self {
            Self::run(setup, false)
        }

        /// As [`App::start`], drawing the window into an egui context each frame, as the
        /// desktop app does (`ui_inspect`, `ui_click`).
        fn with_window() -> Self {
            Self::run(|_| {}, true)
        }

        fn run(setup: impl FnOnce(&mut ProjectWindow) + Send + 'static, window_ui: bool) -> Self {
            let stop = Arc::new(AtomicBool::new(false));
            let (tx, rx) = mpsc::channel();
            let flag = Arc::clone(&stop);
            let thread = std::thread::spawn(move || {
                let mut window = ProjectWindow::new(Project::default(), None, String::new());
                // Labels as the tests name them, whatever the system's language and the
                // settings say (the language is per thread).
                studiocraft_ui::i18n::set_language(studiocraft_ui::i18n::Language::German);
                setup(&mut window);
                let listener = TcpListener::bind("127.0.0.1:0").unwrap();
                let mut control = Control::serve(listener, TOKEN.into(), || {}).unwrap();
                tx.send(control.port()).unwrap();
                let ctx = egui::Context::default();
                while !flag.load(Ordering::Relaxed) {
                    if window_ui {
                        let input = egui::RawInput {
                            screen_rect: Some(egui::Rect::from_min_size(
                                egui::Pos2::ZERO,
                                egui::vec2(1280.0, 800.0),
                            )),
                            ..Default::default()
                        };
                        let mut out = ctx.run_ui(input, |ui| {
                            control.poll(&mut window, Some(ui.ctx()));
                            window.ui(ui);
                        });
                        // Nothing renders here: drop the textures on purpose.
                        out.textures_delta.clear();
                    } else {
                        control.poll(&mut window, None);
                    }
                    std::thread::sleep(Duration::from_millis(2));
                }
            });
            let port = rx.recv().unwrap();
            Self {
                addr: format!("127.0.0.1:{port}"),
                stop,
                thread: Some(thread),
            }
        }
    }

    impl Drop for App {
        fn drop(&mut self) {
            self.stop.store(true, Ordering::Relaxed);
            if let Some(t) = self.thread.take() {
                let _ = t.join();
            }
        }
    }

    #[test]
    fn the_tools_edit_the_apps_window() {
        let dir = temp("connect-edit");
        let app = App::start(|_| {});
        let mut a = Agent::connected(&dir, &dir, &app.addr, TOKEN);

        let names: Vec<String> = a.request("tools/list", json!({}))["tools"]
            .as_array()
            .unwrap()
            .iter()
            .map(|t| t["name"].as_str().unwrap().to_owned())
            .collect();
        assert!(names.iter().any(|n| n == "ui_screenshot"));
        assert!(names.iter().any(|n| n == "transport"));

        a.ok("add_track", json!({"kind": "midi", "name": "Bass"}));
        let part = a.ok(
            "create_part",
            json!({"track": "Bass", "start": 0, "length": "1.0.0.0"}),
        );
        a.ok(
            "add_notes",
            json!({"event": part["id"], "notes": [{"pitch": 36, "start": 0, "length": 480}]}),
        );
        let p = a.ok("inspect_project", json!({}));
        assert_eq!(p["tracks"][0]["name"], "Bass");
        assert_eq!(p["tracks"][0]["events"][0]["notes"], 1);
        a.ok("undo", json!({}));
        assert_eq!(
            a.ok("inspect_project", json!({}))["tracks"][0]["events"][0]["notes"],
            0
        );

        // The window's own work is not thrown away unasked.
        let e = a.call("new_project", json!({})).unwrap_err();
        assert!(e.contains("unsaved changes"), "{e}");
        a.ok("new_project", json!({"discard_changes": true}));

        // Without an audio device and a window, playing and screenshots are refused, not faked.
        let e = a.call("transport", json!({"action": "play"})).unwrap_err();
        assert!(e.contains("no audio engine"), "{e}");
        assert!(a.call("ui_screenshot", json!({})).is_err());
        a.ok("transport", json!({"action": "locate", "at": "3.1.1.0"}));
        assert_eq!(
            a.ok("inspect_project", json!({}))["cursor_position"],
            "3.1.1.000"
        );
    }

    #[test]
    fn paths_are_confined_before_they_reach_the_app() {
        let dir = temp("connect-confined");
        let (granted, other) = (dir.join("granted"), dir.join("other"));
        std::fs::create_dir_all(&granted).unwrap();
        std::fs::create_dir_all(&other).unwrap();
        // The person saved the open project outside the agent's folders.
        let saved = other.join("Theirs.studiocraft");
        let in_app = saved.clone();
        let app = App::start(move |w| {
            w.project
                .add_track(studiocraft_project::TrackKind::Midi, "Keys");
            w.save_as(&in_app).unwrap();
        });
        let mut a = Agent::connected(&granted, &granted, &app.addr, TOKEN);

        let e = a.call("save_project", json!({})).unwrap_err();
        assert!(e.contains("outside the write root"), "{e}");
        let e = a
            .call("export_midi", json!({"path": other.join("x.mid")}))
            .unwrap_err();
        assert!(e.contains("outside the write root"), "{e}");
        assert!(!other.join("x.mid").exists());

        // Inside the granted folder it works, with the path made absolute here.
        a.ok("export_midi", json!({"path": granted.join("x.mid")}));
        assert!(granted.join("x.mid").exists());
    }

    #[test]
    fn a_wrong_token_is_refused() {
        let dir = temp("connect-token");
        let app = App::start(|_| {});
        let mut a = Agent::connected(
            &dir,
            &dir,
            &app.addr,
            "not-the-token-not-the-token-not-the-token",
        );
        assert!(a.call("inspect_project", json!({})).is_err());
    }

    #[test]
    fn no_app_running_is_an_error_the_agent_can_read() {
        let dir = temp("connect-none");
        let port = TcpListener::bind("127.0.0.1:0")
            .unwrap()
            .local_addr()
            .unwrap()
            .port();
        let mut a = Agent::connected(&dir, &dir, &format!("127.0.0.1:{port}"), TOKEN);
        let e = a.call("inspect_project", json!({})).unwrap_err();
        assert!(!e.is_empty());
    }

    /// The widgets `ui_inspect` lists for `label`.
    fn widgets(a: &mut Agent, label: &str) -> Vec<Value> {
        a.ok("ui_inspect", json!({"label": label}))["widgets"]
            .as_array()
            .unwrap()
            .clone()
    }

    #[test]
    fn the_agent_finds_and_clicks_widgets_in_the_window() {
        let dir = temp("connect-click");
        let app = App::with_window();
        let mut a = Agent::connected(&dir, &dir, &app.addr, TOKEN);

        let all = a.ok("ui_inspect", json!({}));
        assert_eq!(all["window"], json!([1280.0, 800.0]));
        assert_eq!(all["truncated"], false);
        let menu = widgets(&mut a, "Projekt");
        let button = menu
            .iter()
            .find(|w| w["label"] == "Projekt" && w["clickable"] == true)
            .unwrap_or_else(|| panic!("no Projekt menu in {menu:?}"));
        assert_eq!(button["rect"].as_array().unwrap().len(), 4);
        // Nothing of the menu is open yet.
        assert!(widgets(&mut a, "Gruppen-Spur hinzufügen").is_empty());

        // Open the menu by id, then click an entry by its label: a group track appears.
        let clicked = a.ok("ui_click", json!({"id": button["id"]}));
        assert_eq!(clicked["clicked"]["label"], "Projekt");
        a.ok("ui_click", json!({"label": "Gruppen-Spur hinzufügen"}));
        let tracks = a.ok("inspect_project", json!({}))["tracks"].clone();
        assert!(
            tracks
                .as_array()
                .unwrap()
                .iter()
                .any(|t| t["kind"] == "group"),
            "{tracks}"
        );

        // A click at a point: on an empty spot of the window it lands, without effect.
        let at = a.ok("ui_click", json!({"x": 1270, "y": 790}));
        assert_eq!(at["clicked"]["at"], json!([1270.0, 790.0]));

        // Wrong targets are errors the agent can read.
        for bad in [
            json!({}),
            json!({"label": "Gibt es nicht"}),
            json!({"id": 1, "label": "Projekt"}),
            json!({"x": 10}),
            json!({"x": -1, "y": 10}),
            json!({"label": "Projekt", "button": "fourth"}),
            json!({"label": "Projekt", "modifiers": ["hyper"]}),
        ] {
            assert!(a.call("ui_click", bad.clone()).is_err(), "{bad}");
        }
        // The tree is built again after a click and stays readable.
        assert!(!widgets(&mut a, "Projekt").is_empty());
    }

    #[test]
    fn without_a_window_inspect_and_click_are_refused() {
        let dir = temp("connect-nowindow");
        let app = App::start(|_| {});
        let mut a = Agent::connected(&dir, &dir, &app.addr, TOKEN);
        assert!(a.call("ui_inspect", json!({})).is_err());
        assert!(a.call("ui_click", json!({"x": 1, "y": 1})).is_err());
    }
}
