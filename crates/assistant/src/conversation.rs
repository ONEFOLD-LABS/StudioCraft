//! One chat with the model: the messages it sees (OpenAI form), what the panel shows, and the
//! tool calls in between. No network and no drawing here: the panel sends
//! [`Conversation::messages`] and hands the reply to [`Conversation::reply`], which runs the
//! tools on the project window and says whether to ask the model again.

use std::path::Path;

use serde_json::{Value, json};
use studiocraft_automation::{Roots, assistant_tools, instructions, run_confined, uses_files};
use studiocraft_ui::ProjectWindow;
use studiocraft_ui::i18n::{tr, trf};

use crate::client::shorten;

/// Most rounds of tool calls for one message of the person.
pub const MAX_STEPS: usize = 24;
/// Most tool calls one reply may ask for.
const MAX_CALLS: usize = 32;
/// Longest tool result the model gets back (characters).
const MAX_RESULT: usize = 30_000;
/// Messages kept for the model before the oldest exchanges are dropped.
const MAX_MESSAGES: usize = 400;
/// Longest text of the person or the model kept (characters).
const MAX_TEXT: usize = 20_000;
/// Largest history sent to the model (bytes of JSON, about 100 000 tokens).
pub const MAX_HISTORY: usize = 400_000;
/// When the history is too large, older tool results are cut to this many characters.
const SHORT_RESULT: usize = 2_000;

/// `text` cut to `max` characters with a note for the model, never silently.
fn cut_note(text: &str, max: usize) -> String {
    let total = text.chars().count();
    if total <= max {
        return text.to_owned();
    }
    format!(
        "{} [truncated: {max} of {total} characters]",
        shorten(text, max)
    )
}

/// Size of a message as it goes to the model.
fn size(m: &Value) -> usize {
    m.to_string().len()
}

/// A line in the panel.
#[derive(Clone, Debug, PartialEq)]
pub enum Entry {
    /// What the person wrote.
    User(String),
    /// What the model answered.
    Model(String),
    /// A tool the model ran: name, arguments in short, and whether it worked (with the error).
    Tool {
        name: String,
        args: String,
        error: Option<String>,
    },
    /// The request failed (network, endpoint, limits).
    Error(String),
}

/// What the panel does next.
#[derive(Debug, PartialEq, Eq)]
pub enum Next {
    /// Send [`Conversation::messages`] again: the model wants to see tool results.
    Ask,
    /// The model is done; wait for the person.
    Done,
}

pub struct Conversation {
    messages: Vec<Value>,
    entries: Vec<Entry>,
    steps: usize,
}

impl Default for Conversation {
    fn default() -> Self {
        Self::new()
    }
}

impl Conversation {
    pub fn new() -> Self {
        let system = format!(
            "You are the assistant inside StudioCraft, talking with the person using it. \
             Answer briefly in the language they write in. Change the project with the tools \
             when they ask for it; explain what you did in one or two sentences. Ask before \
             deleting things they did not name. Files can only be read and written inside the \
             project's folder, and only once the project is saved. {}",
            instructions(true)
        );
        Self {
            messages: vec![json!({"role": "system", "content": system})],
            entries: Vec::new(),
            steps: 0,
        }
    }

    pub fn entries(&self) -> &[Entry] {
        &self.entries
    }

    /// The messages for the next request.
    pub fn messages(&self) -> &[Value] {
        &self.messages
    }

    /// The tools in the form chat completions expect.
    pub fn tools() -> Vec<Value> {
        assistant_tools()
            .into_iter()
            .map(|t| {
                json!({"type": "function", "function": {
                    "name": t["name"],
                    "description": t["description"],
                    "parameters": t["inputSchema"],
                }})
            })
            .collect()
    }

    /// The person wrote `text`.
    pub fn user(&mut self, text: &str) {
        let text = shorten(text.trim(), MAX_TEXT);
        self.entries.push(Entry::User(text.clone()));
        self.messages.push(json!({"role": "user", "content": text}));
        self.steps = 0;
        self.trim();
        self.fit();
    }

    /// The request failed with `error`.
    pub fn failed(&mut self, error: &str) {
        self.entries.push(Entry::Error(error.to_owned()));
    }

    /// The model answered `message`: show its text, run the tools it asks for on `window`
    /// (files only inside `folder`, the project's folder) and say whether to ask it again.
    pub fn reply(
        &mut self,
        message: &Value,
        window: &mut ProjectWindow,
        folder: Option<&Path>,
    ) -> Next {
        let text = message["content"].as_str().unwrap_or("").trim();
        let calls: Vec<&Value> = message["tool_calls"]
            .as_array()
            .map(|a| a.iter().take(MAX_CALLS).collect())
            .unwrap_or_default();
        if !text.is_empty() {
            self.entries.push(Entry::Model(shorten(text, MAX_TEXT)));
        }
        // The message as the model must see it again: only the fields it sent that matter.
        let mut kept = json!({"role": "assistant", "content": shorten(text, MAX_TEXT)});
        if !calls.is_empty()
            && let Some(o) = kept.as_object_mut()
        {
            o.insert("tool_calls".into(), json!(calls));
        }
        self.messages.push(kept);
        self.trim();
        if calls.is_empty() {
            if text.is_empty() {
                self.entries
                    .push(Entry::Error(tr("Das Modell hat leer geantwortet.").into()));
            }
            return Next::Done;
        }
        let roots = folder.and_then(|f| Roots::new(&[f.to_path_buf()], &[f.to_path_buf()]).ok());
        for call in calls {
            let id = call["id"].as_str().unwrap_or("").to_owned();
            let name = call["function"]["name"].as_str().unwrap_or("").to_owned();
            let result = parse_args(&call["function"]["arguments"])
                .and_then(|args| run(window, roots.as_ref(), &name, &args).map(|r| (args, r)));
            let (args, content) = match &result {
                Ok((args, value)) => (args.clone(), value.to_string()),
                Err(e) => (
                    parse_args(&call["function"]["arguments"]).unwrap_or(Value::Null),
                    json!({"error": e}).to_string(),
                ),
            };
            self.entries.push(Entry::Tool {
                name: name.clone(),
                args: short_args(&args),
                error: result.err(),
            });
            self.messages.push(json!({
                "role": "tool",
                "tool_call_id": id,
                "name": name,
                "content": cut_note(&content, MAX_RESULT),
            }));
        }
        self.steps += 1;
        self.trim();
        if !self.fit() {
            self.entries.push(Entry::Error(
                tr("Abgebrochen: zu viele Schritte – der Verlauf für das Modell ist zu groß geworden. Schreib weiter, um neu anzusetzen.")
                    .into(),
            ));
            return Next::Done;
        }
        if self.steps >= MAX_STEPS {
            self.entries.push(Entry::Error(trf(
                "Nach {n} Schritten angehalten; schreib weiter, um fortzufahren.",
                &[("n", &MAX_STEPS)],
            )));
            return Next::Done;
        }
        Next::Ask
    }

    /// Size of the history as it goes to the model.
    pub fn history_size(&self) -> usize {
        self.messages.iter().map(size).sum()
    }

    /// Keeps the history within [`MAX_HISTORY`]: first older long tool results are cut (with
    /// a note for the model), then the oldest exchanges dropped. `false` when even the
    /// current exchange alone is too large.
    fn fit(&mut self) -> bool {
        let mut total = self.history_size();
        for m in &mut self.messages {
            if total <= MAX_HISTORY {
                return true;
            }
            if m["role"] != "tool" {
                continue;
            }
            let Some(text) = m["content"].as_str() else {
                continue;
            };
            // Already cut ones (the note adds a few dozen characters) stay as they are.
            if text.chars().count() <= SHORT_RESULT + 64 {
                continue;
            }
            let before = size(m);
            m["content"] = json!(cut_note(text, SHORT_RESULT));
            total = total - before + size(m);
        }
        while total > MAX_HISTORY {
            let next_user = self
                .messages
                .iter()
                .enumerate()
                .skip(2)
                .find(|(_, m)| m["role"] == "user")
                .map(|(i, _)| i);
            let Some(i) = next_user else {
                return false;
            };
            self.messages.drain(1..i);
            total = self.history_size();
        }
        true
    }

    /// Drop the oldest exchanges (from one message of the person to the next) while there are
    /// too many messages; the system message stays.
    fn trim(&mut self) {
        while self.messages.len() > MAX_MESSAGES {
            let next_user = self
                .messages
                .iter()
                .enumerate()
                .skip(2)
                .find(|(_, m)| m["role"] == "user")
                .map(|(i, _)| i);
            match next_user {
                Some(i) => {
                    self.messages.drain(1..i);
                }
                None => break,
            }
        }
    }
}

/// Tool arguments: a JSON object, sent as text (OpenAI) or as an object (some servers).
fn parse_args(raw: &Value) -> Result<Value, String> {
    let v = match raw {
        Value::String(s) if s.trim().is_empty() => json!({}),
        Value::String(s) => {
            serde_json::from_str(s).map_err(|e| format!("arguments are not JSON: {e}"))?
        }
        Value::Null => json!({}),
        other => other.clone(),
    };
    if v.is_object() {
        Ok(v)
    } else {
        Err("arguments must be a JSON object".into())
    }
}

fn run(
    window: &mut ProjectWindow,
    roots: Option<&Roots>,
    name: &str,
    args: &Value,
) -> Result<Value, String> {
    if !assistant_tools().iter().any(|t| t["name"] == name) {
        return Err(format!("unknown tool `{name}`"));
    }
    if uses_files(name) {
        let roots = roots.ok_or(
            "the project is not saved yet: files can only be used inside its folder, so ask the \
             person to save it first",
        )?;
        return run_confined(window, roots, name, args);
    }
    // Paths are only read by the file tools; the rest need no roots.
    studiocraft_automation::execute(window, name, args)
}

/// The arguments in one short line for the panel.
fn short_args(args: &Value) -> String {
    let Some(o) = args.as_object() else {
        return String::new();
    };
    let parts: Vec<String> = o
        .iter()
        .map(|(k, v)| match v {
            Value::String(s) => format!("{k}: {}", shorten(s, 40)),
            Value::Array(a) => format!("{k}: [{}]", a.len()),
            Value::Object(_) => format!("{k}: {{…}}"),
            other => format!("{k}: {other}"),
        })
        .collect();
    shorten(&parts.join(", "), 120)
}

#[cfg(test)]
mod tests {
    use studiocraft_project::{Project, TrackKind};

    use super::*;

    fn window() -> ProjectWindow {
        let mut w = ProjectWindow::new(Project::default(), None, String::new());
        w.clear_history();
        w
    }

    fn call(id: &str, name: &str, args: &str) -> Value {
        json!({"id": id, "type": "function", "function": {"name": name, "arguments": args}})
    }

    /// Model replies are foreign input: random text, tool calls with unknown names, broken or
    /// odd arguments and too many of them never crash the window, and what the conversation
    /// keeps for the next request stays bounded.
    #[test]
    fn fuzzed_model_replies() {
        use crate::fuzzkit::{Case, Rng, cases};
        let tools: Vec<String> = assistant_tools()
            .iter()
            .filter_map(|t| t["name"].as_str().map(str::to_owned))
            .collect();
        let dir = std::env::temp_dir().join(format!("sc-assist-fuzz-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let mut w = window();
        let mut c = Conversation::new();
        // As in the panel: after the model is done, the person writes again.
        let mut done = true;
        for seed in cases(0xa55, 400) {
            let _case = Case("model-reply", seed);
            let mut rng = Rng(seed | 1);
            if done || rng.below(10) == 0 {
                c.user(&"Bitte ".repeat(rng.below(5000)));
            }
            let args = |rng: &mut Rng| -> Value {
                match rng.below(9) {
                    0 => json!(""),
                    1 => json!("{"),
                    2 => json!("[1, 2]"),
                    3 => json!(r#"{"kind": "midi", "name": "Spur"}"#),
                    4 => json!({"track": 0, "start": 0, "length": 3840}),
                    5 => json!(null),
                    6 => json!(42),
                    7 => json!(r#"{"path": "../../etc/passwd"}"#),
                    _ => json!(format!(
                        "{{\"name\": \"{}\"}}",
                        "x".repeat(rng.below(50_000))
                    )),
                }
            };
            let n = [0, 1, 3, 40, 500][rng.below(5)];
            let calls: Vec<Value> = (0..n)
                .map(|i| {
                    let name = if rng.below(4) == 0 {
                        "kein_werkzeug".to_owned()
                    } else {
                        tools[rng.below(tools.len())].clone()
                    };
                    let a = args(&mut rng);
                    match rng.below(6) {
                        0 => json!({"id": i}),
                        1 => json!("kein Objekt"),
                        _ => json!({"id": format!("c{i}"), "type": "function",
                            "function": {"name": name, "arguments": a}}),
                    }
                })
                .collect();
            let content = match rng.below(4) {
                0 => json!(null),
                1 => json!(""),
                2 => json!("x".repeat(rng.below(100_000))),
                _ => json!(42),
            };
            let message = match rng.below(8) {
                0 => json!("kein Objekt"),
                1 => json!({"content": content}),
                2 => json!({"content": content, "tool_calls": "keine Liste"}),
                _ => json!({"role": "assistant", "content": content, "tool_calls": calls}),
            };
            let folder = (rng.below(2) == 0).then_some(dir.as_path());
            done = c.reply(&message, &mut w, folder) == Next::Done;
            // At most the kept history plus one exchange of MAX_STEPS rounds.
            let exchange = MAX_STEPS * (MAX_CALLS + 1) + 1;
            assert!(c.messages().len() <= MAX_MESSAGES + exchange);
            for m in c.messages() {
                let text = m["content"].as_str().unwrap_or("");
                // The cut text, its ellipsis and the note for the model.
                assert!(text.chars().count() <= MAX_TEXT.max(MAX_RESULT) + 64);
            }
        }
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// A model that calls tools without end: the exchange stops with a visible message, every
    /// request it would send stays within MAX_HISTORY, long results carry a note, and the
    /// next message of the person starts afresh within the limit.
    #[test]
    fn an_endless_tool_loop_stops_and_the_history_stays_bounded() {
        let endless = |tool: &str| {
            let calls: Vec<Value> = (0..MAX_CALLS)
                .map(|i| call(&format!("c{i}"), tool, "{}"))
                .collect();
            json!({"role": "assistant", "content": "", "tool_calls": calls})
        };
        // Small results (an unknown tool's short error): the round limit stops it. Large ones
        // (the project of 300 tracks, again and again): the size limit.
        // Both languages: other tests switch the process-wide language meanwhile.
        let by_rounds = [
            "Nach 24 Schritten angehalten; schreib weiter, um fortzufahren.",
            "Stopped after 24 steps; write again to continue.",
        ];
        let by_size = [
            "Abgebrochen: zu viele Schritte – der Verlauf für das Modell ist zu groß geworden. Schreib weiter, um neu anzusetzen.",
            "Stopped: too many steps – the history for the model has grown too large. Write again to start afresh.",
        ];
        assert_eq!(MAX_STEPS, 24, "the texts above name it");
        for (tool, tracks, stop) in [
            ("kein_werkzeug", 0, by_rounds),
            ("inspect_project", 300, by_size),
        ] {
            let endless = endless(tool);
            let mut w = window();
            for i in 0..tracks {
                w.project
                    .add_track(TrackKind::Audio, format!("Spur mit langem Namen {i}"));
            }
            let mut c = Conversation::new();
            c.user("Bitte");
            let mut rounds = 0;
            while c.reply(&endless, &mut w, None) == Next::Ask {
                rounds += 1;
                assert!(rounds < MAX_STEPS, "the exchange ends");
                assert!(
                    c.history_size() <= MAX_HISTORY,
                    "request {rounds}: {} bytes",
                    c.history_size()
                );
            }
            let said = c
                .entries()
                .iter()
                .any(|e| matches!(e, Entry::Error(t) if stop.contains(&t.as_str())));
            assert!(said, "{tool}: the stop is shown ({stop:?})");
            if tracks > 0 {
                let noted = c
                    .messages()
                    .iter()
                    .filter(|m| m["role"] == "tool")
                    .filter(|m| m["content"].as_str().unwrap_or("").contains("[truncated: "))
                    .count();
                assert!(noted > 0, "cut results say so");
            }
            c.user("weiter");
            assert!(c.history_size() <= MAX_HISTORY, "the next request fits");
        }
    }

    #[test]
    fn tools_run_and_their_results_go_back_to_the_model() {
        let mut w = window();
        let tracks = w.project.tracks.len();
        let mut c = Conversation::new();
        c.user("Lege eine Bass-Spur an");
        let next = c.reply(
            &json!({"role": "assistant", "content": "", "tool_calls": [
                call("a", "add_track", r#"{"kind": "midi", "name": "Bass"}"#),
                call("b", "set_track", r#"{"track": "Bass", "volume_db": -6}"#),
            ]}),
            &mut w,
            None,
        );
        assert_eq!(next, Next::Ask);
        assert_eq!(w.project.tracks.len(), tracks + 1);
        let tools: Vec<&Value> = c
            .messages()
            .iter()
            .filter(|m| m["role"] == "tool")
            .collect();
        assert_eq!(tools.len(), 2);
        assert_eq!(tools[0]["tool_call_id"], "a");
        assert!(
            matches!(&c.entries()[1], Entry::Tool { name, error: None, args } if name == "add_track" && args.contains("Bass"))
        );

        let next = c.reply(
            &json!({"role": "assistant", "content": "Fertig."}),
            &mut w,
            None,
        );
        assert_eq!(next, Next::Done);
        assert_eq!(c.entries().last(), Some(&Entry::Model("Fertig.".into())));
        // Each edit is its own undo step.
        assert!(w.history_depth().0 >= 2);
    }

    #[test]
    fn bad_calls_are_errors_the_model_reads() {
        let mut w = window();
        let mut c = Conversation::new();
        c.user("x");
        c.reply(
            &json!({"tool_calls": [
                call("1", "rm_rf", "{}"),
                call("2", "add_track", "{nope"),
                call("3", "add_track", "[1]"),
                call("4", "new_project", r#"{"discard_changes": true}"#),
                call("5", "ui_click", r#"{"x": 1, "y": 1}"#),
                call("6", "save_project", r#"{"path": "/tmp/x"}"#),
            ]}),
            &mut w,
            None,
        );
        let errors: Vec<String> = c
            .messages()
            .iter()
            .filter(|m| m["role"] == "tool")
            .map(|m| m["content"].as_str().unwrap().to_owned())
            .collect();
        assert_eq!(errors.len(), 6);
        assert!(errors.iter().all(|e| e.contains("\"error\"")), "{errors:?}");
        assert!(errors[5].contains("not saved"), "{}", errors[5]);
    }

    #[test]
    fn files_stay_inside_the_project_folder() {
        let dir =
            std::env::temp_dir().join(format!("studiocraft-assistant-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let mut w = window();
        studiocraft_automation::execute(&mut w, "add_track", &json!({"kind": "midi"})).unwrap();
        let mut c = Conversation::new();
        c.user("x");
        let inside = dir.join("Song.mid");
        let outside = std::env::temp_dir().join("studiocraft-assistant-outside.mid");
        c.reply(
            &json!({"tool_calls": [
                call("1", "export_midi", &json!({"path": inside}).to_string()),
                call("2", "export_midi", &json!({"path": outside}).to_string()),
            ]}),
            &mut w,
            Some(&dir),
        );
        assert!(inside.exists(), "{:?}", c.messages().last());
        assert!(!outside.exists());
        let results: Vec<&Value> = c
            .messages()
            .iter()
            .filter(|m| m["role"] == "tool")
            .collect();
        assert!(results[1]["content"].as_str().unwrap().contains("rejected"));
    }

    #[test]
    fn rounds_of_tool_calls_are_capped() {
        let mut w = window();
        let mut c = Conversation::new();
        c.user("x");
        let mut rounds = 0;
        while c.reply(
            &json!({"tool_calls": [call("1", "inspect_project", "")]}),
            &mut w,
            None,
        ) == Next::Ask
        {
            rounds += 1;
            assert!(rounds < 100);
        }
        assert_eq!(rounds, MAX_STEPS - 1);
        assert!(matches!(c.entries().last(), Some(Entry::Error(_))));
        // A new message of the person starts counting again.
        c.user("weiter");
        assert_eq!(
            c.reply(
                &json!({"tool_calls": [call("1", "inspect_project", "")]}),
                &mut w,
                None
            ),
            Next::Ask
        );
    }

    #[test]
    fn old_exchanges_are_dropped_but_the_system_message_stays() {
        let mut w = window();
        let mut c = Conversation::new();
        for i in 0..MAX_MESSAGES {
            c.user(&format!("Nachricht {i}"));
            c.reply(&json!({"content": "ok"}), &mut w, None);
        }
        assert!(c.messages().len() <= MAX_MESSAGES);
        assert_eq!(c.messages()[0]["role"], "system");
        assert_eq!(c.messages()[1]["role"], "user");
    }

    #[test]
    fn the_tools_offered_leave_out_window_and_project_replacing_ones() {
        let names: Vec<String> = Conversation::tools()
            .iter()
            .map(|t| t["function"]["name"].as_str().unwrap().to_owned())
            .collect();
        assert!(names.contains(&"add_notes".to_owned()));
        assert!(names.contains(&"transport".to_owned()));
        assert!(
            !names
                .iter()
                .any(|n| n.starts_with("ui_") || n == "new_project" || n == "open_project")
        );
    }
}
