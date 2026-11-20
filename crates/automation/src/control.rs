//! The app's control channel (`studiocraft --control PORT`), through `craft-control`: a loopback
//! JSON-lines server behind a per-launch token that only the user can read. Each request names a
//! tool and its arguments; the window runs it between frames ([`Control::poll`]) with the same
//! code as the headless server ([`crate::exec`]).
//!
//! The app itself does not confine paths: anyone who can read the user's token file can drive it,
//! by design. `studiocraft-cli mcp --connect` confines them before they are
//! sent ([`crate::confine`]).

use std::ffi::OsString;
use std::net::TcpListener;
use std::path::PathBuf;
use std::sync::Arc;
use std::sync::mpsc::{Receiver, channel};

use craft_control::auth::{self, App};
use craft_control::server::{Hub, Protocol, Summary, new_token};
use craft_control::{ControlRequest, Effect};
use serde_json::{Value, json};
use studiocraft_ui::ProjectWindow;

use crate::{exec, ui_agent};

/// StudioCraft's control channel: where its token files live (`<config>/control/<port>.token`),
/// its environment (`STUDIOCRAFT_CONTROL_DIR`, `STUDIOCRAFT_CONTROL_TOKEN`) and its default port.
pub const APP: App = App {
    name: "StudioCraft",
    data_dir: studiocraft_ui::settings::config_dir,
    env: env_var,
    utc: utc_text,
    default_port: 7850,
};

/// Longest request line the app reads (16 MiB, as the MCP server).
const MAX_LINE: usize = 16 << 20;

fn env_var(name: &str) -> Option<OsString> {
    std::env::var_os(format!("STUDIOCRAFT_{name}"))
}

/// A time for messages, without calendar arithmetic of our own.
fn utc_text(secs: u64) -> String {
    format!("{secs} s after 1970-01-01 UTC")
}

/// What a tool does if it outlasts its wait, for the client's messages.
pub fn effect(tool: &str) -> Effect {
    match tool {
        "inspect_project" | "get_notes" | "ui_screenshot" | "ui_inspect" => Effect::ReadsOnly,
        "save_project" | "export_mixdown" | "export_midi" => Effect::WritesFile,
        _ => Effect::Changes,
    }
}

/// Tools that may take long (file work, rendering).
fn long(tool: &str) -> bool {
    matches!(
        tool,
        "open_project" | "save_project" | "import_audio" | "import_midi" | "export_mixdown"
    )
}

struct StudioProtocol;

impl Protocol for StudioProtocol {
    fn summary(&self, method: &str, _params: &Value) -> Summary {
        Summary {
            what: format!("`{method}`"),
            effect: effect(method),
            command: None,
            steps: None,
            long: long(method),
        }
    }
    fn document(&self) -> &'static str {
        "project"
    }
    fn change_event(&self) -> &'static str {
        "project.changed"
    }
    fn revision_method(&self) -> &'static str {
        "inspect_project"
    }
    fn docs(&self) -> &'static str {
        "docs/mcp.md"
    }
    fn capabilities(&self) -> Value {
        json!({"app": "StudioCraft", "version": env!("CARGO_PKG_VERSION"), "tools": crate::tool_names(true)})
    }
}

/// The running control channel. Dropping it removes the token files.
pub struct Control {
    requests: Receiver<ControlRequest>,
    port: u16,
    token: String,
    /// The token file this launch wrote, removed again on drop.
    token_file: Option<PathBuf>,
}

impl Control {
    /// Listen on `127.0.0.1:port` with a new token written to the user's token file.
    /// `wake` makes the window run a frame now (`egui::Context::request_repaint`).
    pub fn start(port: u16, wake: impl Fn() + Send + Sync + 'static) -> Result<Self, String> {
        let listener = TcpListener::bind(("127.0.0.1", port))
            .map_err(|e| format!("control port {port}: {e}"))?;
        let port = listener.local_addr().map_err(|e| e.to_string())?.port();
        let token = APP
            .env_token()
            .or_else(new_token)
            .ok_or("no random token for the control channel")?;
        let file = APP
            .token_path(port)
            .ok_or("no folder for the control token (settings folder unknown)")?;
        APP.write_token_file(&file, &token)
            .map_err(|e| format!("{}: {e}", file.display()))?;
        // Discovery for `--connect` without a port; best effort.
        let _ = APP.write_current_port(port);
        let mut c = Self::serve(listener, token, wake)?;
        c.token_file = Some(file);
        Ok(c)
    }

    /// Serve `listener` with `token`, writing no files (tests, embedding).
    pub fn serve(
        listener: TcpListener,
        token: String,
        wake: impl Fn() + Send + Sync + 'static,
    ) -> Result<Self, String> {
        let port = listener.local_addr().map_err(|e| e.to_string())?.port();
        let hub = Arc::new(Hub::new(StudioProtocol, wake, MAX_LINE));
        let (tx, requests) = channel();
        hub.listen(listener, token.clone(), tx, false)
            .map_err(|e| e.to_string())?;
        Ok(Self {
            requests,
            port,
            token,
            token_file: None,
        })
    }

    pub fn port(&self) -> u16 {
        self.port
    }

    /// Answer the requests that came in since the last frame. `ctx` is the window's: it takes
    /// screenshots and runs `ui_inspect` and `ui_click`; without it these are refused.
    pub fn poll(&mut self, window: &mut ProjectWindow, ctx: Option<&egui::Context>) {
        while let Ok(req) = self.requests.try_recv() {
            if !req.ticket.take() {
                continue;
            }
            req.started
                .store(true, std::sync::atomic::Ordering::Relaxed);
            if matches!(req.method.as_str(), "ui_inspect" | "ui_click") {
                match ctx {
                    Some(ctx) => ui_agent::submit(ctx, &req.method, &req.params, req.reply.clone()),
                    None => {
                        let _ = req
                            .reply
                            .send(json!({"ok": false, "error": "no window to inspect or click"}));
                    }
                }
                continue;
            }
            if req.method == "ui_screenshot" {
                match ctx {
                    Some(ctx) => {
                        let reply = req.reply.clone();
                        ctx.request_screenshot(move |image| {
                            let _ = reply.send(screenshot_reply(&image));
                        });
                        ctx.request_repaint();
                    }
                    None => {
                        let _ = req.reply.send(
                            json!({"ok": false, "error": "no window to take a screenshot of"}),
                        );
                    }
                }
                continue;
            }
            let reply = match exec::execute(window, &req.method, &req.params) {
                Ok(result) => json!({"ok": true, "result": result}),
                Err(error) => json!({"ok": false, "error": error}),
            };
            let _ = req.reply.send(reply);
        }
    }
}

impl Drop for Control {
    fn drop(&mut self) {
        if let Some(file) = &self.token_file {
            auth::remove_token_file(file, &self.token);
            APP.remove_current_port(self.port, &self.token);
        }
    }
}

/// The reply to `ui_screenshot`: the window as a base64 PNG.
fn screenshot_reply(image: &egui::ColorImage) -> Value {
    let [w, h] = image.size;
    match encode_png(image) {
        Ok(png) => json!({"ok": true, "result": {
            "mime": "image/png",
            "data": craft_mcp_server::base64(&png),
            "width": w,
            "height": h,
        }}),
        Err(e) => json!({"ok": false, "error": format!("screenshot: {e}")}),
    }
}

fn encode_png(image: &egui::ColorImage) -> Result<Vec<u8>, String> {
    let [w, h] = image.size;
    let (w32, h32) = (
        u32::try_from(w).map_err(|e| e.to_string())?,
        u32::try_from(h).map_err(|e| e.to_string())?,
    );
    let rgba: Vec<u8> = image
        .pixels
        .iter()
        .flat_map(|c| c.to_srgba_unmultiplied())
        .collect();
    let mut out = Vec::new();
    {
        let mut enc = png::Encoder::new(&mut out, w32, h32);
        enc.set_color(png::ColorType::Rgba);
        enc.set_depth(png::BitDepth::Eight);
        let mut writer = enc.write_header().map_err(|e| e.to_string())?;
        writer.write_image_data(&rgba).map_err(|e| e.to_string())?;
    }
    Ok(out)
}
