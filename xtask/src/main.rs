//! Entwicklungsaufgaben für StudioCraft:
//!
//! - `cargo xtask ci` – die Prüfungen vor jedem Commit und in der CI: Formatierung,
//!   Clippy ohne Warnungen, alle Tests.
//! - `cargo xtask version` – zeigt die Version; `cargo xtask version set 0.2.0` setzt sie
//!   (sie steht nur in `[workspace.package]` der Wurzel-Cargo.toml).
//! - `cargo xtask licenses` – schreibt `THIRD-PARTY-LICENSES.txt`; `--check` prüft nur.

mod licenses;

use std::{path::PathBuf, process::Command, process::ExitCode};

fn root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .map(PathBuf::from)
        .unwrap_or_default()
}

fn run(step: &str, cmd: &mut Command) -> Result<(), String> {
    eprintln!("==> {step}");
    let status = cmd
        .current_dir(root())
        .status()
        .map_err(|e| format!("{step}: {e}"))?;
    if status.success() {
        Ok(())
    } else {
        Err(format!("{step} fehlgeschlagen"))
    }
}

fn ci() -> Result<(), String> {
    eprintln!("==> Lizenzen von Drittanbietern");
    licenses::run(&root(), true)?;
    run(
        "Formatierung",
        Command::new("cargo").args(["fmt", "--all", "--check"]),
    )?;
    run(
        "Clippy",
        Command::new("cargo").args([
            "clippy",
            "--workspace",
            "--all-targets",
            "--locked",
            "--",
            "-D",
            "warnings",
        ]),
    )?;
    run(
        "Tests",
        Command::new("cargo").args(["test", "--workspace", "--locked"]),
    )?;
    eprintln!("==> alles in Ordnung");
    Ok(())
}

/// Liest bzw. ersetzt `version` im Abschnitt `[workspace.package]`.
fn version(set: Option<&str>) -> Result<(), String> {
    let path = root().join("Cargo.toml");
    let text = std::fs::read_to_string(&path).map_err(|e| e.to_string())?;
    let mut in_pkg = false;
    let mut found = None;
    let mut out = String::with_capacity(text.len());
    for line in text.lines() {
        if line.starts_with('[') {
            in_pkg = line.trim() == "[workspace.package]";
        }
        let t = line.trim_start();
        if in_pkg && found.is_none() && t.starts_with("version") && t.contains('=') {
            found = t.split('"').nth(1).map(str::to_owned);
            if let Some(new) = set {
                out.push_str(&format!("version = \"{new}\"\n"));
                continue;
            }
        }
        out.push_str(line);
        out.push('\n');
    }
    let current = found.ok_or("keine Version in [workspace.package]")?;
    match set {
        None => println!("{current}"),
        Some(new) => {
            let ok =
                new.split('.').count() == 3 && new.split('.').all(|p| p.parse::<u32>().is_ok());
            if !ok {
                return Err(format!("„{new}“ ist keine Version der Form X.Y.Z"));
            }
            std::fs::write(&path, out).map_err(|e| e.to_string())?;
            println!("{current} → {new}");
        }
    }
    Ok(())
}

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let result = match args
        .iter()
        .map(String::as_str)
        .collect::<Vec<_>>()
        .as_slice()
    {
        ["ci"] => ci(),
        ["version"] => version(None),
        ["version", "set", v] => version(Some(v)),
        ["licenses"] => licenses::run(&root(), false),
        ["licenses", "--check"] => licenses::run(&root(), true),
        _ => Err("Aufruf: cargo xtask ci | version [set X.Y.Z] | licenses [--check]".into()),
    };
    match result {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => {
            eprintln!("Fehler: {e}");
            ExitCode::FAILURE
        }
    }
}
