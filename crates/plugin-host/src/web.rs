//! Browser (wasm32): keine Plugins. Gleiche öffentliche Schnittstelle wie auf dem Desktop, damit
//! Engine und Oberfläche unverändert bauen; Instanzen und Griffe lassen sich nicht erzeugen
//! (unbewohnte Typen), Scans liefern nichts.

use std::{
    convert::Infallible,
    path::{Path, PathBuf},
};

use crate::info::{ParamInfo, PluginInfo, Preset, ScanOptions, ScanReport};

/// Gibt es im Browser nicht; [`PluginInstance::new`] scheitert immer.
pub struct PluginInstance(Infallible);

impl PluginInstance {
    pub fn new_isolated(
        info: &PluginInfo,
        sample_rate: u32,
        max_frames: u32,
        _exe: &std::path::Path,
    ) -> Result<Self, crate::error::PluginError> {
        Self::new(info, sample_rate, max_frames)
    }

    pub fn new(info: &PluginInfo, _: u32, _: u32) -> Result<Self, crate::error::PluginError> {
        Err(crate::error::PluginError::Create {
            plugin: info.name.clone(),
            step: crate::error::Step::Browser,
        })
    }
    pub fn info(&self) -> &PluginInfo {
        match self.0 {}
    }
    pub fn params(&self) -> &[ParamInfo] {
        match self.0 {}
    }
    pub fn handle(&self) -> PluginHandle {
        match self.0 {}
    }
    pub fn value_text(&self, _: u32, _: f64) -> Option<String> {
        match self.0 {}
    }
    pub fn set_param(&mut self, _: u32, _: f64) {
        match self.0 {}
    }
    pub fn set_param_at(&mut self, _: u32, _: u32, _: f64) {
        match self.0 {}
    }
    pub fn midi(&mut self, _: u32, _: [u8; 3]) {
        match self.0 {}
    }
    pub fn note(&mut self, _: u32, _: u8, _: f32, _: bool) {
        match self.0 {}
    }
    pub fn process(&mut self, _: &mut [f32], _: &mut [f32]) {
        match self.0 {}
    }
    pub fn stop_processing(&mut self) {
        match self.0 {}
    }
    pub fn set_offline(&mut self, _: bool) {
        match self.0 {}
    }
}

/// Gibt es im Browser nicht (entsteht nur aus einer Instanz).
pub struct PluginHandle(Infallible);

impl PluginHandle {
    pub fn is_isolated(&self) -> bool {
        false
    }

    pub fn crashed(&self) -> bool {
        false
    }

    pub fn save_state(&self) -> Option<Vec<u8>> {
        match self.0 {}
    }
    pub fn load_state(&self, _: &[u8]) -> bool {
        match self.0 {}
    }
    pub fn presets(&self) -> Vec<Preset> {
        match self.0 {}
    }
    pub fn preset_values(&self, _: &str) -> Vec<(u32, f64)> {
        match self.0 {}
    }
    pub fn load_preset(&self, _: &str) -> Result<(), crate::error::PluginError> {
        match self.0 {}
    }
    pub fn param_values(&self) -> Vec<(u32, f64)> {
        match self.0 {}
    }
    pub fn latency(&self) -> u32 {
        match self.0 {}
    }
    pub fn take_latency_changed(&self) -> bool {
        match self.0 {}
    }
    pub fn has_gui(&self) -> bool {
        match self.0 {}
    }
    pub fn gui_open(&self) -> bool {
        match self.0 {}
    }
    pub fn take_dirty(&self) -> bool {
        match self.0 {}
    }
    pub fn open_gui(&mut self, _: &str) -> Result<(), crate::error::PluginError> {
        match self.0 {}
    }
    pub fn close_gui(&mut self) {
        match self.0 {}
    }
    pub fn idle(&mut self) -> bool {
        match self.0 {}
    }
    pub fn take_param_changes(&self) -> Vec<(u32, f64)> {
        match self.0 {}
    }
    pub fn set_ui_value(&self, _: u32, _: f64) {
        match self.0 {}
    }
}

pub fn default_paths() -> Vec<PathBuf> {
    Vec::new()
}

pub fn find_all(_: &[PathBuf]) -> Vec<PathBuf> {
    Vec::new()
}

pub fn scan(_: &[PathBuf]) -> (Vec<PluginInfo>, Vec<String>) {
    (Vec::new(), Vec::new())
}

pub fn scan_bundle(_: &Path) -> Result<Vec<PluginInfo>, crate::error::PluginError> {
    Err(crate::error::PluginError::Unavailable(
        crate::error::Unavailable::Browser,
    ))
}

pub fn scan_isolated(_: &[PathBuf], _: &ScanOptions) -> ScanReport {
    ScanReport::default()
}

pub fn clear_scan_cache(_: &Path) -> std::io::Result<()> {
    Ok(())
}

/// Im Browser gibt es keine Kindprozesse.
pub fn run_scan_child(_: &[String]) -> Option<i32> {
    None
}

/// Im Browser gibt es keine Hilfsprozesse (Plugin-Bridge).
pub fn run_bridge_child(_: &[String]) -> Option<i32> {
    None
}

pub fn lv2_available() -> bool {
    false
}
