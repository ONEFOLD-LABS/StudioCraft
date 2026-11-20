//! Every path a tool call names is checked against the operator's roots before the call runs, in
//! the MCP process, and replaced by its resolved absolute form. That holds whether the tools run
//! here (headless) or in the open app (`--connect`): the app never sees an unchecked path from an
//! agent, and a relative path cannot be read against the app's working directory instead.

use std::path::{Path, PathBuf};

use serde_json::{Value, json};

use crate::Roots;
use crate::exec::{str_arg, with_extension};

/// `args` of tool `name` with every path checked and resolved. `current_path` answers where the
/// open project is saved (for `save_project` without `path`).
pub fn confine(
    roots: &Roots,
    name: &str,
    args: &Value,
    current_path: &mut dyn FnMut() -> Result<Option<PathBuf>, String>,
) -> Result<Value, String> {
    let mut args = args.clone();
    let resolved = match name {
        "open_project" => {
            let path = roots.readable(Path::new(str_arg(&args, "path")?))?;
            check_project_audio(roots, &path)?;
            Some(path)
        }
        "import_audio" | "import_midi" => Some(roots.readable(Path::new(str_arg(&args, "path")?))?),
        "export_mixdown" | "export_midi" => {
            Some(roots.writable(Path::new(str_arg(&args, "path")?))?)
        }
        "save_project" => {
            let target = match args.get("path").and_then(Value::as_str) {
                Some(p) => with_extension(Path::new(p)),
                None => current_path()?.ok_or("the project has no file yet: pass `path`")?,
            };
            Some(roots.writable(&target)?)
        }
        _ => None,
    };
    if let (Some(path), Some(o)) = (resolved, args.as_object_mut()) {
        o.insert("path".into(), json!(path.to_string_lossy()));
    }
    Ok(args)
}

/// The audio a project names is read too: refuse a project whose files lie outside the read
/// roots instead of opening it partly. Files that are missing are left to the open, which
/// reports them.
fn check_project_audio(roots: &Roots, path: &Path) -> Result<(), String> {
    let text = std::fs::read_to_string(path).map_err(|e| format!("{}: {e}", path.display()))?;
    let project = studiocraft_project::file::from_json(&text)
        .map_err(|e| format!("{}: {e}", path.display()))?;
    let dir = path.parent().unwrap_or(Path::new("."));
    for clip in &project.pool {
        if let Some(rel) = &clip.path {
            let abs = dir.join(rel);
            if abs.exists() {
                roots.readable(&abs)?;
            }
        }
    }
    Ok(())
}
