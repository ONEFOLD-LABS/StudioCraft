//! Plugin-Hosting. Reihenfolge laut Spec: CLAP zuerst, dann VST3 (MIT-lizenziertes
//! SDK) und LV2. Kein VST2.
//!
//! Thread-Regeln von CLAP, die diese Crate einhält:
//! - Hauptthread: Bibliothek laden, Plugin erzeugen, aktivieren, deaktivieren,
//!   zerstören, Parameterliste lesen.
//! - Audio-Thread: `start_processing`, `process`, `stop_processing`.
// Kein unwrap/expect/panic außerhalb von Tests (AGENTS.md „Robustness“, Issue #49).
#![cfg_attr(
    not(test),
    deny(
        clippy::unwrap_used,
        clippy::expect_used,
        clippy::panic,
        clippy::todo,
        clippy::unimplemented,
        clippy::unreachable
    )
)]

#[cfg(not(target_arch = "wasm32"))]
mod clap;
#[cfg(not(target_arch = "wasm32"))]
mod gui;
#[cfg(not(target_arch = "wasm32"))]
mod host;
#[cfg(not(target_arch = "wasm32"))]
mod scan;

#[cfg(not(target_arch = "wasm32"))]
pub use clap::{ClapInstance, MAX_LATENCY, ParamInfo, PluginHandle};
#[cfg(not(target_arch = "wasm32"))]
pub use scan::{PluginInfo, default_paths, scan};
