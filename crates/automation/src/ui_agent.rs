//! `ui_inspect` and `ui_click` in the running app: an egui plugin that reads the widget tree egui
//! builds for screen readers (AccessKit) and clicks with the mouse as a person would.
//!
//! A request becomes a job. The plugin answers it between passes: `ui_inspect` once a pass has
//! produced a tree (AccessKit is switched on for that and off again afterwards, unless something
//! else, such as a screen reader, had it on); `ui_click` resolves its target in that tree, feeds
//! pointer events into the next pass ([`egui::Plugin::input_hook`]) and answers once that pass
//! has run.

use std::collections::{HashMap, VecDeque};
use std::sync::mpsc::Sender;

use egui::accesskit::{self, Node, NodeId, Role, Toggled};
use egui::{Context, Event, FullOutput, Modifiers, PointerButton, Pos2, RawInput};
use serde_json::{Value, json};

/// Most widgets one `ui_inspect` lists.
const MAX_WIDGETS: usize = 2000;
/// A label filter longer than this cannot match anything sensible.
const MAX_QUERY: usize = 1000;

/// What a job asks for.
enum Action {
    Inspect {
        label: Option<String>,
        role: Option<String>,
    },
    Click(Click),
}

/// Where and how to click.
struct Click {
    target: Target,
    button: PointerButton,
    double: bool,
    modifiers: Modifiers,
}

enum Target {
    /// A widget from `ui_inspect`.
    Id(u64),
    /// A widget by its exact label, optionally of one role; `index` picks among several.
    Label {
        label: String,
        role: Option<String>,
        index: Option<usize>,
    },
    /// A point in the window, in points (logical pixels).
    At(Pos2),
}

/// How far a job is.
enum Phase {
    /// Waiting for a pass that produced a widget tree.
    Tree,
    /// Pointer events go into the next pass.
    Inject(Pos2, Value),
    /// The click went in; answer after this pass.
    Settle(Value),
}

struct Job {
    action: Action,
    phase: Phase,
    reply: Sender<Value>,
}

/// The plugin. Added to the context by [`submit`] on the first request.
#[derive(Default)]
pub(crate) struct UiAgent {
    jobs: VecDeque<Job>,
    /// AccessKit was switched on here and must be switched off when the jobs are done.
    enabled_here: bool,
    /// The last pass produced a tree without us asking (a screen reader is active).
    others_use_tree: bool,
    /// A click held keys: let go of them in the next pass (in the click's pass the widget reads
    /// them, as when a person clicks with Shift held).
    release_keys: bool,
}

/// Queue `ui_inspect` or `ui_click` (`method`) with `params`; the answer goes to `reply`.
/// Wrong arguments are answered at once.
pub(crate) fn submit(ctx: &Context, method: &str, params: &Value, reply: Sender<Value>) {
    let action = match parse(method, params) {
        Ok(a) => a,
        Err(e) => {
            let _ = reply.send(json!({"ok": false, "error": e}));
            return;
        }
    };
    let phase = match &action {
        Action::Click(Click {
            target: Target::At(p),
            ..
        }) => Phase::Inject(*p, json!({"at": [p.x, p.y]})),
        _ => Phase::Tree,
    };
    let needs_tree = matches!(phase, Phase::Tree);
    ctx.add_plugin(UiAgent::default());
    let job = Job {
        action,
        phase,
        reply,
    };
    let enable = ctx
        .with_plugin(|agent: &mut UiAgent| {
            agent.jobs.push_back(job);
            let enable = needs_tree && !agent.others_use_tree && !agent.enabled_here;
            agent.enabled_here |= enable;
            enable
        })
        .unwrap_or(false);
    if enable {
        ctx.enable_accesskit();
    }
    ctx.request_repaint();
}

impl egui::Plugin for UiAgent {
    fn debug_name(&self) -> &'static str {
        "studiocraft-ui-agent"
    }

    fn input_hook(&mut self, _ctx: &Context, input: &mut RawInput) {
        if std::mem::take(&mut self.release_keys) {
            input
                .events
                .push(Event::ModifiersChanged(Modifiers::default()));
        }
        let Some(job) = self.jobs.front_mut() else {
            return;
        };
        let Phase::Inject(pos, ref what) = job.phase else {
            return;
        };
        let Action::Click(click) = &job.action else {
            return;
        };
        let held = click.modifiers != Modifiers::default();
        if held {
            input.events.push(Event::ModifiersChanged(click.modifiers));
        }
        input.events.push(Event::PointerMoved(pos));
        let presses = if click.double { 2 } else { 1 };
        for _ in 0..presses {
            for pressed in [true, false] {
                input.events.push(Event::PointerButton {
                    pos,
                    button: click.button,
                    pressed,
                    modifiers: click.modifiers,
                });
            }
        }
        self.release_keys = held;
        job.phase = Phase::Settle(what.clone());
    }

    fn output_hook(&mut self, ctx: &Context, output: &mut FullOutput) {
        let tree = output.platform_output.accesskit_update.as_ref();
        if self.release_keys {
            ctx.request_repaint();
        }
        if self.jobs.is_empty() {
            self.others_use_tree = tree.is_some() && !self.enabled_here;
            return;
        }
        let screen = ctx.content_rect();
        let mut index: Option<Widgets> = None;
        // Work through the queue as far as this pass allows.
        while let Some(job) = self.jobs.front_mut() {
            let reply = match &job.phase {
                Phase::Inject(..) => break,
                Phase::Settle(what) => json!({"ok": true, "result": {"clicked": what}}),
                Phase::Tree => {
                    let Some(tree) = tree else { break };
                    let widgets = index.get_or_insert_with(|| Widgets::of(tree));
                    match &job.action {
                        Action::Inspect { label, role } => json!({
                            "ok": true,
                            "result": inspect(widgets, label.as_deref(), role.as_deref(), ctx),
                        }),
                        Action::Click(click) => match resolve(widgets, &click.target, screen) {
                            Ok((pos, what)) => {
                                job.phase = Phase::Inject(pos, what);
                                break;
                            }
                            Err(e) => json!({"ok": false, "error": e}),
                        },
                    }
                }
            };
            let _ = job.reply.send(reply);
            self.jobs.pop_front();
        }
        if self.jobs.is_empty() {
            if self.enabled_here {
                ctx.disable_accesskit();
                self.enabled_here = false;
            }
        } else {
            ctx.request_repaint();
        }
    }
}

/// The widgets of one tree, in reading order.
struct Widgets {
    list: Vec<Widget>,
}

struct Widget {
    id: u64,
    depth: usize,
    role: Role,
    label: Option<String>,
    value: Option<String>,
    rect: Option<egui::Rect>,
    disabled: bool,
    clickable: bool,
    toggled: Option<bool>,
    expanded: Option<bool>,
    focused: bool,
}

impl Widgets {
    fn of(tree: &accesskit::TreeUpdate) -> Self {
        let nodes: HashMap<NodeId, &Node> = tree.nodes.iter().map(|(id, n)| (*id, n)).collect();
        let root = tree.tree.as_ref().map(|t| t.root);
        let mut list = Vec::new();
        // Depth first from the root; `seen` guards against a malformed tree with cycles.
        let mut seen = std::collections::HashSet::new();
        let mut stack: Vec<(NodeId, usize)> = root.into_iter().map(|r| (r, 0)).collect();
        while let Some((id, depth)) = stack.pop() {
            if !seen.insert(id) {
                continue;
            }
            let Some(node) = nodes.get(&id) else { continue };
            if !node.is_hidden() {
                list.push(Widget::of(id, depth, node, tree.focus == id));
            }
            for child in node.children().iter().rev() {
                stack.push((*child, depth + 1));
            }
        }
        Self { list }
    }
}

impl Widget {
    fn of(id: NodeId, depth: usize, node: &Node, focused: bool) -> Self {
        let rect = node.bounds().and_then(|b| {
            let r = egui::Rect::from_min_max(
                egui::pos2(b.x0 as f32, b.y0 as f32),
                egui::pos2(b.x1 as f32, b.y1 as f32),
            );
            (r.is_finite() && r.is_positive()).then_some(r)
        });
        Self {
            id: id.0,
            depth,
            role: node.role(),
            label: node.label().map(str::to_owned),
            value: node.value().map(str::to_owned),
            rect,
            disabled: node.is_disabled(),
            clickable: node.supports_action(accesskit::Action::Click),
            toggled: node.toggled().map(|t| t == Toggled::True),
            expanded: node.is_expanded(),
            focused,
        }
    }

    /// Worth listing: something a person can read or use.
    fn notable(&self) -> bool {
        self.label.is_some() || self.value.is_some() || self.clickable
    }

    fn role_name(&self) -> String {
        format!("{:?}", self.role)
    }

    fn has_role(&self, role: Option<&str>) -> bool {
        role.is_none_or(|r| self.role_name().eq_ignore_ascii_case(r))
    }

    fn json(&self) -> Value {
        let mut o = json!({"id": self.id, "role": self.role_name(), "depth": self.depth});
        let Some(m) = o.as_object_mut() else {
            return o;
        };
        if let Some(l) = &self.label {
            m.insert("label".into(), json!(l));
        }
        if let Some(v) = &self.value {
            m.insert("value".into(), json!(v));
        }
        if let Some(r) = self.rect {
            m.insert(
                "rect".into(),
                json!([
                    round(r.min.x),
                    round(r.min.y),
                    round(r.width()),
                    round(r.height())
                ]),
            );
        }
        if self.clickable {
            m.insert("clickable".into(), json!(true));
        }
        if self.disabled {
            m.insert("disabled".into(), json!(true));
        }
        if let Some(t) = self.toggled {
            m.insert("checked".into(), json!(t));
        }
        if let Some(e) = self.expanded {
            m.insert("expanded".into(), json!(e));
        }
        if self.focused {
            m.insert("focused".into(), json!(true));
        }
        o
    }

    fn describe(&self) -> Value {
        json!({"id": self.id, "role": self.role_name(), "label": self.label})
    }
}

fn round(v: f32) -> f32 {
    (v * 10.0).round() / 10.0
}

fn inspect(widgets: &Widgets, label: Option<&str>, role: Option<&str>, ctx: &Context) -> Value {
    let needle = label.map(str::to_lowercase);
    let mut out = Vec::new();
    let mut truncated = false;
    for w in widgets.list.iter().filter(|w| w.notable()) {
        let text_matches = needle.as_deref().is_none_or(|n| {
            [&w.label, &w.value]
                .into_iter()
                .flatten()
                .any(|t| t.to_lowercase().contains(n))
        });
        if !text_matches || !w.has_role(role) {
            continue;
        }
        if out.len() == MAX_WIDGETS {
            truncated = true;
            break;
        }
        out.push(w.json());
    }
    let screen = ctx.content_rect();
    json!({
        "window": [round(screen.width()), round(screen.height())],
        "pixels_per_point": ctx.pixels_per_point(),
        "widgets": out,
        "truncated": truncated,
    })
}

/// Where to click for `target`, and what is clicked (for the answer).
fn resolve(
    widgets: &Widgets,
    target: &Target,
    screen: egui::Rect,
) -> Result<(Pos2, Value), String> {
    let widget = match target {
        Target::At(p) => return Ok((*p, json!({"at": [p.x, p.y]}))),
        Target::Id(id) => widgets.list.iter().find(|w| w.id == *id).ok_or(format!(
            "no widget with id {id} (ids change when the window does: call ui_inspect again)"
        ))?,
        Target::Label { label, role, index } => {
            let found: Vec<&Widget> = widgets
                .list
                .iter()
                .filter(|w| {
                    w.label.as_deref() == Some(label.as_str()) && w.has_role(role.as_deref())
                })
                .collect();
            match (found.len(), index) {
                (0, _) => {
                    return Err(format!(
                        "no widget labelled `{label}` (ui_inspect with `label` finds similar ones)"
                    ));
                }
                (1, None) => found[0],
                (n, None) => {
                    return Err(format!(
                        "{n} widgets are labelled `{label}`: pass `role`, `index` (0 to {}) or `id`",
                        n - 1
                    ));
                }
                (n, Some(i)) => found.get(*i).copied().ok_or(format!(
                    "`index` {i}: only {n} widgets are labelled `{label}`"
                ))?,
            }
        }
    };
    if widget.disabled {
        return Err(format!(
            "`{}` is disabled",
            widget.label.as_deref().unwrap_or("widget")
        ));
    }
    let rect = widget
        .rect
        .map(|r| r.intersect(screen))
        .filter(|r| r.is_positive())
        .ok_or("the widget is not visible in the window (scroll to it first)")?;
    let pos = rect.center();
    let mut what = widget.describe();
    if let Some(m) = what.as_object_mut() {
        m.insert("at".into(), json!([round(pos.x), round(pos.y)]));
    }
    Ok((pos, what))
}

fn parse(method: &str, p: &Value) -> Result<Action, String> {
    let text = |key: &str| -> Result<Option<String>, String> {
        match p.get(key) {
            None | Some(Value::Null) => Ok(None),
            Some(Value::String(s)) if s.len() <= MAX_QUERY => Ok(Some(s.clone())),
            Some(Value::String(_)) => Err(format!("`{key}` is too long")),
            Some(_) => Err(format!("`{key}` must be text")),
        }
    };
    if method == "ui_inspect" {
        return Ok(Action::Inspect {
            label: text("label")?,
            role: text("role")?,
        });
    }
    let coord = |key: &str| -> Result<Option<f32>, String> {
        match p.get(key) {
            None | Some(Value::Null) => Ok(None),
            Some(v) => v
                .as_f64()
                .map(|f| f as f32)
                .filter(|f| f.is_finite() && (0.0..=100_000.0).contains(f))
                .map(Some)
                .ok_or(format!("`{key}` must be a position in the window")),
        }
    };
    let target = match (p.get("id"), text("label")?, coord("x")?, coord("y")?) {
        (Some(id), None, None, None) => Target::Id(
            id.as_u64()
                .ok_or("`id` must be a widget id from ui_inspect")?,
        ),
        (None, Some(label), None, None) => Target::Label {
            label,
            role: text("role")?,
            index: match p.get("index") {
                None | Some(Value::Null) => None,
                Some(v) => Some(
                    v.as_u64()
                        .and_then(|i| usize::try_from(i).ok())
                        .ok_or("`index` must be a number from 0")?,
                ),
            },
        },
        (None, None, Some(x), Some(y)) => Target::At(egui::pos2(x, y)),
        _ => return Err("name one target: `id`, `label`, or both `x` and `y`".into()),
    };
    let button = match text("button")?.as_deref() {
        None | Some("left") => PointerButton::Primary,
        Some("right") => PointerButton::Secondary,
        Some("middle") => PointerButton::Middle,
        Some(b) => return Err(format!("`button` must be left, right or middle, not `{b}`")),
    };
    let double = match p.get("double") {
        None | Some(Value::Null) => false,
        Some(v) => v.as_bool().ok_or("`double` must be true or false")?,
    };
    let mut modifiers = Modifiers::default();
    if let Some(list) = p.get("modifiers").filter(|v| !v.is_null()) {
        let list = list
            .as_array()
            .ok_or("`modifiers` must be a list such as [\"shift\"]")?;
        for m in list {
            match m.as_str() {
                Some("shift") => modifiers.shift = true,
                Some("alt") => modifiers.alt = true,
                // As the window reports keys: Ctrl is the command key except on macOS.
                Some("ctrl") => {
                    modifiers.ctrl = true;
                    modifiers.command |= !cfg!(target_os = "macos");
                }
                Some("command") => {
                    modifiers.command = true;
                    if cfg!(target_os = "macos") {
                        modifiers.mac_cmd = true;
                    } else {
                        modifiers.ctrl = true;
                    }
                }
                _ => return Err(format!("unknown modifier {m}: shift, ctrl, alt or command")),
            }
        }
    }
    Ok(Action::Click(Click {
        target,
        button,
        double,
        modifiers,
    }))
}

#[cfg(test)]
mod tests {
    use std::sync::mpsc::{Receiver, channel};

    use super::*;

    /// What the test button saw.
    #[derive(Default, Debug, PartialEq)]
    struct Seen {
        clicks: u32,
        double: bool,
        secondary: bool,
        shift: bool,
    }

    /// Runs frames of a window with one button until `rx` answers.
    fn until_answer(ctx: &Context, seen: &mut Seen, rx: &Receiver<Value>) -> Value {
        // The first pass lays the button out before anything can be clicked.
        for _ in 0..20 {
            let input = RawInput {
                screen_rect: Some(egui::Rect::from_min_size(
                    Pos2::ZERO,
                    egui::vec2(400.0, 300.0),
                )),
                ..Default::default()
            };
            let mut out = ctx.run_ui(input, |ui| {
                let r = ui.button("Knopf");
                if r.clicked() {
                    seen.clicks += 1;
                    seen.shift |= ui.input(|i| i.modifiers.shift);
                }
                seen.double |= r.double_clicked();
                seen.secondary |= r.secondary_clicked();
            });
            out.textures_delta.clear();
            if let Ok(v) = rx.try_recv() {
                return v;
            }
        }
        panic!("no answer");
    }

    fn ask(ctx: &Context, seen: &mut Seen, method: &str, params: Value) -> Value {
        let (tx, rx) = channel();
        submit(ctx, method, &params, tx);
        until_answer(ctx, seen, &rx)
    }

    #[test]
    fn clicks_reach_the_widget_with_button_count_and_keys() {
        let ctx = Context::default();
        let mut seen = Seen::default();

        let found = ask(&ctx, &mut seen, "ui_inspect", json!({"label": "knopf"}));
        let widgets = found["result"]["widgets"].as_array().unwrap();
        assert_eq!(widgets.len(), 1, "{found}");
        assert_eq!(widgets[0]["role"], "Button");

        let r = ask(
            &ctx,
            &mut seen,
            "ui_click",
            json!({"label": "Knopf", "modifiers": ["shift"]}),
        );
        assert_eq!(r["ok"], true, "{r}");
        assert_eq!((seen.clicks, seen.shift, seen.double), (1, true, false));
        // The key is let go of after the click.
        let mut out = ctx.run_ui(RawInput::default(), |_| {});
        out.textures_delta.clear();
        assert!(!ctx.input(|i| i.modifiers.shift));

        let r = ask(
            &ctx,
            &mut seen,
            "ui_click",
            json!({"id": widgets[0]["id"], "double": true}),
        );
        assert_eq!(r["ok"], true, "{r}");
        assert!(seen.double);

        let r = ask(
            &ctx,
            &mut seen,
            "ui_click",
            json!({"label": "Knopf", "button": "right"}),
        );
        assert_eq!(r["ok"], true, "{r}");
        assert!(seen.secondary);

        // With the jobs done, the tree is no longer built.
        let mut out = ctx.run_ui(RawInput::default(), |_| {});
        out.textures_delta.clear();
        let mut out = ctx.run_ui(RawInput::default(), |_| {});
        out.textures_delta.clear();
        assert!(out.platform_output.accesskit_update.is_none());
    }
}
