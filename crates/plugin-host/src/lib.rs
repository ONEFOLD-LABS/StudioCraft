//! Plugin-Hosting. Reihenfolge laut Spec: CLAP zuerst, dann VST3 (MIT-lizenziertes
//! SDK) und LV2. Kein VST2.
//!
//! Thread-Regeln von CLAP, die diese Crate einhält:
//! - Hauptthread: Bibliothek laden, Plugin erzeugen, aktivieren, deaktivieren,
//!   zerstören, Parameterliste lesen.
//! - Audio-Thread: `start_processing`, `process`, `stop_processing`.

#[cfg(not(target_arch = "wasm32"))]
mod clap;
#[cfg(not(target_arch = "wasm32"))]
mod scan;

#[cfg(not(target_arch = "wasm32"))]
pub use clap::{ClapInstance, ParamInfo, PluginHandle};
#[cfg(not(target_arch = "wasm32"))]
pub use scan::{PluginInfo, default_paths, scan};
