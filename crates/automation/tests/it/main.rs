//! Drives the MCP server through JSON-RPC lines only, as an agent would.

use std::path::{Path, PathBuf};

use serde_json::{Value, json};
use studiocraft_automation::{Roots, Studio, server};

struct Agent {
    server: craft_mcp_server::Server<Studio>,
    next: u64,
}

impl Agent {
    fn new(read: &Path, write: &Path) -> Self {
        let roots = Roots::new(&[read.to_path_buf()], &[write.to_path_buf()]).unwrap();
        let mut a = Self {
            server: server(roots),
            next: 1,
        };
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
