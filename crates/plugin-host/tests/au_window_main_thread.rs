//! Öffnet unter macOS das Fenster einer Audio Unit (Apples AUDelay, auf jedem Mac vorhanden).
//! Fenster brauchen den Hauptthread, deshalb ein eigenes Testprogramm ohne Test-Harness.
//! Auf anderen Systemen gibt es nichts zu prüfen.

#[cfg(not(target_os = "macos"))]
fn main() {
    println!("Audio-Unit-Fenster gibt es nur unter macOS – übersprungen");
}

#[cfg(target_os = "macos")]
fn main() {
    use std::time::{Duration, Instant};

    use objc2::{msg_send, runtime::AnyClass, runtime::AnyObject};
    use studiocraft_plugin_host::{PluginFormat, PluginInstance, default_paths, scan};

    // Wie in der App (dort über das Fenster-Framework): die Anwendung anlegen.
    if let Some(app) = AnyClass::get(c"NSApplication") {
        // SAFETY: Klassenmethode ohne Argumente, liefert die gemeinsame Anwendung.
        let _: *mut AnyObject = unsafe { msg_send![app, sharedApplication] };
    }
    let plugins = scan(&default_paths()).0;
    let Some(info) = plugins
        .iter()
        .find(|p| p.format == PluginFormat::Au && p.id == "aufx:dely:appl")
    else {
        panic!("AUDelay fehlt");
    };
    let inst = PluginInstance::new(info, 48_000, 512).expect("AUDelay");
    let mut h = inst.handle();
    assert!(h.has_gui(), "Audio Units haben ein Fenster");
    h.open_gui("StudioCraft-Test – AUDelay")
        .expect("Fenster öffnet");
    assert!(h.gui_open());
    let t = Instant::now();
    while t.elapsed() < Duration::from_secs(1) {
        assert!(h.idle(), "Fenster bleibt offen");
        std::thread::sleep(Duration::from_millis(15));
    }
    h.close_gui();
    assert!(!h.gui_open());
    drop(h);
    drop(inst);
    println!("AUDelay: Fenster geöffnet, bedient und geschlossen");
}
