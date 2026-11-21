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

mod info;
#[cfg(target_arch = "wasm32")]
mod web;

#[cfg(not(target_arch = "wasm32"))]
mod au;
#[cfg(not(target_arch = "wasm32"))]
mod bridge;
#[cfg(not(target_arch = "wasm32"))]
mod clap;
#[cfg(not(target_arch = "wasm32"))]
mod clap_presets;
#[cfg(not(target_arch = "wasm32"))]
mod gui;
#[cfg(not(target_arch = "wasm32"))]
mod host;
#[cfg(not(target_arch = "wasm32"))]
mod instance;
#[cfg(not(target_arch = "wasm32"))]
mod isolated;
#[cfg(not(target_arch = "wasm32"))]
mod lv2;
#[cfg(not(target_arch = "wasm32"))]
mod lv2_gui;
#[cfg(not(target_arch = "wasm32"))]
mod scan;
#[cfg(not(target_arch = "wasm32"))]
mod vst3;
#[cfg(not(target_arch = "wasm32"))]
mod vst3_gui;

#[cfg(not(target_arch = "wasm32"))]
pub use bridge::{
    BRIDGE_FLAG, BridgeArgs, BridgeHandle, BridgeInstance, BridgedAudio, BridgedMain, Hosted,
    run_bridge_child, run_bridge_child_with,
};
pub use info::{
    MAX_LATENCY, ParamInfo, PluginFormat, PluginInfo, Preset as PluginPreset, SCAN_FLAG,
    ScanOptions, ScanReport,
};
#[cfg(not(target_arch = "wasm32"))]
pub use instance::{PluginHandle, PluginInstance};
#[cfg(not(target_arch = "wasm32"))]
pub use isolated::{clear_scan_cache, run_scan_child, scan_isolated};
#[cfg(not(target_arch = "wasm32"))]
pub use lv2::available as lv2_available;
#[cfg(not(target_arch = "wasm32"))]
pub use scan::{default_paths, find_all, scan, scan_bundle};
#[cfg(target_arch = "wasm32")]
pub use web::{
    PluginHandle, PluginInstance, clear_scan_cache, default_paths, find_all, lv2_available,
    run_bridge_child, run_scan_child, scan, scan_bundle, scan_isolated,
};
