//! Fragt die Latenz installierter Plugins ab (`clap.latency`). Läuft im Hauptthread (eigenes
//! Testprogramm ohne Test-Harness) und nur auf Wunsch, weil es installierte Plugins braucht:
//!
//! ```sh
//! STUDIOCRAFT_LATENCY_TEST=1 cargo test -p studiocraft-plugin-host --test latency_main_thread
//! ```

use studiocraft_plugin_host::{MAX_LATENCY, PluginInstance, default_paths, scan};

fn main() {
    if std::env::var_os("STUDIOCRAFT_LATENCY_TEST").is_none() {
        println!("Latenz-Test übersprungen (STUDIOCRAFT_LATENCY_TEST=1 setzen)");
        return;
    }
    // Nur CLAP (gleichnamige LV2-Plugins haben noch kein Fenster).
    let plugins: Vec<_> = scan(&default_paths())
        .0
        .into_iter()
        .filter(|p| p.format == studiocraft_plugin_host::PluginFormat::Clap)
        .collect();
    let mut with_latency = 0;
    for info in plugins.iter().take(200) {
        let Ok(inst) = PluginInstance::new(info, 48_000, 512) else {
            continue;
        };
        let h = inst.handle();
        let lat = h.latency();
        assert!(lat <= MAX_LATENCY, "{}: {lat}", info.name);
        if lat > 0 {
            println!("{}: {lat} Samples", info.name);
            with_latency += 1;
        }
        let _ = h.take_latency_changed();
        drop(h);
        drop(inst);
    }
    println!("{} Plugins, {with_latency} mit Latenz", plugins.len());
}
