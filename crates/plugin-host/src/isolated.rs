//! Plugin-Scan in Kindprozessen (Issue #35): Ein fehlerhaftes Plugin, das beim Laden abstürzt
//! oder hängt, beendet nur seinen Scan-Prozess. Es landet auf einer Sperrliste und wird nicht
//! mehr geladen, bis sich die Datei ändert oder die Liste geleert wird.
//!
//! Das Programm startet sich dafür selbst mit [`SCAN_FLAG`] und einer Plugin-Datei; der
//! Kindprozess ruft [`run_scan_child`] auf und schreibt das Ergebnis zeilenweise nach stdout.
//! Ergebnisse je Datei (Pfad, Änderungszeit, Größe) bleiben in einer Cache-Datei, damit nur
//! neue oder geänderte Dateien erneut geprüft werden.

use std::{
    collections::HashMap,
    io::Read,
    path::{Path, PathBuf},
    process::{Command, Stdio},
    time::{Duration, Instant},
};

use crate::scan::{PluginInfo, find_all, scan_bundle, sort_plugins};

/// Befehlszeilen-Schalter des Kindprozesses: `<programm> --studiocraft-plugin-scan <datei>`.
pub const SCAN_FLAG: &str = "--studiocraft-plugin-scan";

/// Letzte Zeile eines vollständigen Scans; fehlt sie, gilt der Kindprozess als abgestürzt.
const DONE: &str = "DONE";

/// Höchstens so viel Ausgabe je Kindprozess (Schutz gegen endlose Ausgabe).
const MAX_OUTPUT: u64 = 4 << 20;

/// Einstellungen für [`scan_isolated`].
#[derive(Debug, Clone)]
pub struct ScanOptions {
    /// Programm, das [`run_scan_child`] bei [`SCAN_FLAG`] aufruft (meist das eigene).
    pub exe: PathBuf,
    /// Cache-Datei; `None`: kein Cache, alles wird geprüft.
    pub cache: Option<PathBuf>,
    /// So lange darf ein Kindprozess je Datei brauchen.
    pub timeout: Duration,
    /// Höchstzahl gleichzeitiger Kindprozesse.
    pub parallel: usize,
    /// Auch LV2-Plugins aufnehmen (lilv findet sie über seine eigenen Pfade).
    pub lv2: bool,
}

impl ScanOptions {
    pub fn new(exe: PathBuf, cache: Option<PathBuf>) -> Self {
        Self {
            exe,
            cache,
            timeout: Duration::from_secs(20),
            parallel: std::thread::available_parallelism()
                .map_or(2, |n| n.get())
                .clamp(1, 8),
            lv2: true,
        }
    }
}

/// Ergebnis von [`scan_isolated`].
#[derive(Debug, Clone, Default, PartialEq)]
pub struct ScanReport {
    /// Gefundene Plugins, nach Name sortiert.
    pub plugins: Vec<PluginInfo>,
    /// Dateien, die sich nicht laden ließen (ohne Absturz), mit Meldung.
    pub errors: Vec<String>,
    /// Gesperrte Dateien (Absturz oder Zeitüberschreitung beim Scan) mit Grund.
    pub blocked: Vec<(PathBuf, String)>,
}

/// Kindprozess: Ist `args[1]` [`SCAN_FLAG`], wird die Datei `args[2]` geprüft, das Ergebnis
/// nach stdout geschrieben und der Exit-Code zurückgegeben. Sonst `None` (normaler Start).
pub fn run_scan_child(args: &[String]) -> Option<i32> {
    if args.get(1).map(String::as_str) != Some(SCAN_FLAG) {
        return None;
    }
    let Some(path) = args.get(2) else {
        println!("E\tkeine Datei angegeben");
        println!("{DONE}");
        return Some(2);
    };
    let out = match scan_bundle(Path::new(path)) {
        Ok(list) => list.iter().map(plugin_line).collect::<Vec<_>>().join("\n"),
        Err(e) => format!("E\t{}", escape(&e)),
    };
    if !out.is_empty() {
        println!("{out}");
    }
    println!("{DONE}");
    Some(0)
}

fn plugin_line(p: &PluginInfo) -> String {
    format!(
        "P\t{}\t{}\t{}\t{}\t{}",
        escape(&p.id),
        escape(&p.name),
        escape(&p.vendor),
        escape(&p.version),
        p.features
            .iter()
            .map(|f| escape(f))
            .collect::<Vec<_>>()
            .join(",")
    )
}

/// Tabulator, Zeilenumbruch, Komma und Backslash maskieren (Zeilenformat).
fn escape(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for c in s.chars() {
        match c {
            '\\' => out.push_str("\\\\"),
            '\t' => out.push_str("\\t"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            ',' => out.push_str("\\c"),
            c => out.push(c),
        }
    }
    out
}

fn unescape(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut it = s.chars();
    while let Some(c) = it.next() {
        if c != '\\' {
            out.push(c);
            continue;
        }
        match it.next() {
            Some('t') => out.push('\t'),
            Some('n') => out.push('\n'),
            Some('r') => out.push('\r'),
            Some('c') => out.push(','),
            Some(o) => out.push(o),
            None => {}
        }
    }
    out
}

/// Ergebnis für eine Datei.
#[derive(Debug, Clone, PartialEq)]
enum Outcome {
    Plugins(Vec<PluginInfo>),
    Error(String),
    Blocked(String),
}

/// Ausgabe eines Kindprozesses lesen; `None`, wenn sie unvollständig ist.
fn parse_child(text: &str, path: &Path) -> Option<Outcome> {
    let mut plugins = Vec::new();
    let mut error = None;
    let mut done = false;
    for line in text.lines() {
        let f: Vec<&str> = line.split('\t').collect();
        match f.as_slice() {
            [DONE] => done = true,
            ["E", msg] => error = Some(unescape(msg)),
            ["P", id, name, vendor, version, features] => plugins.push(PluginInfo {
                // Das Format folgt aus der Datei (alte Cache-Zeilen bleiben gültig).
                format: if crate::vst3::is_vst3(path) {
                    crate::scan::PluginFormat::Vst3
                } else {
                    crate::scan::PluginFormat::Clap
                },
                id: unescape(id),
                name: unescape(name),
                vendor: unescape(vendor),
                version: unescape(version),
                features: features
                    .split(',')
                    .filter(|f| !f.is_empty())
                    .map(unescape)
                    .collect(),
                path: path.to_path_buf(),
            }),
            // Fremde Ausgaben des Plugins (Diagnose auf stdout) überspringen.
            _ => {}
        }
    }
    if !done {
        return None;
    }
    Some(match error {
        Some(e) => Outcome::Error(e),
        None => Outcome::Plugins(plugins),
    })
}

/// Prüft eine Datei in einem Kindprozess mit Zeitlimit.
fn scan_in_child(opts: &ScanOptions, path: &Path) -> Outcome {
    let child = Command::new(&opts.exe)
        .arg(SCAN_FLAG)
        .arg(path)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn();
    let mut child = match child {
        Ok(c) => c,
        // Ohne Kindprozess lässt sich nichts sicher sagen: nicht sperren, später erneut.
        Err(e) => return Outcome::Error(format!("Scan-Prozess startet nicht: {e}")),
    };
    // Ausgabe nebenher lesen, damit ein voller Puffer den Kindprozess nicht blockiert.
    let reader = child.stdout.take().map(|out| {
        std::thread::spawn(move || {
            let mut text = String::new();
            let _ = out.take(MAX_OUTPUT).read_to_string(&mut text);
            text
        })
    });
    let started = Instant::now();
    let status = loop {
        match child.try_wait() {
            Ok(Some(status)) => break Some(status),
            Ok(None) if started.elapsed() > opts.timeout => {
                let _ = child.kill();
                let _ = child.wait();
                break None;
            }
            Ok(None) => std::thread::sleep(Duration::from_millis(10)),
            Err(_) => break None,
        }
    };
    let text = reader.and_then(|r| r.join().ok()).unwrap_or_default();
    let Some(status) = status else {
        return Outcome::Blocked(format!(
            "Zeitüberschreitung beim Scan (über {} s)",
            opts.timeout.as_secs()
        ));
    };
    match parse_child(&text, path) {
        Some(outcome) if status.success() => outcome,
        _ => Outcome::Blocked(crash_reason(&status)),
    }
}

fn crash_reason(status: &std::process::ExitStatus) -> String {
    #[cfg(unix)]
    {
        use std::os::unix::process::ExitStatusExt;
        if let Some(sig) = status.signal() {
            return format!("beim Scan abgestürzt (Signal {sig})");
        }
    }
    match status.code() {
        Some(code) => format!("beim Scan abgestürzt (Exit-Code {code})"),
        None => "beim Scan abgestürzt".into(),
    }
}

/// Merkmale einer Datei, an denen der Cache Änderungen erkennt.
fn stamp(path: &Path) -> Option<(u64, u64)> {
    // Bundles (macOS) sind Ordner: die Bibliothek darin zählt.
    let file = if path.is_dir() {
        let stem = path.file_stem().unwrap_or_default();
        path.join("Contents/MacOS").join(stem)
    } else {
        path.to_path_buf()
    };
    let meta = std::fs::metadata(file).ok()?;
    let mtime = meta
        .modified()
        .ok()?
        .duration_since(std::time::UNIX_EPOCH)
        .ok()?
        .as_secs();
    Some((mtime, meta.len()))
}

/// Änderungszeit (s) und Größe einer Datei.
type Stamp = (u64, u64);

type Cache = HashMap<PathBuf, (Stamp, Outcome)>;

/// Cache-Datei lesen; Unlesbares wird übersprungen (dann eben neu prüfen).
fn load_cache(path: &Path) -> Cache {
    let mut cache: Cache = HashMap::new();
    let Ok(text) = std::fs::read_to_string(path) else {
        return cache;
    };
    for line in text.lines() {
        let f: Vec<&str> = line.splitn(5, '\t').collect();
        let [kind, file, mtime, size, rest] = f.as_slice() else {
            continue;
        };
        let (Ok(mtime), Ok(size)) = (mtime.parse::<u64>(), size.parse::<u64>()) else {
            continue;
        };
        let file = PathBuf::from(unescape(file));
        let entry = cache
            .entry(file.clone())
            .or_insert(((mtime, size), Outcome::Plugins(Vec::new())));
        entry.0 = (mtime, size);
        match *kind {
            "B" => entry.1 = Outcome::Blocked(unescape(rest)),
            "E" => entry.1 = Outcome::Error(unescape(rest)),
            "N" => {}
            "P" => {
                let text = format!("P\t{rest}\n{DONE}");
                if let (Some(Outcome::Plugins(found)), Outcome::Plugins(list)) =
                    (parse_child(&text, &file), &mut entry.1)
                {
                    list.extend(found);
                }
            }
            _ => {}
        }
    }
    cache
}

fn save_cache(path: &Path, cache: &Cache) {
    let mut keys: Vec<&PathBuf> = cache.keys().collect();
    keys.sort();
    let mut out = String::new();
    for file in keys {
        let Some(((mtime, size), outcome)) = cache.get(file) else {
            continue;
        };
        let head = format!("{}\t{mtime}\t{size}", escape(&file.to_string_lossy()));
        match outcome {
            Outcome::Blocked(r) => out.push_str(&format!("B\t{head}\t{}\n", escape(r))),
            Outcome::Error(e) => out.push_str(&format!("E\t{head}\t{}\n", escape(e))),
            Outcome::Plugins(list) if list.is_empty() => {
                out.push_str(&format!("N\t{head}\t\n"));
            }
            Outcome::Plugins(list) => {
                for p in list {
                    let line = plugin_line(p);
                    let rest = line.strip_prefix("P\t").unwrap_or(&line);
                    out.push_str(&format!("P\t{head}\t{rest}\n"));
                }
            }
        }
    }
    if let Some(dir) = path.parent() {
        let _ = std::fs::create_dir_all(dir);
    }
    let _ = std::fs::write(path, out);
}

/// Durchsucht `paths` wie [`crate::scan`], lädt jede Datei aber in einem Kindprozess.
/// Abgestürzte oder hängende Dateien werden gesperrt; unveränderte Dateien kommen aus
/// dem Cache.
pub fn scan_isolated(paths: &[PathBuf], opts: &ScanOptions) -> ScanReport {
    let mut cache = opts.cache.as_deref().map(load_cache).unwrap_or_default();
    let files = find_all(paths);
    let mut todo = Vec::new();
    let mut results: Vec<(PathBuf, Outcome)> = Vec::new();
    for f in &files {
        let st = stamp(f);
        match (st, cache.get(f)) {
            (Some(st), Some((cached, outcome))) if *cached == st => {
                results.push((f.clone(), outcome.clone()));
            }
            _ => todo.push((f.clone(), st)),
        }
    }
    // Neue und geänderte Dateien in Kindprozessen, mehrere gleichzeitig.
    for chunk in todo.chunks(opts.parallel.max(1)) {
        let done: Vec<(PathBuf, Option<Stamp>, Outcome)> = std::thread::scope(|s| {
            let handles: Vec<_> = chunk
                .iter()
                .map(|(f, st)| s.spawn(move || (f.clone(), *st, scan_in_child(opts, f))))
                .collect();
            handles.into_iter().filter_map(|h| h.join().ok()).collect()
        });
        for (f, st, outcome) in done {
            // Startfehler des Kindprozesses nicht merken: nächstes Mal erneut versuchen.
            let transient = matches!(&outcome, Outcome::Error(e) if e.starts_with("Scan-Prozess startet nicht"));
            if let (Some(st), false) = (st, transient) {
                cache.insert(f.clone(), (st, outcome.clone()));
            }
            results.push((f, outcome));
        }
    }
    // Verschwundene Dateien aus dem Cache nehmen.
    cache.retain(|f, _| files.contains(f));
    if let Some(path) = &opts.cache {
        save_cache(path, &cache);
    }
    let mut report = ScanReport::default();
    // LV2: lilv liest nur Beschreibungen (kein Plugin-Code), daher im eigenen Prozess.
    if opts.lv2 {
        report.plugins.extend(crate::lv2::scan());
    }
    for (f, outcome) in results {
        match outcome {
            Outcome::Plugins(list) => report.plugins.extend(list),
            Outcome::Error(e) => report.errors.push(format!("{}: {e}", f.display())),
            Outcome::Blocked(r) => report.blocked.push((f, r)),
        }
    }
    sort_plugins(&mut report.plugins);
    report.blocked.sort();
    report
}

/// Sperrliste leeren (und den ganzen Cache): Beim nächsten Scan wird alles neu geprüft.
pub fn clear_scan_cache(cache: &Path) -> std::io::Result<()> {
    match std::fs::remove_file(cache) {
        Err(e) if e.kind() != std::io::ErrorKind::NotFound => Err(e),
        _ => Ok(()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn info(id: &str, name: &str) -> PluginInfo {
        PluginInfo {
            format: crate::scan::PluginFormat::Clap,
            id: id.into(),
            name: name.into(),
            vendor: "V\tx".into(),
            version: "1,0\\".into(),
            features: vec!["audio-effect".into(), "a,b".into()],
            path: PathBuf::from("/x/a.clap"),
        }
    }

    #[test]
    fn child_output_roundtrips_and_needs_the_done_line() {
        let p = info("com.example.a", "Ä\nB");
        let text = format!("noise from the plugin\n{}\n{DONE}\n", plugin_line(&p));
        assert_eq!(
            parse_child(&text, Path::new("/x/a.clap")),
            Some(Outcome::Plugins(vec![p.clone()]))
        );
        assert_eq!(parse_child(&plugin_line(&p), Path::new("/x/a.clap")), None);
        assert_eq!(
            parse_child(&format!("E\tkaputt\\tda\n{DONE}"), Path::new("/x")),
            Some(Outcome::Error("kaputt\tda".into()))
        );
    }

    #[test]
    fn cache_file_roundtrips_and_skips_garbage() {
        let dir = std::env::temp_dir().join(format!("sc-scan-cache-{}", std::process::id()));
        let path = dir.join("cache.tsv");
        let mut cache: Cache = HashMap::new();
        let a = PathBuf::from("/x/a.clap");
        cache.insert(
            a.clone(),
            (
                (1, 2),
                Outcome::Plugins(vec![info("a", "A"), info("b", "B")]),
            ),
        );
        cache.insert(
            PathBuf::from("/x/b.clap"),
            ((3, 4), Outcome::Blocked("Signal 11".into())),
        );
        cache.insert(
            PathBuf::from("/x/c.clap"),
            ((5, 6), Outcome::Error("kein clap_entry".into())),
        );
        cache.insert(
            PathBuf::from("/x/d\tx.clap"),
            ((7, 8), Outcome::Plugins(Vec::new())),
        );
        save_cache(&path, &cache);
        let mut text = std::fs::read_to_string(&path).unwrap_or_default();
        text.push_str("Müll ohne Felder\nB\t/x/e\tkeine-zahl\t1\tgrund\n");
        std::fs::write(&path, text).unwrap();
        let back = load_cache(&path);
        assert_eq!(back.len(), 4);
        for (k, v) in &cache {
            let mut want = v.1.clone();
            if let Outcome::Plugins(list) = &mut want {
                for p in list.iter_mut() {
                    p.path = k.clone();
                }
            }
            assert_eq!(back.get(k), Some(&(v.0, want)), "{}", k.display());
        }
        assert!(clear_scan_cache(&path).is_ok());
        assert!(clear_scan_cache(&path).is_ok(), "missing cache is fine");
        let _ = std::fs::remove_dir_all(dir);
    }
}
