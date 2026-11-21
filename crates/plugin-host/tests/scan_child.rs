//! Scan in Kindprozessen (Issue #35). Dieses Testprogramm ist zugleich das Scan-Programm:
//! Mit dem Scan-Schalter verhält es sich wie der Kindprozess der App. Dateinamen mit
//! `absturz` oder `haengt` simulieren Plugins, die beim Laden abstürzen oder hängen;
//! `entladen` eines, das seine Plugins vollständig meldet und erst beim Entladen abstürzt.

// A test program with its own main (harness = false): it may unwrap and panic like any test
// (Cargo.toml lints, clippy.toml).
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

use std::{path::PathBuf, time::Duration};

use studiocraft_plugin_host::{
    SCAN_FLAG, ScanOptions, clear_scan_cache, default_paths, run_scan_child, scan, scan_isolated,
};

fn main() {
    let args: Vec<String> = std::env::args().collect();
    if args.get(1).map(String::as_str) == Some(SCAN_FLAG) {
        let file = args.get(2).cloned().unwrap_or_default();
        if file.contains("absturz") {
            std::process::abort();
        }
        if file.contains("entladen") {
            // Wie der echte Kindprozess vor dem Entladen: Plugin-Zeilen, dann die Abschlusszeile.
            println!("P\tcom.example.entladen\tEntladen\tBeispiel\t1.0\taudio-effect\nL");
            std::process::abort();
        }
        if file.contains("haengt") {
            std::thread::sleep(Duration::from_secs(60));
        }
        std::process::exit(run_scan_child(&args).unwrap_or(1));
    }
    crashing_and_hanging_files_are_blocked_and_cached();
    matches_the_in_process_scan_for_installed_plugins();
    println!("scan_child: ok");
}

fn options(cache: Option<PathBuf>) -> ScanOptions {
    let exe = std::env::current_exe().expect("Testprogramm");
    let mut o = ScanOptions::new(exe, cache);
    o.timeout = Duration::from_secs(2);
    o.lv2 = false;
    o
}

fn crashing_and_hanging_files_are_blocked_and_cached() {
    let dir = std::env::temp_dir().join(format!("sc-scan-child-{}", std::process::id()));
    let plugins = dir.join("plugins");
    std::fs::create_dir_all(&plugins).expect("Ordner");
    for name in [
        "absturz.clap",
        "entladen.clap",
        "haengt.clap",
        "kaputt.clap",
    ] {
        std::fs::write(plugins.join(name), b"keine Bibliothek").expect("Datei");
    }
    let cache = dir.join("cache.tsv");
    let opts = options(Some(cache.clone()));
    let paths = vec![plugins.clone()];

    let report = scan_isolated(&paths, &opts);
    // Nur das Plugin, das vor dem Absturz vollständig gemeldet wurde, bleibt ladbar.
    let ids: Vec<&str> = report.plugins.iter().map(|p| p.id.as_str()).collect();
    assert_eq!(ids, ["com.example.entladen"]);
    assert_eq!(report.own_process, ["com.example.entladen"]);
    assert_eq!(report.errors.len(), 1, "{:?}", report.errors);
    assert!(report.errors[0].contains("kaputt.clap"));
    let blocked: Vec<String> = report
        .blocked
        .iter()
        .map(|(p, r)| format!("{} {r}", p.display()))
        .collect();
    assert_eq!(blocked.len(), 3, "{blocked:?}");
    assert!(blocked[0].contains("absturz.clap") && blocked[0].contains("abgestürzt"));
    assert!(blocked[1].contains("entladen.clap") && blocked[1].contains("abgestürzt"));
    assert!(blocked[2].contains("haengt.clap") && blocked[2].contains("Zeitüberschreitung"));

    // Zweiter Lauf aus dem Cache: auch mit einem Programm, das nicht startet, gleiches Ergebnis.
    let mut cached = options(Some(cache.clone()));
    cached.exe = dir.join("gibt-es-nicht");
    assert_eq!(scan_isolated(&paths, &cached), report);

    // Geänderte Datei wird neu geprüft; Sperrliste leeren prüft alles neu.
    std::fs::write(plugins.join("kaputt.clap"), b"anders und laenger").expect("Datei");
    let again = scan_isolated(&paths, &cached);
    assert!(
        again.errors[0].contains("Scan-Prozess startet nicht"),
        "{:?}",
        again.errors
    );
    assert!(clear_scan_cache(&cache).is_ok());
    let fresh = scan_isolated(&paths, &cached);
    assert!(
        fresh.blocked.is_empty(),
        "nothing blocked without a working scanner"
    );
    let _ = std::fs::remove_dir_all(dir);
}

fn matches_the_in_process_scan_for_installed_plugins() {
    let paths = default_paths();
    let (direct, _) = scan(&paths);
    if direct.is_empty() {
        println!("keine CLAP-Plugins installiert – Vergleich übersprungen");
        return;
    }
    let mut opts = options(None);
    opts.timeout = Duration::from_secs(30);
    opts.lv2 = true;
    let report = scan_isolated(&paths, &opts);
    assert_eq!(
        report.plugins, direct,
        "same plugins as the in-process scan"
    );
}
