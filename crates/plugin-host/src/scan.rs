//! CLAP-Plugins finden: Standardpfade je System, `CLAP_PATH`, Beschreibungen lesen.

use std::{
    ffi::CStr,
    path::{Path, PathBuf},
};

use crate::clap::Library;
pub use crate::info::{PluginFormat, PluginInfo};

/// Liste aus einer Pfad-Variablen (`:`, unter Windows `;`), leere Einträge übersprungen.
fn split_paths(value: &str) -> Vec<PathBuf> {
    let sep = if cfg!(windows) { ';' } else { ':' };
    value
        .split(sep)
        .filter(|s| !s.is_empty())
        .map(PathBuf::from)
        .collect()
}

/// Suchpfade: erst `CLAP_PATH` und `VST3_PATH` (z. B. Flatpak-Erweiterungen), dann die
/// Standardorte beider Formate.
pub fn default_paths() -> Vec<PathBuf> {
    let mut paths: Vec<PathBuf> = ["CLAP_PATH", "VST3_PATH"]
        .iter()
        .filter_map(|v| std::env::var(v).ok())
        .flat_map(|v| split_paths(&v))
        .collect();
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
        // VST3 (Standardorte des Formats).
        if let Some(p) = std::env::var_os("COMMONPROGRAMFILES") {
            paths.push(PathBuf::from(p).join("VST3"));
        }
        if let Some(p) = std::env::var_os("LOCALAPPDATA") {
            paths.push(PathBuf::from(p).join("Programs/Common/VST3"));
        }
    } else {
        if let Some(h) = &home {
            paths.push(h.join(".clap"));
        }
        paths.push("/usr/lib/clap".into());
        paths.push("/usr/local/lib/clap".into());
        // VST3 (Standardorte des Formats).
        if let Some(h) = &home {
            paths.push(h.join(".vst3"));
        }
        paths.push("/usr/lib/vst3".into());
        paths.push("/usr/local/lib/vst3".into());
    }
    paths
}

/// Alle `.clap`- und `.vst3`-Dateien bzw. Bundles unter `dir`, auch in Unterordnern.
fn find_bundles(dir: &Path, out: &mut Vec<PathBuf>, depth: u32) {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };
    for e in entries.flatten() {
        let p = e.path();
        if p.extension()
            .is_some_and(|x| x.eq_ignore_ascii_case("clap"))
            || crate::vst3::is_vst3(&p)
        {
            out.push(p);
        } else if p.is_dir() && depth < 4 {
            find_bundles(&p, out, depth + 1);
        }
    }
}

/// Alle Plugin-Dateien bzw. Bundles unter `paths`, sortiert und ohne Doppelte.
pub fn find_all(paths: &[PathBuf]) -> Vec<PathBuf> {
    let mut bundles = Vec::new();
    for p in paths {
        find_bundles(p, &mut bundles, 0);
    }
    bundles.sort();
    bundles.dedup();
    bundles
}

/// Liest die Beschreibungen aller Plugins einer Datei, im eigenen Prozess.
pub fn scan_bundle(bundle: &Path) -> Result<Vec<PluginInfo>, String> {
    scan_bundle_then(bundle, |r| r)
}

/// Wie [`scan_bundle`], gibt das Ergebnis aber an `f`, bevor die Bibliothek entladen wird:
/// Der Scan-Prozess meldet so die Plugins auch dann, wenn das Entladen abstürzt.
pub(crate) fn scan_bundle_then<R>(
    bundle: &Path,
    f: impl FnOnce(Result<Vec<PluginInfo>, String>) -> R,
) -> R {
    if crate::vst3::is_vst3(bundle) {
        // VST3-Module bleiben bis zum Prozessende geladen.
        return f(crate::vst3::descriptors(bundle));
    }
    let lib = match Library::load(bundle) {
        Ok(lib) => lib,
        Err(e) => return f(Err(e)),
    };
    let list = lib
        .descriptors()
        .into_iter()
        .map(|d| PluginInfo {
            path: bundle.to_path_buf(),
            ..d
        })
        .collect();
    let out = f(Ok(list));
    drop(lib);
    out
}

/// Nach Name sortieren (ohne Groß-/Kleinschreibung), bei Gleichstand nach Id.
pub(crate) fn sort_plugins(list: &mut [PluginInfo]) {
    list.sort_by(|a, b| {
        a.name
            .to_lowercase()
            .cmp(&b.name.to_lowercase())
            .then(a.id.cmp(&b.id))
    });
}

/// Durchsucht `paths` und liest die Beschreibungen aller Plugins.
/// Rückgabe: gefundene Plugins (nach Name sortiert) und Fehlermeldungen je Datei.
///
/// Die Bibliotheken werden dafür im eigenen Prozess geladen; ein fehlerhaftes Plugin kann
/// so das Programm beenden. Das Programm nutzt deshalb [`crate::scan_isolated`]; diese
/// Variante bleibt für Tests und Werkzeuge.
pub fn scan(paths: &[PathBuf]) -> (Vec<PluginInfo>, Vec<String>) {
    let mut found = Vec::new();
    let mut errors = Vec::new();
    for b in find_all(paths) {
        match scan_bundle(&b) {
            Ok(list) => found.extend(list),
            Err(e) => errors.push(format!("{}: {e}", b.display())),
        }
    }
    found.extend(crate::lv2::scan());
    found.extend(crate::au::scan());
    sort_plugins(&mut found);
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
    fn path_variables_are_split_and_skip_empty_parts() {
        let sep = if cfg!(windows) { ";" } else { ":" };
        let v = format!("/opt/a{sep}{sep}/opt/vst3{sep}");
        assert_eq!(
            split_paths(&v),
            vec![PathBuf::from("/opt/a"), PathBuf::from("/opt/vst3")]
        );
        assert!(split_paths("").is_empty());
    }

    #[test]
    fn clap_path_comes_first() {
        // SAFETY: Test setzt die Variable nur für sich selbst; andere Tests lesen sie nicht.
        let v = if cfg!(windows) {
            "/opt/a;/opt/b"
        } else {
            "/opt/a:/opt/b"
        };
        unsafe { std::env::set_var("CLAP_PATH", v) };
        let p = default_paths();
        unsafe { std::env::remove_var("CLAP_PATH") };
        assert_eq!(&p[..2], &[PathBuf::from("/opt/a"), PathBuf::from("/opt/b")]);
        assert!(p.len() > 2);
    }
}
