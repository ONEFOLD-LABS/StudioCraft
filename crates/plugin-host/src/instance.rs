//! Formatneutrale Plugin-Instanz und Griff: Engine und Oberfläche sprechen nur mit diesen
//! Typen; dahinter steht je Format die eigene Umsetzung (heute CLAP).

use crate::{
    clap::{ClapHandle, ClapInstance, ParamInfo},
    lv2::{Lv2Handle, Lv2Instance, Lv2Preset},
    scan::{PluginFormat, PluginInfo},
};

/// Eine laufende Plugin-Instanz. Erzeugen und Freigeben im Hauptthread, `process` im
/// Audio-Thread (Thread-Regeln siehe Crate-Doku).
pub enum PluginInstance {
    Clap(ClapInstance),
    Lv2(Lv2Instance),
}

impl PluginInstance {
    /// Erzeugt, initialisiert und aktiviert das Plugin im Format aus `info`. Nur im
    /// Hauptthread.
    pub fn new(info: &PluginInfo, sample_rate: u32, max_frames: u32) -> Result<Self, String> {
        match info.format {
            PluginFormat::Clap => ClapInstance::new(info, sample_rate, max_frames).map(Self::Clap),
            PluginFormat::Lv2 => Lv2Instance::new(info, sample_rate, max_frames).map(Self::Lv2),
        }
    }

    pub fn info(&self) -> &PluginInfo {
        match self {
            Self::Clap(i) => &i.info,
            Self::Lv2(i) => &i.info,
        }
    }

    /// Parameter beim Anlegen der Instanz.
    pub fn params(&self) -> &[ParamInfo] {
        match self {
            Self::Clap(i) => &i.params,
            Self::Lv2(i) => &i.params,
        }
    }

    /// Griff für Hauptthread-Aufrufe; muss vor der Instanz freigegeben werden.
    pub fn handle(&self) -> PluginHandle {
        match self {
            Self::Clap(i) => PluginHandle::Clap(i.handle()),
            Self::Lv2(i) => PluginHandle::Lv2(i.handle()),
        }
    }

    /// Text zu einem Parameterwert, wie das Plugin ihn anzeigt. Nur im Hauptthread.
    pub fn value_text(&self, id: u32, value: f64) -> Option<String> {
        match self {
            Self::Clap(i) => i.value_text(id, value),
            Self::Lv2(_) => None,
        }
    }

    pub fn set_param(&mut self, id: u32, value: f64) {
        match self {
            Self::Clap(i) => i.set_param(id, value),
            Self::Lv2(i) => i.set_param(id, value),
        }
    }

    /// Parameteränderung an Frame `time` des nächsten Blocks (Audio-Thread).
    pub fn set_param_at(&mut self, time: u32, id: u32, value: f64) {
        match self {
            Self::Clap(i) => i.set_param_at(time, id, value),
            Self::Lv2(i) => i.set_param_at(time, id, value),
        }
    }

    pub fn midi(&mut self, time: u32, data: [u8; 3]) {
        match self {
            Self::Clap(i) => i.midi(time, data),
            Self::Lv2(i) => i.midi(time, data),
        }
    }

    pub fn note(&mut self, time: u32, key: u8, velocity: f32, on: bool) {
        match self {
            Self::Clap(i) => i.note(time, key, velocity, on),
            Self::Lv2(i) => i.note(time, key, velocity, on),
        }
    }

    /// Einen Stereo-Block an Ort und Stelle rechnen (Audio-Thread).
    pub fn process(&mut self, left: &mut [f32], right: &mut [f32]) {
        match self {
            Self::Clap(i) => i.process(left, right),
            Self::Lv2(i) => i.process(left, right),
        }
    }

    pub fn stop_processing(&mut self) {
        match self {
            Self::Clap(i) => i.stop_processing(),
            Self::Lv2(i) => i.stop_processing(),
        }
    }
}

/// Griff auf eine Instanz für Hauptthread-Aufrufe (Zustand, Latenz, Fenster), während die
/// Instanz selbst in der Engine rechnet. Muss vor der Instanz freigegeben werden.
pub enum PluginHandle {
    Clap(ClapHandle),
    Lv2(Lv2Handle),
}

impl PluginHandle {
    pub fn save_state(&self) -> Option<Vec<u8>> {
        match self {
            Self::Clap(h) => h.save_state(),
            Self::Lv2(h) => h.save_state(),
        }
    }

    pub fn load_state(&self, data: &[u8]) -> bool {
        match self {
            Self::Clap(h) => h.load_state(data),
            Self::Lv2(h) => h.load_state(data),
        }
    }

    /// Presets des Plugins (heute nur LV2), nach Name sortiert.
    pub fn presets(&self) -> Vec<Lv2Preset> {
        match self {
            Self::Clap(_) => Vec::new(),
            Self::Lv2(h) => h.presets(),
        }
    }

    /// Parameterwerte (Id, Wert) eines Presets aus [`Self::presets`].
    pub fn preset_values(&self, uri: &str) -> Vec<(u32, f64)> {
        match self {
            Self::Clap(_) => Vec::new(),
            Self::Lv2(h) => h.preset_values(uri),
        }
    }

    pub fn latency(&self) -> u32 {
        match self {
            Self::Clap(h) => h.latency(),
            Self::Lv2(h) => h.latency(),
        }
    }

    pub fn take_latency_changed(&self) -> bool {
        match self {
            Self::Clap(h) => h.take_latency_changed(),
            Self::Lv2(h) => h.take_latency_changed(),
        }
    }

    pub fn has_gui(&self) -> bool {
        match self {
            Self::Clap(h) => h.has_gui(),
            Self::Lv2(_) => false,
        }
    }

    pub fn gui_open(&self) -> bool {
        match self {
            Self::Clap(h) => h.gui_open(),
            Self::Lv2(_) => false,
        }
    }

    /// Hat das Plugin seit dem letzten Aufruf geänderten Zustand gemeldet?
    pub fn take_dirty(&self) -> bool {
        match self {
            Self::Clap(h) => h.take_dirty(),
            Self::Lv2(_) => false,
        }
    }

    pub fn open_gui(&mut self, title: &str) -> Result<(), String> {
        match self {
            Self::Clap(h) => h.open_gui(title),
            Self::Lv2(_) => Err("LV2-Fenster werden noch nicht unterstützt".into()),
        }
    }

    pub fn close_gui(&mut self) {
        match self {
            Self::Clap(h) => h.close_gui(),
            Self::Lv2(_) => {}
        }
    }

    /// Fenster bedienen; `true`, solange eines offen ist.
    pub fn idle(&mut self) -> bool {
        match self {
            Self::Clap(h) => h.idle(),
            Self::Lv2(_) => false,
        }
    }
}
