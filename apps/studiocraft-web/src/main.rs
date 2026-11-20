//! StudioCraft im Browser: dasselbe Projektfenster wie die Desktop-App, gezeichnet in ein
//! Canvas (WebGPU, sonst WebGL2), Wiedergabe über Web Audio. Gebaut mit `trunk build --release`
//! (siehe Trunk.toml und packaging/web/package.sh).
// Kein unwrap/expect/panic außerhalb von Tests (AGENTS.md „Robustness“, Issue #49).
#![deny(clippy::unwrap_used, clippy::expect_used, clippy::panic)]

#[cfg(target_arch = "wasm32")]
mod web;

#[cfg(target_arch = "wasm32")]
fn main() {
    web::start();
}

#[cfg(not(target_arch = "wasm32"))]
fn main() {
    eprintln!(
        "studiocraft-web läuft nur im Browser: cd apps/studiocraft-web && trunk build --release"
    );
}
