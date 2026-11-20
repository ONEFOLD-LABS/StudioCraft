//! StudioCraft for AI agents: an MCP server (Model Context Protocol, JSON-RPC over stdio) that
//! drives a headless project window, so an agent can build, edit, save and render projects the
//! way the desktop app does: the same undo history, import, save and mixdown code.
//!
//! The protocol itself comes from `craft-mcp-server` (craft-libs). This crate defines the tools
//! ([`Studio`]) and confines file access to the folders the operator grants ([`Roots`]).
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

mod roots;

use std::path::{Path, PathBuf};

use craft_mcp_server::{
    Contents, Effect, Info, Options, Server, Tools, object, string, text_result, tool,
};
use serde_json::{Value, json};
use studiocraft_project::{
    EventId, MAX_DB, MIN_DB, Note, Project, TICKS_PER_QUARTER, TimeSignature, TrackKind,
    file::EXTENSION, smf,
};
use studiocraft_ui::ProjectWindow;

pub use roots::Roots;

/// Longest request line the server reads (16 MiB): room for large note lists.
const MAX_LINE: usize = 16 << 20;
/// Most notes one `add_notes` call takes.
const MAX_NOTES_PER_CALL: usize = 20_000;
/// Most events one `delete_events` call takes.
const MAX_IDS_PER_CALL: usize = 10_000;
/// URI of the project resource.
const PROJECT_URI: &str = "studiocraft://project";

/// The MCP server over a fresh, empty project, confined to `roots`.
pub fn server(roots: Roots) -> Server<Studio> {
    Server::new(
        Info {
            name: "studiocraft",
            title: "StudioCraft",
            version: env!("CARGO_PKG_VERSION"),
            max_line: MAX_LINE,
        },
        Studio::new(roots),
    )
    .with_options(Options {
        tool_errors_as_results: true,
        ..Options::default()
    })
}

/// The tools: a headless project window plus the file roots.
pub struct Studio {
    window: ProjectWindow,
    roots: Roots,
}

impl Studio {
    pub fn new(roots: Roots) -> Self {
        let mut window = ProjectWindow::new(Project::default(), None, String::new());
        window.clear_history();
        Self { window, roots }
    }

    /// The project window the tools act on (for tests and embedding).
    pub fn window(&self) -> &ProjectWindow {
        &self.window
    }

    fn run(&mut self, name: &str, a: &Value) -> Result<Value, String> {
        match name {
            "inspect_project" => Ok(self.inspect()),
            "get_notes" => self.get_notes(a),
            "new_project" => self.new_project(a),
            "open_project" => self.open_project(a),
            "save_project" => self.save_project(a),
            "import_audio" => self.import_audio(a),
            "import_midi" => self.import_midi(a),
            "export_mixdown" => self.export_mixdown(a),
            "export_midi" => self.export_midi(a),
            "add_track" => self.add_track(a),
            "set_track" => self.set_track(a),
            "create_part" => self.create_part(a),
            "add_notes" => self.add_notes(a),
            "move_event" => self.move_event(a),
            "split_event" => self.split_event(a),
            "delete_events" => self.delete_events(a),
            "set_tempo" => self.set_tempo(a),
            "add_marker" => self.add_marker(a),
            "set_locators" => self.set_locators(a),
            "undo" => {
                self.window.undo();
                Ok(self.history())
            }
            "redo" => {
                self.window.redo();
                Ok(self.history())
            }
            _ => Err(format!("unknown tool `{name}`")),
        }
    }

    // ----- reading -----

    fn inspect(&self) -> Value {
        let w = &self.window;
        let p = &w.project;
        let tempo = &p.tempo;
        let (left, right) = w.locators();
        let (undo, redo) = w.history_depth();
        let tracks: Vec<Value> = p
            .tracks
            .iter()
            .enumerate()
            .map(|(i, t)| {
                let events: Vec<Value> = t
                    .events
                    .iter()
                    .map(|e| {
                        let mut v = json!({
                            "id": e.id,
                            "name": e.name,
                            "start": e.start.0,
                            "start_position": tempo.format_position(e.start),
                            "length": e.length,
                            "length_text": tempo.format_length(e.length),
                            "muted": e.muted,
                        });
                        match e.clip {
                            Some(c) => {
                                v["type"] = json!("audio");
                                v["clip"] = json!(p.pool.get(c as usize).map(|c| c.name.clone()));
                            }
                            None => {
                                v["type"] = json!("part");
                                v["notes"] = json!(e.notes.len());
                            }
                        }
                        v
                    })
                    .collect();
                json!({
                    "index": i,
                    "id": t.id,
                    "name": t.name,
                    "kind": kind_name(t.kind),
                    "volume_db": t.volume_db,
                    "pan": t.pan,
                    "mute": t.mute,
                    "solo": t.solo,
                    "output": t.output.and_then(|o| p.track_index(o)),
                    "events": events,
                })
            })
            .collect();
        let markers: Vec<Value> = p
            .markers
            .iter()
            .map(|m| json!({"id": m.id, "name": m.name, "start": m.start, "end": m.end}))
            .collect();
        json!({
            "path": w.project_path().map(|p| p.display().to_string()),
            "modified": w.is_modified(),
            "undo_steps": undo,
            "redo_steps": redo,
            "sample_rate": w.sample_rate(),
            "ticks_per_quarter": TICKS_PER_QUARTER,
            "tempo": {
                "bpm": tempo.bpm,
                "signature": format!("{}/{}", tempo.signature.numerator, tempo.signature.denominator),
                "varies": tempo.varies(),
            },
            "length": p.length(),
            "length_text": tempo.format_length(p.length()),
            "locators": {"left": left, "right": right},
            "markers": markers,
            "tracks": tracks,
        })
    }

    fn get_notes(&self, a: &Value) -> Result<Value, String> {
        let id = event_id(a, "event")?;
        let e = self.window.project.event(id).ok_or_else(|| no_event(id))?;
        if e.clip.is_some() {
            return Err(format!("event {id} is audio and has no notes"));
        }
        let notes: Vec<Value> = e
            .notes
            .iter()
            .map(|n| json!({"pitch": n.pitch, "start": n.start, "length": n.length, "velocity": n.velocity}))
            .collect();
        Ok(json!({"event": id, "part_start": e.start.0, "part_length": e.length, "notes": notes}))
    }

    fn history(&self) -> Value {
        let (undo, redo) = self.window.history_depth();
        json!({"undo_steps": undo, "redo_steps": redo})
    }

    // ----- files -----

    fn new_project(&mut self, a: &Value) -> Result<Value, String> {
        let bpm = a.get("bpm").and_then(Value::as_f64);
        self.window.new_project();
        if let Some(bpm) = bpm {
            self.window.project.set_tempo(bpm);
            self.window.clear_history();
        }
        Ok(self.inspect())
    }

    fn open_project(&mut self, a: &Value) -> Result<Value, String> {
        let path = self.roots.readable(Path::new(str_arg(a, "path")?))?;
        // The audio a project names is read too: refuse a project whose files lie outside the
        // read roots instead of opening it partly.
        let text =
            std::fs::read_to_string(&path).map_err(|e| format!("{}: {e}", path.display()))?;
        let project = studiocraft_project::file::from_json(&text)
            .map_err(|e| format!("{}: {e}", path.display()))?;
        let dir = path.parent().unwrap_or(Path::new("."));
        for clip in &project.pool {
            if let Some(rel) = &clip.path {
                let abs = dir.join(rel);
                if abs.exists() {
                    self.roots.readable(&abs)?;
                }
            }
        }
        self.window.open(&path)?;
        let mut v = self.inspect();
        v["message"] = json!(self.window.message());
        Ok(v)
    }

    fn save_project(&mut self, a: &Value) -> Result<Value, String> {
        let target = match a.get("path").and_then(Value::as_str) {
            Some(p) => with_extension(Path::new(p)),
            None => self
                .window
                .project_path()
                .map(Path::to_path_buf)
                .ok_or("the project has no file yet: pass `path`")?,
        };
        let target = self.roots.writable(&target)?;
        let same = self.window.project_path() == Some(target.as_path());
        if target.exists() && !same && !bool_arg(a, "replace") {
            return Err(format!(
                "{} already exists: pass `replace: true` to overwrite it",
                target.display()
            ));
        }
        self.window.save_as(&target)?;
        Ok(json!({"saved": target.display().to_string()}))
    }

    fn import_audio(&mut self, a: &Value) -> Result<Value, String> {
        let path = self.roots.readable(Path::new(str_arg(a, "path")?))?;
        let target = match a.get("track").filter(|v| !v.is_null()) {
            Some(t) => {
                let t = self.track(t)?;
                if self.window.project.tracks[t].kind != TrackKind::Audio {
                    return Err(format!("track {t} is not an audio track"));
                }
                Some((t, self.position(a.get("start"))?.unwrap_or(0)))
            }
            // A target track that does not exist makes `import_audio` add a new audio track
            // there, at `start`.
            None => self.position(a.get("start"))?.map(|at| (usize::MAX, at)),
        };
        let id = self.window.edit(|w| w.import_audio(&path, target))?;
        Ok(self.event_summary(id))
    }

    fn import_midi(&mut self, a: &Value) -> Result<Value, String> {
        let path = self.roots.readable(Path::new(str_arg(a, "path")?))?;
        let size = std::fs::metadata(&path)
            .map_err(|e| format!("{}: {e}", path.display()))?
            .len();
        if size > smf::MAX_FILE_BYTES as u64 {
            return Err(format!(
                "{}: file too large for a MIDI file",
                path.display()
            ));
        }
        let bytes = std::fs::read(&path).map_err(|e| format!("{}: {e}", path.display()))?;
        let name = path
            .file_stem()
            .map_or("MIDI".into(), |s| s.to_string_lossy().into_owned());
        let before = self.window.project.tracks.len();
        self.window.edit(|w| w.import_midi_bytes(&name, &bytes))?;
        let added: Vec<usize> = (before..self.window.project.tracks.len()).collect();
        Ok(json!({"added_tracks": added, "tempo_bpm": self.window.project.tempo.bpm}))
    }

    fn export_mixdown(&mut self, a: &Value) -> Result<Value, String> {
        let path = PathBuf::from(str_arg(a, "path")?);
        let ext = path
            .extension()
            .and_then(|e| e.to_str())
            .map(str::to_ascii_lowercase);
        if !matches!(ext.as_deref(), Some("wav" | "flac" | "aif" | "aiff")) {
            return Err("the mixdown path must end in .wav, .flac, .aif or .aiff".into());
        }
        let path = self.writable_new(&path, a)?;
        if self.window.project.length() == 0 {
            return Err("the project is empty: nothing to render".into());
        }
        let (from, to) = self.window.export_range();
        let peak = self.window.export_mixdown(&path)?;
        let peak_db = if peak > 0.0 {
            json!((20.0 * f64::from(peak).log10() * 10.0).round() / 10.0)
        } else {
            Value::Null
        };
        Ok(json!({
            "written": path.display().to_string(),
            "range": {"start": from, "end": to},
            "peak_dbfs": peak_db,
            "clipped": peak > 1.0,
            "note": "Rendered without plug-ins and external MIDI devices: MIDI parts are silent unless an instrument renders them in the desktop app.",
        }))
    }

    fn export_midi(&mut self, a: &Value) -> Result<Value, String> {
        let path = self.writable_new(Path::new(str_arg(a, "path")?), a)?;
        let (_, bytes) = self.window.midi_file_bytes()?;
        std::fs::write(&path, bytes).map_err(|e| format!("{}: {e}", path.display()))?;
        Ok(json!({"written": path.display().to_string()}))
    }

    /// `path` checked for writing; an existing file only with `replace: true`.
    fn writable_new(&self, path: &Path, a: &Value) -> Result<PathBuf, String> {
        let path = self.roots.writable(path)?;
        if path.exists() && !bool_arg(a, "replace") {
            return Err(format!(
                "{} already exists: pass `replace: true` to overwrite it",
                path.display()
            ));
        }
        Ok(path)
    }

    // ----- editing -----

    fn add_track(&mut self, a: &Value) -> Result<Value, String> {
        let kind_text = str_arg(a, "kind")?;
        let kind = parse_kind(kind_text)?;
        let name = a
            .get("name")
            .and_then(Value::as_str)
            .map_or_else(|| kind.label().to_owned(), str::to_owned);
        let index = self.window.edit(|w| w.project.add_track(kind, name));
        let t = &self.window.project.tracks[index];
        Ok(json!({"index": index, "id": t.id, "name": t.name, "kind": kind_name(t.kind)}))
    }

    fn set_track(&mut self, a: &Value) -> Result<Value, String> {
        let t = self.track(a.get("track").unwrap_or(&Value::Null))?;
        let name = a.get("name").and_then(Value::as_str).map(str::to_owned);
        let volume = a.get("volume_db").and_then(Value::as_f64);
        let pan = a.get("pan").and_then(Value::as_f64);
        let mute = a.get("mute").and_then(Value::as_bool);
        let solo = a.get("solo").and_then(Value::as_bool);
        self.window.edit(|w| {
            let track = &mut w.project.tracks[t];
            if let Some(n) = name {
                track.name = n;
            }
            if let Some(v) = volume.filter(|v| v.is_finite()) {
                track.volume_db = (v as f32).clamp(MIN_DB, MAX_DB);
            }
            if let Some(p) = pan.filter(|p| p.is_finite()) {
                track.pan = (p as f32).clamp(-1.0, 1.0);
            }
            if let Some(m) = mute {
                track.mute = m;
            }
            if let Some(s) = solo {
                track.solo = s;
            }
        });
        let track = &self.window.project.tracks[t];
        Ok(json!({
            "index": t, "name": track.name, "volume_db": track.volume_db, "pan": track.pan,
            "mute": track.mute, "solo": track.solo,
        }))
    }

    fn create_part(&mut self, a: &Value) -> Result<Value, String> {
        let t = self.track(a.get("track").unwrap_or(&Value::Null))?;
        let kind = self.window.project.tracks[t].kind;
        if !matches!(kind, TrackKind::Midi | TrackKind::Instrument) {
            return Err(format!("track {t} is not a MIDI or instrument track"));
        }
        let start = self.position(a.get("start"))?.ok_or("missing `start`")?;
        let length = self.length(a.get("length"))?.ok_or("missing `length`")?;
        let name = a.get("name").and_then(Value::as_str).map(str::to_owned);
        let id = self
            .window
            .edit(|w| {
                let id = w.project.create_event(t, start, length)?;
                if let (Some(name), Some(e)) = (name, w.project.event_mut(id)) {
                    e.name = name;
                }
                Some(id)
            })
            .ok_or("the part could not be created (length must be above 0)")?;
        Ok(self.event_summary(id))
    }

    fn add_notes(&mut self, a: &Value) -> Result<Value, String> {
        let id = event_id(a, "event")?;
        let e = self.window.project.event(id).ok_or_else(|| no_event(id))?;
        if e.clip.is_some() {
            return Err(format!("event {id} is audio: notes go into MIDI parts"));
        }
        let list = a
            .get("notes")
            .and_then(Value::as_array)
            .ok_or("`notes` must be an array")?;
        if list.len() > MAX_NOTES_PER_CALL {
            return Err(format!("at most {MAX_NOTES_PER_CALL} notes per call"));
        }
        let tempo = self.window.project.tempo.clone();
        let mut notes = Vec::with_capacity(list.len());
        for (i, n) in list.iter().enumerate() {
            let bad = |why: &str| format!("notes[{i}]: {why}");
            let pitch = n
                .get("pitch")
                .and_then(Value::as_u64)
                .filter(|p| *p <= 127)
                .ok_or_else(|| bad("`pitch` must be 0–127 (60 = middle C)"))?;
            let start = match n.get("start") {
                Some(Value::Number(v)) => v.as_i64().ok_or_else(|| bad("`start` must be ticks"))?,
                Some(Value::String(s)) => tempo
                    .parse_length(s)
                    .and_then(|t| i64::try_from(t).ok())
                    .ok_or_else(|| bad("`start` is not a length like `0.2.0.0`"))?,
                _ => return Err(bad("missing `start` (ticks from the part start)")),
            };
            let length = match n.get("length") {
                Some(Value::Number(v)) => v.as_u64(),
                Some(Value::String(s)) => tempo.parse_length(s),
                _ => None,
            }
            .filter(|l| *l > 0)
            .ok_or_else(|| bad("`length` must be ticks above 0 or a length like `0.1.0.0`"))?;
            let velocity = match n.get("velocity") {
                None | Some(Value::Null) => 100,
                Some(v) => v
                    .as_u64()
                    .filter(|v| (1..=127).contains(v))
                    .ok_or_else(|| bad("`velocity` must be 1–127"))?,
            };
            notes.push(Note {
                pitch: pitch as u8,
                velocity: velocity as u8,
                start,
                length,
            });
        }
        let replace = bool_arg(a, "replace");
        let count = notes.len();
        self.window.edit(|w| {
            if let Some(e) = w.project.event_mut(id) {
                if replace {
                    e.notes.clear();
                }
                e.notes.extend(notes);
            }
        });
        let e = self.window.project.event(id).ok_or_else(|| no_event(id))?;
        let outside = e
            .notes
            .iter()
            .filter(|n| n.start < 0 || n.end() > i64::try_from(e.length).unwrap_or(i64::MAX))
            .count();
        Ok(json!({
            "event": id, "added": count, "notes": e.notes.len(), "part_length": e.length,
            "notes_outside_part": outside,
        }))
    }

    fn move_event(&mut self, a: &Value) -> Result<Value, String> {
        let id = event_id(a, "event")?;
        let (cur_track, _) = self.window.project.find(id).ok_or_else(|| no_event(id))?;
        let e = self.window.project.event(id).ok_or_else(|| no_event(id))?;
        let is_audio = e.clip.is_some();
        let start = self.position(a.get("start"))?.unwrap_or(e.start.0);
        let track = match a.get("track").filter(|v| !v.is_null()) {
            Some(t) => self.track(t)?,
            None => cur_track,
        };
        let kind = self.window.project.tracks[track].kind;
        let fits = if is_audio {
            kind == TrackKind::Audio
        } else {
            matches!(kind, TrackKind::Midi | TrackKind::Instrument)
        };
        if !fits {
            return Err(format!(
                "event {id} cannot go on a {} track",
                kind_name(kind)
            ));
        }
        let copy = bool_arg(a, "copy");
        let new = self
            .window
            .edit(|w| w.project.move_event(id, track, start, copy))
            .ok_or("the event could not be moved")?;
        Ok(self.event_summary(new))
    }

    fn split_event(&mut self, a: &Value) -> Result<Value, String> {
        let id = event_id(a, "event")?;
        self.window.project.event(id).ok_or_else(|| no_event(id))?;
        let at = self.position(a.get("at"))?.ok_or("missing `at`")?;
        let right = self
            .window
            .edit(|w| w.project.split(id, at))
            .ok_or("`at` must lie inside the event")?;
        Ok(json!({"left": id, "right": right}))
    }

    fn delete_events(&mut self, a: &Value) -> Result<Value, String> {
        let ids = a
            .get("events")
            .and_then(Value::as_array)
            .ok_or("`events` must be an array of event ids")?;
        if ids.len() > MAX_IDS_PER_CALL {
            return Err(format!("at most {MAX_IDS_PER_CALL} events per call"));
        }
        let ids: Vec<EventId> = ids
            .iter()
            .map(|v| v.as_u64().and_then(|v| EventId::try_from(v).ok()))
            .collect::<Option<_>>()
            .ok_or("`events` must hold event ids")?;
        let deleted = self
            .window
            .edit(|w| ids.iter().filter(|&&id| w.project.delete(id)).count());
        Ok(json!({"deleted": deleted, "not_found": ids.len() - deleted}))
    }

    fn set_tempo(&mut self, a: &Value) -> Result<Value, String> {
        let bpm = a
            .get("bpm")
            .and_then(Value::as_f64)
            .filter(|b| b.is_finite());
        let sig = match (a.get("numerator"), a.get("denominator")) {
            (None, None) => None,
            (n, d) => {
                let n = n.and_then(Value::as_u64).filter(|n| (1..=32).contains(n));
                let d = d
                    .and_then(Value::as_u64)
                    .filter(|d| [1, 2, 4, 8, 16, 32].contains(d));
                match (n, d) {
                    (Some(n), Some(d)) => Some(TimeSignature {
                        numerator: n as u32,
                        denominator: d as u32,
                    }),
                    _ => {
                        return Err(
                            "time signature: `numerator` 1–32 and `denominator` 1, 2, 4, 8, 16 or 32, both together"
                                .into(),
                        );
                    }
                }
            }
        };
        if bpm.is_none() && sig.is_none() {
            return Err("pass `bpm`, or `numerator` and `denominator`, or both".into());
        }
        self.window.edit(|w| {
            if let Some(bpm) = bpm {
                w.project.set_tempo(bpm);
            }
            if let Some(sig) = sig {
                let mut map = w.project.tempo.clone();
                map.set_signature(1, sig);
                w.project.set_tempo_map(map);
            }
        });
        let t = &self.window.project.tempo;
        Ok(json!({
            "bpm": t.bpm,
            "signature": format!("{}/{}", t.signature.numerator, t.signature.denominator),
        }))
    }

    fn add_marker(&mut self, a: &Value) -> Result<Value, String> {
        let at = self.position(a.get("at"))?.ok_or("missing `at`")?;
        let end = self.position(a.get("end"))?;
        let name = a.get("name").and_then(Value::as_str).map(str::to_owned);
        let id = self.window.edit(|w| {
            let id = match end {
                Some(end) => w.project.add_cycle_marker(at, end)?,
                None => w.project.add_marker(at),
            };
            let cycle = end.is_some();
            if let Some(name) = name
                && let Some(m) = w
                    .project
                    .markers
                    .iter_mut()
                    .find(|m| m.id == id && m.is_cycle() == cycle)
            {
                m.name = name;
            }
            Some(id)
        });
        let id = id.ok_or("a cycle marker needs `end` different from `at`")?;
        Ok(json!({"id": id, "cycle": end.is_some()}))
    }

    fn set_locators(&mut self, a: &Value) -> Result<Value, String> {
        let left = self.position(a.get("left"))?.ok_or("missing `left`")?;
        let right = self.position(a.get("right"))?.ok_or("missing `right`")?;
        self.window.set_locators(left, right);
        let (l, r) = self.window.locators();
        Ok(json!({"left": l, "right": r}))
    }

    // ----- helpers -----

    fn event_summary(&self, id: EventId) -> Value {
        let p = &self.window.project;
        let track = p.find(id).map(|(t, _)| t);
        match p.event(id) {
            Some(e) => json!({
                "id": id,
                "track": track,
                "name": e.name,
                "start": e.start.0,
                "start_position": p.tempo.format_position(e.start),
                "length": e.length,
                "length_text": p.tempo.format_length(e.length),
            }),
            None => json!({"id": id}),
        }
    }

    /// A track by index (0-based) or exact name.
    fn track(&self, v: &Value) -> Result<usize, String> {
        let tracks = &self.window.project.tracks;
        match v {
            Value::Number(n) => n
                .as_u64()
                .and_then(|i| usize::try_from(i).ok())
                .filter(|&i| i < tracks.len())
                .ok_or_else(|| format!("no track {n} (there are {})", tracks.len())),
            Value::String(s) => {
                let mut hits = tracks.iter().enumerate().filter(|(_, t)| t.name == *s);
                match (hits.next(), hits.next()) {
                    (Some((i, _)), None) => Ok(i),
                    (Some(_), Some(_)) => {
                        Err(format!("several tracks are named `{s}`: use the index"))
                    }
                    _ => Err(format!("no track named `{s}`")),
                }
            }
            _ => Err("`track` must be an index or a name".into()),
        }
    }

    /// A position: ticks, or text like `5.1.1.0` (bar.beat.sixteenth.tick, counted from 1).
    fn position(&self, v: Option<&Value>) -> Result<Option<u64>, String> {
        match v {
            None | Some(Value::Null) => Ok(None),
            Some(Value::Number(n)) => n
                .as_u64()
                .map(Some)
                .ok_or_else(|| format!("position {n} must be ticks ≥ 0")),
            Some(Value::String(s)) => self
                .window
                .project
                .tempo
                .parse_position(s)
                .map(|p| Some(p.0))
                .ok_or_else(|| format!("`{s}` is not a position like `5.1.1.0` (bar 5, beat 1)")),
            Some(_) => Err("a position is ticks or text like `5.1.1.0`".into()),
        }
    }

    /// A length: ticks, or text like `2.0.0.0` (bars.beats.sixteenths.ticks, counted from 0).
    fn length(&self, v: Option<&Value>) -> Result<Option<u64>, String> {
        match v {
            None | Some(Value::Null) => Ok(None),
            Some(Value::Number(n)) => n
                .as_u64()
                .map(Some)
                .ok_or_else(|| format!("length {n} must be ticks ≥ 0")),
            Some(Value::String(s)) => self
                .window
                .project
                .tempo
                .parse_length(s)
                .map(Some)
                .ok_or_else(|| format!("`{s}` is not a length like `2.0.0.0` (two bars)")),
            Some(_) => Err("a length is ticks or text like `2.0.0.0`".into()),
        }
    }
}

impl Tools for Studio {
    fn instructions(&self) -> String {
        format!(
            "StudioCraft is a digital audio workstation. These tools edit one project in memory \
             (no window, no audio device). Call inspect_project first and after changes to verify. \
             Time: {TICKS_PER_QUARTER} ticks per quarter note. Positions are ticks or text \
             `bar.beat.sixteenth.tick` counted from 1 (`1.1.1.0` is the start, `5.1.1.0` bar 5); \
             lengths are ticks or `bars.beats.sixteenths.ticks` counted from 0 (`1.0.0.0` is one \
             bar). Note positions inside a part are relative to the part start. Tracks are named \
             by index (0-based) or exact name, events by id. Every edit is one undo step (undo, \
             redo). Nothing reaches disk until save_project. File access is confined to the \
             folders the operator granted; a refused path is reported as an error."
        )
    }

    fn list(&mut self) -> Vec<Value> {
        tool_list()
    }

    fn call(&mut self, name: &str, args: &Value) -> Value {
        match self.run(name, args) {
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
        (uri == PROJECT_URI).then(|| Ok(Contents::json(&self.inspect())))
    }
}

fn tool_list() -> Vec<Value> {
    let pos = |d: &str| json!({"anyOf": [{"type": "integer", "minimum": 0}, {"type": "string"}], "description": d});
    let track = || json!({"anyOf": [{"type": "integer", "minimum": 0}, {"type": "string"}], "description": "Track index (0-based) or exact name"});
    let event = || json!({"type": "integer", "minimum": 0, "description": "Event id (from inspect_project)"});
    let flag = |d: &str| json!({"type": "boolean", "description": d});
    let replace = || flag("Overwrite an existing file");
    vec![
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
            "Replace the open project with an empty one (unsaved changes are lost).",
            object(
                json!({"bpm": {"type": "number", "description": "Tempo of the new project"}}),
                &[],
            ),
            Effect::Replaces,
        ),
        tool(
            "open_project",
            "Open project",
            "Open a .studiocraft project file and load its audio (unsaved changes are lost). Must lie inside a read root.",
            object(
                json!({"path": string("Path of the .studiocraft file")}),
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
    ]
}

fn kind_name(kind: TrackKind) -> &'static str {
    match kind {
        TrackKind::Audio => "audio",
        TrackKind::Instrument => "instrument",
        TrackKind::Midi => "midi",
        TrackKind::Group => "group",
        TrackKind::Fx => "fx",
        TrackKind::Folder => "folder",
        TrackKind::Marker => "marker",
        TrackKind::Tempo => "tempo",
        TrackKind::Ruler => "ruler",
    }
}

fn parse_kind(s: &str) -> Result<TrackKind, String> {
    Ok(match s {
        "audio" => TrackKind::Audio,
        "instrument" => TrackKind::Instrument,
        "midi" => TrackKind::Midi,
        "group" => TrackKind::Group,
        "fx" => TrackKind::Fx,
        "folder" => TrackKind::Folder,
        _ => return Err(format!("unknown track kind `{s}`")),
    })
}

fn str_arg<'a>(a: &'a Value, key: &str) -> Result<&'a str, String> {
    a.get(key)
        .and_then(Value::as_str)
        .ok_or_else(|| format!("missing `{key}`"))
}

fn bool_arg(a: &Value, key: &str) -> bool {
    a.get(key).and_then(Value::as_bool).unwrap_or(false)
}

fn event_id(a: &Value, key: &str) -> Result<EventId, String> {
    a.get(key)
        .and_then(Value::as_u64)
        .and_then(|v| EventId::try_from(v).ok())
        .ok_or_else(|| format!("`{key}` must be an event id"))
}

fn no_event(id: EventId) -> String {
    format!("no event {id}")
}

fn with_extension(path: &Path) -> PathBuf {
    if path.extension().is_some_and(|e| e == EXTENSION) {
        path.to_path_buf()
    } else {
        let mut p = path.as_os_str().to_owned();
        p.push(".");
        p.push(EXTENSION);
        PathBuf::from(p)
    }
}
