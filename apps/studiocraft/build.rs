//! Nur Windows: App-Symbol und Versionsangaben (VERSIONINFO) in `studiocraft.exe` einbetten,
//! damit sie im Explorer, in der Taskleiste, im Startmenü und bei Alt+Tab erscheinen.
//!
//! Auf allen anderen Zielen tut das Skript nichts. Fehlt der Ressourcen-Compiler, gibt es eine
//! Warnung (Cross-Builds linken trotzdem); mit `STUDIOCRAFT_REQUIRE_WINRES=1` ist es ein Fehler
//! (Release-Builds). Aufbau nach SoundCraft (storytold/soundcraft, Apache-2.0), siehe NOTICE.

fn main() {
    println!("cargo:rerun-if-changed=build.rs");
    println!("cargo:rerun-if-changed=../../assets/app-icon/studiocraft.ico");
    println!("cargo:rerun-if-env-changed=STUDIOCRAFT_REQUIRE_WINRES");
    if std::env::var("CARGO_CFG_TARGET_OS").as_deref() != Ok("windows") {
        return;
    }
    let mut res = winresource::WindowsResource::new();
    res.set_icon("../../assets/app-icon/studiocraft.ico")
        .set("ProductName", "StudioCraft")
        .set("FileDescription", "StudioCraft digital audio workstation")
        .set(
            "LegalCopyright",
            "Copyright (c) StudioCraft contributors. MIT OR Apache-2.0.",
        )
        .set("OriginalFilename", "studiocraft.exe")
        .set("InternalName", "studiocraft");
    if let Err(e) = res.compile() {
        if std::env::var_os("STUDIOCRAFT_REQUIRE_WINRES").is_some() {
            eprintln!("Windows-Ressourcen ließen sich nicht einbetten: {e}");
            std::process::exit(1);
        }
        println!("cargo:warning=studiocraft.exe ohne Symbol und Versionsangaben gebaut: {e}");
    }
}
