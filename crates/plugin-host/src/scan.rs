//! CLAP-Plugins finden: Standardpfade je System, `CLAP_PATH`, Beschreibungen lesen.

use std::{
    ffi::CStr,
    path::{Path, PathBuf},
};

use crate::clap::Library;

/// Beschreibung eines gefundenen Plugins.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PluginInfo {
    pub id: String,
    pub name: String,
    pub vendor: String,
    pub version: String,
    /// CLAP-Features, z. B. `audio-effect`, `instrument`, `equalizer`.
    pub features: Vec<String>,
    /// Pfad der `.clap`-Datei bzw. des Bundles.
    pub path: PathBuf,
}

impl PluginInfo {
    pub fn is_instrument(&self) -> bool {
        self.features.iter().any(|f| f == "instrument")
    }
}

/// Suchpfade nach CLAP-Konvention: erst `CLAP_PATH`, dann die Standardorte.
pub fn default_paths() -> Vec<PathBuf> {
    let sep = if cfg!(windows) { ';' } else { ':' };
    let mut paths: Vec<PathBuf> = std::env::var("CLAP_PATH")
        .map(|v| {
            v.split(sep)
                .filter(|s| !s.is_empty())
                .map(PathBuf::from)
                .collect()
        })
        .unwrap_or_default();
    let home = std::env::var_os("HOME").map(PathBuf::from);
    if cfg!(target_os = "macos") {
        if let Some(h) = &home {
            paths.push(h.join("Library/Audio/Plug-Ins/CLAP"));
        }
        paths.push("/Library/Audio/Plug-Ins/CLAP".into());
    } else if cfg!(windows) {
        if let Some(p) = std::env::var_os("COMMONPROGRAMFILES") {
            paths.push(PathBuf::from(p).join("CLAP"));
        }
        if let Some(p) = std::env::var_os("LOCALAPPDATA") {
            paths.push(PathBuf::from(p).join("Programs/Common/CLAP"));
        }
    } else {
        if let Some(h) = &home {
            paths.push(h.join(".clap"));
        }
        paths.push("/usr/lib/clap".into());
        paths.push("/usr/local/lib/clap".into());
    }
    paths
}

/// Alle `.clap`-Dateien bzw. Bundles unter `dir`, auch in Unterordnern.
fn find_bundles(dir: &Path, out: &mut Vec<PathBuf>, depth: u32) {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };
    for e in entries.flatten() {
        let p = e.path();
        if p.extension()
            .is_some_and(|x| x.eq_ignore_ascii_case("clap"))
        {
            out.push(p);
        } else if p.is_dir() && depth < 4 {
            find_bundles(&p, out, depth + 1);
        }
    }
}

/// Durchsucht `paths` und liest die Beschreibungen aller Plugins.
/// Rückgabe: gefundene Plugins (nach Name sortiert) und Fehlermeldungen je Datei.
///
/// Hinweis: Die Bibliotheken werden dafür im eigenen Prozess geladen. Ein
/// fehlerhaftes Plugin kann so das Programm beenden; ein Scan in einem
/// eigenen Prozess folgt später.
pub fn scan(paths: &[PathBuf]) -> (Vec<PluginInfo>, Vec<String>) {
    let mut bundles = Vec::new();
    for p in paths {
        find_bundles(p, &mut bundles, 0);
    }
    bundles.sort();
    bundles.dedup();
    let mut found = Vec::new();
    let mut errors = Vec::new();
    for b in bundles {
        match Library::load(&b) {
            Ok(lib) => found.extend(lib.descriptors().into_iter().map(|d| PluginInfo {
                path: b.clone(),
                ..d
            })),
            Err(e) => errors.push(format!("{}: {e}", b.display())),
        }
    }
    found.sort_by(|a, b| {
        a.name
            .to_lowercase()
            .cmp(&b.name.to_lowercase())
            .then(a.id.cmp(&b.id))
    });
    (found, errors)
}

/// C-Zeichenkette lesen; Null-Zeiger ergibt "".
pub(crate) fn cstr(p: *const std::ffi::c_char) -> String {
    if p.is_null() {
        return String::new();
    }
    // SAFETY: CLAP garantiert nullterminierte Zeichenketten, solange das Objekt lebt.
    unsafe { CStr::from_ptr(p) }.to_string_lossy().into_owned()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn clap_path_comes_first() {
        // SAFETY: Test setzt die Variable nur für sich selbst; andere Tests lesen sie nicht.
        unsafe { std::env::set_var("CLAP_PATH", "/opt/a:/opt/b") };
        let p = default_paths();
        unsafe { std::env::remove_var("CLAP_PATH") };
        assert_eq!(&p[..2], &[PathBuf::from("/opt/a"), PathBuf::from("/opt/b")]);
        assert!(p.len() > 2);
    }
}
