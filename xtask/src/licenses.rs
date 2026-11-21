//! `cargo xtask licenses`: schreibt `THIRD-PARTY-LICENSES.txt` mit den Lizenztexten aller
//! Abhängigkeiten, die in StudioCraft eingebaut werden (alle Plattformen, ohne reine
//! Test-Abhängigkeiten). MIT, Apache, BSD und Co. verlangen, dass ihr Hinweis jeder Kopie
//! beiliegt; die Pakete und der Dialog „Über StudioCraft“ enthalten diese Datei.
//!
//! Die Texte kommen aus den Quellen der Crates selbst (LICENSE-, COPYING-, NOTICE-Dateien,
//! auch eine Ebene tiefer, z. B. für eingebettete Schriften). Gleiche Texte erscheinen einmal
//! mit allen Crates, die sie nutzen. Dazu kommen die Lizenzen der Schriften unter
//! `assets/fonts`. `--check` vergleicht nur und schlägt fehl, wenn die
//! Datei veraltet ist (Teil von `cargo xtask ci`).

use std::collections::{BTreeMap, BTreeSet, HashMap};
use std::path::{Path, PathBuf};
use std::process::Command;

use serde_json::Value;

/// Name der erzeugten Datei im Wurzelverzeichnis.
pub const FILE: &str = "THIRD-PARTY-LICENSES.txt";
/// Größte Lizenzdatei, die gelesen wird.
const MAX_TEXT: u64 = 256 * 1024;

/// Ob ein Dateiname nach einem Lizenz- oder Hinweistext aussieht.
fn is_license_file(name: &str) -> bool {
    let n = name.to_ascii_uppercase();
    let stem = n.split('.').next().unwrap_or_default();
    [
        "LICENSE",
        "LICENCE",
        "COPYING",
        "NOTICE",
        "UNLICENSE",
        "COPYRIGHT",
        "OFL",
        "UFL",
    ]
    .iter()
    .any(|p| stem.starts_with(p))
        && !n.ends_with(".RS")
        && !n.ends_with(".TOML")
}

/// Lizenztexte eines Crates: Dateien im Wurzelordner und eine Ebene tiefer (nicht in
/// Quell- und Testordnern), sortiert nach Pfad.
fn texts(dir: &Path) -> Vec<(String, String)> {
    let mut found = Vec::new();
    let mut visit = |d: &Path, prefix: &str| {
        let Ok(entries) = std::fs::read_dir(d) else {
            return Vec::new();
        };
        let mut subdirs = Vec::new();
        for e in entries.flatten() {
            let name = e.file_name().to_string_lossy().into_owned();
            let path = e.path();
            if path.is_dir() {
                subdirs.push(path);
            } else if is_license_file(&name)
                && e.metadata().is_ok_and(|m| m.len() <= MAX_TEXT)
                && let Ok(text) = std::fs::read_to_string(&path)
            {
                found.push((format!("{prefix}{name}"), text.replace("\r\n", "\n")));
            }
        }
        subdirs
    };
    let subdirs = visit(dir, "");
    for sub in subdirs {
        let name = sub
            .file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_default();
        if ["src", "tests", "benches", "examples", "target", ".git"].contains(&name.as_str()) {
            continue;
        }
        visit(&sub, &format!("{name}/"));
    }
    found.sort();
    found
}

/// Alle Crates, die in einem Programm des Workspace landen (normale und Build-Abhängigkeiten
/// der Workspace-Mitglieder außer xtask, für alle Plattformen).
fn shipped(meta: &Value) -> Result<Vec<&Value>, String> {
    let packages = meta["packages"]
        .as_array()
        .ok_or("metadata: keine packages")?;
    let by_id: HashMap<&str, &Value> = packages
        .iter()
        .filter_map(|p| Some((p["id"].as_str()?, p)))
        .collect();
    let nodes: HashMap<&str, &Value> = meta["resolve"]["nodes"]
        .as_array()
        .ok_or("metadata: kein resolve")?
        .iter()
        .filter_map(|n| Some((n["id"].as_str()?, n)))
        .collect();
    let members: Vec<&str> = meta["workspace_members"]
        .as_array()
        .ok_or("metadata: keine workspace_members")?
        .iter()
        .filter_map(Value::as_str)
        .filter(|id| by_id.get(id).is_some_and(|p| p["name"] != "xtask"))
        .collect();
    let mut seen = BTreeSet::new();
    let mut stack = members.clone();
    while let Some(id) = stack.pop() {
        if !seen.insert(id) {
            continue;
        }
        let Some(node) = nodes.get(id) else { continue };
        for dep in node["deps"].as_array().into_iter().flatten() {
            let kinds = dep["dep_kinds"].as_array().into_iter().flatten();
            let used = kinds
                .clone()
                .any(|k| k["kind"].is_null() || k["kind"] == "build");
            if used && let Some(pkg) = dep["pkg"].as_str() {
                stack.push(pkg);
            }
        }
    }
    Ok(seen
        .into_iter()
        .filter(|id| !members.contains(id))
        .filter_map(|id| by_id.get(id).copied())
        .filter(|p| p["source"].is_string())
        .collect())
}

/// Erzeugt den Inhalt der Datei.
pub fn generate(root: &Path) -> Result<String, String> {
    let out = Command::new(std::env::var("CARGO").unwrap_or_else(|_| "cargo".into()))
        .args(["metadata", "--format-version", "1", "--locked"])
        .current_dir(root)
        .output()
        .map_err(|e| format!("cargo metadata: {e}"))?;
    if !out.status.success() {
        return Err(format!(
            "cargo metadata: {}",
            String::from_utf8_lossy(&out.stderr)
        ));
    }
    let meta: Value = serde_json::from_slice(&out.stdout).map_err(|e| e.to_string())?;
    // Text → Crates, die ihn mitbringen.
    let mut groups: BTreeMap<String, BTreeSet<String>> = BTreeMap::new();
    let mut missing = Vec::new();
    for p in shipped(&meta)? {
        let name = p["name"].as_str().unwrap_or_default();
        let version = p["version"].as_str().unwrap_or_default();
        let license = p["license"].as_str().unwrap_or("siehe Lizenzdatei");
        let mut label = format!("{name} {version} ({license})");
        // MPL-2.0: Quelle der (unveränderten) Dateien angeben.
        if license.contains("MPL")
            && let Some(repo) = p["repository"].as_str()
        {
            label.push_str(&format!(", source: {repo}"));
        }
        let dir = p["manifest_path"]
            .as_str()
            .map(PathBuf::from)
            .and_then(|m| m.parent().map(Path::to_path_buf))
            .ok_or_else(|| format!("{name}: kein manifest_path"))?;
        let found = texts(&dir);
        if found.is_empty() {
            // Ohne Lizenzdatei: die Autoren als Inhaber des Copyrights nennen.
            let authors: Vec<&str> = p["authors"]
                .as_array()
                .into_iter()
                .flatten()
                .filter_map(Value::as_str)
                .collect();
            let holder = if authors.is_empty() {
                format!("the {name} authors")
            } else {
                authors.join(", ")
            };
            missing.push(format!("{label}\n      Copyright (c) {holder}"));
            continue;
        }
        for (_, text) in found {
            groups
                .entry(text.trim().to_owned())
                .or_default()
                .insert(label.clone());
        }
    }
    // Mitgelieferte Schriften (assets/fonts/<name>/): ihre Lizenztexte wie die der Crates.
    if let Ok(entries) = std::fs::read_dir(root.join("assets/fonts")) {
        let mut dirs: Vec<PathBuf> = entries.flatten().map(|e| e.path()).collect();
        dirs.sort();
        for dir in dirs.into_iter().filter(|d| d.is_dir()) {
            let name = dir
                .file_name()
                .map(|n| n.to_string_lossy().into_owned())
                .unwrap_or_default();
            let label = format!("font {name} (assets/fonts/{name})");
            let found = texts(&dir);
            if found.is_empty() {
                return Err(format!("assets/fonts/{name}: keine Lizenzdatei"));
            }
            for (_, text) in found {
                groups
                    .entry(text.trim().to_owned())
                    .or_default()
                    .insert(label.clone());
            }
        }
    }
    let mut ordered: Vec<(&String, &BTreeSet<String>)> = groups.iter().collect();
    ordered.sort_by(|a, b| a.1.first().cmp(&b.1.first()).then(a.0.cmp(b.0)));
    let mut s = String::new();
    s.push_str("StudioCraft – third-party licences\n");
    s.push_str("===================================\n\n");
    s.push_str(
        "StudioCraft includes the following open-source components. Their licence and\n\
         copyright notices follow, each text once with every component that ships it.\n\
         Generated by `cargo xtask licenses` from Cargo.lock; do not edit by hand.\n",
    );
    if !missing.is_empty() {
        s.push_str(
            "\nThese components ship no licence file; their licence is the SPDX expression shown.\n\
             The standard texts of these licences (MIT, Apache-2.0, BSL-1.0, Zlib) are\n\
             reproduced below with other components:\n",
        );
        for m in &missing {
            s.push_str(&format!("  - {m}\n"));
        }
    }
    for (text, crates) in ordered {
        s.push_str("\n\n");
        s.push_str(&"=".repeat(78));
        s.push('\n');
        for c in crates {
            s.push_str(&format!("{c}\n"));
        }
        s.push_str(&"-".repeat(78));
        s.push_str("\n\n");
        s.push_str(text);
        s.push('\n');
    }
    Ok(s)
}

/// Schreibt die Datei bzw. prüft mit `check`, ob sie aktuell ist.
pub fn run(root: &Path, check: bool) -> Result<(), String> {
    let new = generate(root)?;
    let path = root.join(FILE);
    if check {
        let old = std::fs::read_to_string(&path).unwrap_or_default();
        if old.replace("\r\n", "\n") != new {
            return Err(format!(
                "{FILE} ist veraltet: `cargo xtask licenses` ausführen und die Datei committen"
            ));
        }
        eprintln!("==> {FILE} ist aktuell");
        return Ok(());
    }
    std::fs::write(&path, &new).map_err(|e| format!("{}: {e}", path.display()))?;
    eprintln!("==> {FILE} geschrieben ({} kB)", new.len() / 1024);
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn license_file_names() {
        for n in [
            "LICENSE",
            "LICENSE-MIT",
            "license-apache.txt",
            "COPYING",
            "NOTICE.md",
            "UNLICENSE",
            "OFL.txt",
            "UFL.txt",
            "LICENCE",
        ] {
            assert!(is_license_file(n), "{n}");
        }
        for n in [
            "README.md",
            "Cargo.toml",
            "license.rs",
            "lib.rs",
            "CHANGELOG.md",
        ] {
            assert!(!is_license_file(n), "{n}");
        }
    }
}
