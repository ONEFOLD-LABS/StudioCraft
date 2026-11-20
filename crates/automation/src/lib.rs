//! StudioCraft for AI agents: an MCP server (Model Context Protocol, JSON-RPC over stdio).
//!
//! It runs in one of two ways. Headless ([`server`]), it drives a project window of its own,
//! without a window on screen or an audio device. With `--connect` ([`connect`]), it drives the
//! running app through the app's control channel ([`control`]), so the person sees every change
//! and the agent can play, stop and take screenshots. Both run the same tool code ([`exec`]) on a
//! `ProjectWindow`, so undo, import, save and mixdown behave as in the app.
//!
//! The protocol comes from `craft-mcp-server` and the control channel from `craft-control`
//! (craft-libs). File access is confined to the folders the operator grants ([`Roots`]) in this
//! process, before a call runs or is sent ([`confine`]).
//!
//! Tool texts and results are English: they are read by agents, not shown in the UI.
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

mod confine;
pub mod control;
mod exec;
mod roots;
mod ui_agent;

use std::path::PathBuf;

use craft_control::client::{Connection, Request};
use craft_mcp_server::{
    Contents, Effect, Info, Options, Server, Tools, object, string, text_result, tool,
};
use serde_json::{Value, json};
use studiocraft_project::{Project, TICKS_PER_QUARTER};
use studiocraft_ui::ProjectWindow;

pub use exec::execute;
pub use roots::Roots;

/// Longest request line the server reads (16 MiB): room for large note lists.
const MAX_LINE: usize = 16 << 20;
/// URI of the project resource.
const PROJECT_URI: &str = "studiocraft://project";

/// The MCP server over a fresh, empty, headless project, confined to `roots`.
pub fn server(roots: Roots) -> Server<Studio> {
    mcp(Studio::headless(roots))
}

/// The MCP server driving the app listening at `addr` (`host:port` on loopback), confined to
/// `roots`. `token`: the app's token, else read from the user's token file. Nothing is
/// connected until the first call, so the app may start later.
pub fn connect(roots: Roots, addr: &str, token: Option<&str>) -> Server<Studio> {
    let conn = Connection::new(control::APP, addr, token).with_document("project");
    mcp(Studio {
        backend: Backend::Remote(Box::new(conn)),
        roots,
    })
}

fn mcp(studio: Studio) -> Server<Studio> {
    Server::new(
        Info {
            name: "studiocraft",
            title: "StudioCraft",
            version: env!("CARGO_PKG_VERSION"),
            max_line: MAX_LINE,
        },
        studio,
    )
    .with_options(Options {
        tool_errors_as_results: true,
        ..Options::default()
    })
}

/// Where the tools run.
enum Backend {
    /// A project window of this process, without a window on screen.
    Local(Box<ProjectWindow>),
    /// The running app's window, through its control channel.
    Remote(Box<Connection>),
}

/// The tools of one MCP session and the folders they may touch.
pub struct Studio {
    backend: Backend,
    roots: Roots,
}

impl Studio {
    /// Tools over a headless project window.
    pub fn headless(roots: Roots) -> Self {
        let mut window = ProjectWindow::new(Project::default(), None, String::new());
        window.clear_history();
        Self {
            backend: Backend::Local(Box::new(window)),
            roots,
        }
    }

    fn connected(&self) -> bool {
        matches!(self.backend, Backend::Remote(_))
    }

    fn run(&mut self, name: &str, args: &Value) -> Result<Value, String> {
        let Self { backend, roots } = self;
        let args = confine::confine(roots, name, args, &mut || backend.current_path())?;
        backend.run(name, &args)
    }
}

impl Backend {
    fn run(&mut self, name: &str, args: &Value) -> Result<Value, String> {
        match self {
            Backend::Local(w) => exec::execute(w, name, args),
            Backend::Remote(conn) => {
                let effect = control::effect(name);
                let request = Request {
                    method: name.to_owned(),
                    params: args.clone(),
                    what: format!("`{name}`"),
                    effect,
                    // An edit is never sent twice: the app may have run it before the connection
                    // was lost.
                    once: effect != craft_control::Effect::ReadsOnly,
                };
                conn.call(request).map_err(String::from)
            }
        }
    }

    /// Where the open project is saved.
    fn current_path(&mut self) -> Result<Option<PathBuf>, String> {
        let v = self.run("inspect_project", &json!({}))?;
        Ok(v["path"].as_str().map(PathBuf::from))
    }
}

impl Tools for Studio {
    fn instructions(&self) -> String {
        let place = if self.connected() {
            "These tools drive the open StudioCraft window: a person may be watching and \
             working in it too. Use ui_screenshot to see the window, ui_inspect to list its \
             widgets, ui_click to click them and transport to play. Do not discard the \
             person's unsaved changes without being asked."
        } else {
            "These tools edit one project in memory (no window, no audio device)."
        };
        format!(
            "StudioCraft is a digital audio workstation. {place} Call inspect_project first and \
             after changes to verify. Time: {TICKS_PER_QUARTER} ticks per quarter note. \
             Positions are ticks or text `bar.beat.sixteenth.tick` counted from 1 (`1.1.1.0` is \
             the start, `5.1.1.0` bar 5); lengths are ticks or `bars.beats.sixteenths.ticks` \
             counted from 0 (`1.0.0.0` is one bar). Note positions inside a part are relative to \
             the part start. Tracks are named by index (0-based) or exact name, events by id. \
             Every edit is one undo step (undo, redo). Nothing reaches disk until save_project. \
             File access is confined to the folders the operator granted; a refused path is \
             reported as an error."
        )
    }

    fn list(&mut self) -> Vec<Value> {
        tool_list(self.connected())
    }

    fn call(&mut self, name: &str, args: &Value) -> Value {
        match self.run(name, args) {
            Ok(v) if name == "ui_screenshot" => json!({"content": [
                {"type": "image", "data": v["data"], "mimeType": v["mime"]},
                {"type": "text", "text": format!("The StudioCraft window, {}×{} pixels.", v["width"], v["height"])},
            ]}),
            Ok(v) => text_result(
                serde_json::to_string_pretty(&v).unwrap_or_else(|_| v.to_string()),
                false,
            ),
            Err(e) => text_result(e, true),
        }
    }

    fn resources(&self) -> Vec<Value> {
        vec![json!({
            "uri": PROJECT_URI,
            "name": "project",
            "description": "The open project as inspect_project returns it",
            "mimeType": "application/json",
        })]
    }

    fn read_resource(&mut self, uri: &str) -> Option<Result<Contents, String>> {
        (uri == PROJECT_URI).then(|| {
            self.backend
                .run("inspect_project", &json!({}))
                .map(|v| Contents::json(&v))
        })
    }
}

/// The names of the tools, as the app's control channel lists them.
pub(crate) fn tool_names(connected: bool) -> Vec<String> {
    tool_list(connected)
        .iter()
        .filter_map(|t| t["name"].as_str().map(str::to_owned))
        .collect()
}

fn tool_list(connected: bool) -> Vec<Value> {
    let pos = |d: &str| json!({"anyOf": [{"type": "integer", "minimum": 0}, {"type": "string"}], "description": d});
    let track = || json!({"anyOf": [{"type": "integer", "minimum": 0}, {"type": "string"}], "description": "Track index (0-based) or exact name"});
    let event = || json!({"type": "integer", "minimum": 0, "description": "Event id (from inspect_project)"});
    let flag = |d: &str| json!({"type": "boolean", "description": d});
    let replace = || flag("Overwrite an existing file");
    let discard = || flag("Replace the open project even if it has unsaved changes");
    let mut tools = vec![
        tool(
            "inspect_project",
            "Inspect project",
            "The whole project: tempo, locators, markers, tracks with their events (ids, positions, lengths), undo depth, file path and unsaved state.",
            object(json!({}), &[]),
            Effect::ReadOnly,
        ),
        tool(
            "get_notes",
            "Get notes",
            "The notes of a MIDI part: pitch (60 = middle C), start relative to the part start and length in ticks, velocity.",
            object(json!({"event": event()}), &["event"]),
            Effect::ReadOnly,
        ),
        tool(
            "new_project",
            "New project",
            "Replace the open project with an empty one. Refused while it has unsaved changes unless `discard_changes` is true.",
            object(
                json!({"bpm": {"type": "number", "description": "Tempo of the new project"}, "discard_changes": discard()}),
                &[],
            ),
            Effect::Replaces,
        ),
        tool(
            "open_project",
            "Open project",
            "Open a .studiocraft project file and load its audio. Refused while the open project has unsaved changes unless `discard_changes` is true. Must lie inside a read root.",
            object(
                json!({"path": string("Path of the .studiocraft file"), "discard_changes": discard()}),
                &["path"],
            ),
            Effect::Files,
        ),
        tool(
            "save_project",
            "Save project",
            "Save the project as <name>.studiocraft with its audio copied into an Audio folder beside it. Without `path`, saves where it was opened or last saved. Must lie inside a write root.",
            object(
                json!({"path": string("Path of the .studiocraft file (extension added if missing)"), "replace": replace()}),
                &[],
            ),
            Effect::Files,
        ),
        tool(
            "import_audio",
            "Import audio",
            "Load an audio file (WAV, AIFF, FLAC, MP3, OGG …) into the pool and place it as an event: on a new audio track, or on `track` at `start`. Must lie inside a read root. Undoable.",
            object(
                json!({"path": string("Audio file"), "track": track(), "start": pos("Position (default: project start)")}),
                &["path"],
            ),
            Effect::Files,
        ),
        tool(
            "import_midi",
            "Import MIDI file",
            "Import a standard MIDI file: one MIDI track per file track and channel. In a project without events it also takes the file's tempo and time signature. Must lie inside a read root. Undoable.",
            object(json!({"path": string("MIDI file (.mid)")}), &["path"]),
            Effect::Files,
        ),
        tool(
            "export_mixdown",
            "Export mixdown",
            "Render the mix between the left and right locator (see inspect_project; set them with set_locators, or the whole project when right is not after left) to a 24-bit WAV, FLAC or AIFF file, chosen by the extension. Returns the peak in dBFS. Plug-ins are not rendered. Must lie inside a write root.",
            object(
                json!({"path": string("Output file (.wav, .flac, .aif, .aiff)"), "replace": replace()}),
                &["path"],
            ),
            Effect::Files,
        ),
        tool(
            "export_midi",
            "Export MIDI file",
            "Write the MIDI and instrument tracks as a standard MIDI file. Must lie inside a write root.",
            object(
                json!({"path": string("Output file (.mid)"), "replace": replace()}),
                &["path"],
            ),
            Effect::Files,
        ),
        tool(
            "add_track",
            "Add track",
            "Add a track at the end of the track list. Undoable.",
            object(
                json!({
                    "kind": {"type": "string", "enum": ["audio", "instrument", "midi", "group", "fx", "folder"], "description": "Track type"},
                    "name": string("Track name (default: the type)"),
                }),
                &["kind"],
            ),
            Effect::Edits,
        ),
        tool(
            "set_track",
            "Set track",
            "Change a track's name, fader (dB, −60 to +6), pan (−1 left to +1 right), mute or solo. Only the given fields change. Undoable.",
            object(
                json!({
                    "track": track(),
                    "name": string("New name"),
                    "volume_db": {"type": "number", "description": "Fader in dB"},
                    "pan": {"type": "number", "description": "−1 to +1"},
                    "mute": flag("Mute"),
                    "solo": flag("Solo"),
                }),
                &["track"],
            ),
            Effect::Edits,
        ),
        tool(
            "create_part",
            "Create MIDI part",
            "Create an empty MIDI part on a MIDI or instrument track. Returns its event id for add_notes. Undoable.",
            object(
                json!({"track": track(), "start": pos("Start position"), "length": pos("Length: ticks or `bars.beats.sixteenths.ticks`, e.g. `4.0.0.0` = four bars"), "name": string("Part name")}),
                &["track", "start", "length"],
            ),
            Effect::Edits,
        ),
        tool(
            "add_notes",
            "Add notes",
            "Add notes to a MIDI part. Each note: pitch 0–127 (60 = middle C), start relative to the part start and length (ticks or `bars.beats.sixteenths.ticks`), velocity 1–127 (default 100). With `replace`, the part's notes are replaced. One undo step.",
            object(
                json!({
                    "event": event(),
                    "notes": {
                        "type": "array",
                        "items": {
                            "type": "object",
                            "properties": {
                                "pitch": {"type": "integer", "minimum": 0, "maximum": 127},
                                "start": {"anyOf": [{"type": "integer"}, {"type": "string"}]},
                                "length": {"anyOf": [{"type": "integer", "minimum": 1}, {"type": "string"}]},
                                "velocity": {"type": "integer", "minimum": 1, "maximum": 127},
                            },
                            "required": ["pitch", "start", "length"],
                        },
                    },
                    "replace": flag("Replace the part's notes instead of adding"),
                }),
                &["event", "notes"],
            ),
            Effect::Edits,
        ),
        tool(
            "move_event",
            "Move event",
            "Move (or with `copy`, copy) an event to another position and/or a track of the same type. Undoable.",
            object(
                json!({"event": event(), "start": pos("New start position"), "track": track(), "copy": flag("Copy instead of move")}),
                &["event"],
            ),
            Effect::Edits,
        ),
        tool(
            "split_event",
            "Split event",
            "Split an event at a position inside it. Returns the ids of both halves. Undoable.",
            object(
                json!({"event": event(), "at": pos("Split position")}),
                &["event", "at"],
            ),
            Effect::Edits,
        ),
        tool(
            "delete_events",
            "Delete events",
            "Delete events by id. One undo step.",
            object(
                json!({"events": {"type": "array", "items": {"type": "integer", "minimum": 0}, "description": "Event ids"}}),
                &["events"],
            ),
            Effect::Edits,
        ),
        tool(
            "set_tempo",
            "Set tempo and time signature",
            "Set the project tempo (BPM, 20–300) and/or the time signature. MIDI parts keep their bar positions, audio events their time. Undoable.",
            object(
                json!({
                    "bpm": {"type": "number", "description": "Beats per minute"},
                    "numerator": {"type": "integer", "minimum": 1, "maximum": 32},
                    "denominator": {"type": "integer", "enum": [1, 2, 4, 8, 16, 32]},
                }),
                &[],
            ),
            Effect::Edits,
        ),
        tool(
            "add_marker",
            "Add marker",
            "Add a position marker at `at`, or a cycle marker from `at` to `end`. Undoable.",
            object(
                json!({"at": pos("Marker position"), "end": pos("End of a cycle marker"), "name": string("Marker name")}),
                &["at"],
            ),
            Effect::Edits,
        ),
        tool(
            "set_locators",
            "Set locators",
            "Set the left and right locators (the range export_mixdown renders).",
            object(
                json!({"left": pos("Left locator"), "right": pos("Right locator")}),
                &["left", "right"],
            ),
            Effect::Edits,
        ),
        tool(
            "undo",
            "Undo",
            "Undo the last edit.",
            object(json!({}), &[]),
            Effect::Edits,
        ),
        tool(
            "redo",
            "Redo",
            "Redo the last undone edit.",
            object(json!({}), &[]),
            Effect::Edits,
        ),
    ];
    if connected {
        tools.push(tool(
            "transport",
            "Transport",
            "Play, stop or move the cursor of the running app. `play` starts at `at` when given, else at the cursor.",
            object(
                json!({
                    "action": {"type": "string", "enum": ["play", "stop", "locate"], "description": "What to do"},
                    "at": pos("Position for play or locate"),
                }),
                &["action"],
            ),
            Effect::Edits,
        ));
        tools.push(tool(
            "ui_screenshot",
            "Screenshot",
            "A PNG of the app window as it looks now: check what the person sees after a change.",
            object(json!({}), &[]),
            Effect::ReadOnly,
        ));
        let text = |d: &str| string(d);
        tools.push(tool(
            "ui_inspect",
            "Inspect window",
            "The widgets of the app window as screen readers see them, in reading order: id, role (Button, CheckBox, MenuItem, TextInput …), label, value, rect [x, y, width, height] in points from the top left, and clickable, disabled, checked, expanded, focused where they apply. Only widgets with a label, a value or a click action are listed. Labels follow the app's language. Use it before ui_click.",
            object(
                json!({
                    "label": text("Only widgets whose label or value contains this text (case-insensitive)"),
                    "role": text("Only widgets of this role, e.g. Button"),
                }),
                &[],
            ),
            Effect::ReadOnly,
        ));
        tools.push(tool(
            "ui_click",
            "Click",
            "Click in the app window with the mouse, as the person would: the widget `id` from ui_inspect, the widget with exactly this `label` as ui_inspect lists it, shortcut text included (with `role` or `index` when several share it), or the point `x`, `y` in points. Returns what was clicked; the result shows in the next ui_inspect or ui_screenshot. Prefer the project tools for edits they cover.",
            object(
                json!({
                    "id": {"type": "integer", "minimum": 0, "description": "Widget id from ui_inspect"},
                    "label": text("Exact label of the widget"),
                    "role": text("Role, when several widgets share the label"),
                    "index": {"type": "integer", "minimum": 0, "description": "Which of several widgets with the label (0-based, reading order)"},
                    "x": {"type": "number", "minimum": 0, "description": "Point from the left, in points"},
                    "y": {"type": "number", "minimum": 0, "description": "Point from the top, in points"},
                    "button": {"type": "string", "enum": ["left", "right", "middle"], "description": "Mouse button (default left)"},
                    "double": flag("Double-click"),
                    "modifiers": {"type": "array", "items": {"type": "string", "enum": ["shift", "ctrl", "alt", "command"]}, "description": "Keys held (command: Ctrl, or ⌘ on macOS)"},
                }),
                &[],
            ),
            Effect::Edits,
        ));
    }
    tools
}
