//! Öffnet echte Plugin-Fenster. Läuft im Hauptthread (eigenes Testprogramm ohne
//! Test-Harness) und nur auf Wunsch, weil Fenster auf dem Bildschirm erscheinen:
//!
//! ```sh
//! STUDIOCRAFT_GUI_TEST=1 cargo test -p studiocraft-plugin-host --test gui_main_thread
//! ```

use std::time::{Duration, Instant};

use studiocraft_plugin_host::{PluginInstance, default_paths, scan};

fn main() {
    if std::env::var_os("STUDIOCRAFT_GUI_TEST").is_none() {
        println!("Plugin-Fenster-Test übersprungen (STUDIOCRAFT_GUI_TEST=1 setzen)");
        return;
    }
    let names: Vec<String> = std::env::var("STUDIOCRAFT_GUI_PLUGINS")
        .map(|v| v.split(',').map(str::to_owned).collect())
        .unwrap_or_else(|_| vec!["ZamComp".into(), "Surge XT".into()]);
    let plugins = scan(&default_paths()).0;
    let mut tested = 0;
    for name in &names {
        let Some(info) = plugins.iter().find(|p| &p.name == name) else {
            println!("{name}: nicht installiert");
            continue;
        };
        let inst = PluginInstance::new(info, 48_000, 512).expect("Instanz");
        let mut h = inst.handle();
        assert!(h.has_gui(), "{name} hat ein Fenster");
        match h.open_gui(&format!("StudioCraft-Test – {name}")) {
            Ok(()) => {
                let t = Instant::now();
                while t.elapsed() < Duration::from_secs(3) && h.gui_open() {
                    h.idle();
                    std::thread::sleep(Duration::from_millis(15));
                }
                println!("{name}: Fenster geöffnet und 3 s bedient");
                h.close_gui();
                tested += 1;
            }
            Err(e) => panic!("{name}: {e}"),
        }
        drop(h);
        drop(inst);
    }
    assert!(tested > 0, "kein Plugin getestet");
}
