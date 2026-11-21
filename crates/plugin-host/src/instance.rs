//! Formatneutrale Plugin-Instanz und Griff: Engine und Oberfläche sprechen nur mit diesen
//! Typen; dahinter steht je Format die eigene Umsetzung (heute CLAP).

use crate::bridge::{BridgeHandle, BridgeInstance};
use crate::{
    au::{AuHandle, AuInstance},
    clap::{ClapHandle, ClapInstance, ParamInfo},
    lv2::{Lv2Handle, Lv2Instance, Lv2Preset},
    scan::{PluginFormat, PluginInfo},
    vst3::{Vst3Handle, Vst3Instance},
};

/// Eine laufende Plugin-Instanz. Erzeugen und Freigeben im Hauptthread, `process` im
/// Audio-Thread (Thread-Regeln siehe Crate-Doku).
pub enum PluginInstance {
    Clap(ClapInstance),
    Lv2(Lv2Instance),
    Vst3(Vst3Instance),
    Au(AuInstance),
    /// In a helper process (plugin bridge, Issue #146).
    Bridged(BridgeInstance),
}

impl PluginInstance {
    /// Erzeugt, initialisiert und aktiviert das Plugin im Format aus `info`. Nur im
    /// Hauptthread.
    pub fn new(info: &PluginInfo, sample_rate: u32, max_frames: u32) -> Result<Self, String> {
        match info.format {
            PluginFormat::Clap => ClapInstance::new(info, sample_rate, max_frames).map(Self::Clap),
            PluginFormat::Lv2 => Lv2Instance::new(info, sample_rate, max_frames).map(Self::Lv2),
            PluginFormat::Vst3 => Vst3Instance::new(info, sample_rate, max_frames).map(Self::Vst3),
            PluginFormat::Au => AuInstance::new(info, sample_rate, max_frames).map(Self::Au),
        }
    }

    /// Wie [`Self::new`], aber in einem Helferprozess (`exe`: das eigene Programm, das
    /// [`crate::run_bridge_child`] aufruft). Nur im Hauptthread.
    pub fn new_isolated(
        info: &PluginInfo,
        sample_rate: u32,
        max_frames: u32,
        exe: &std::path::Path,
    ) -> Result<Self, String> {
        BridgeInstance::spawn(info, sample_rate, max_frames, exe).map(Self::Bridged)
    }

    pub fn info(&self) -> &PluginInfo {
        match self {
            Self::Clap(i) => &i.info,
            Self::Lv2(i) => &i.info,
            Self::Vst3(i) => &i.info,
            Self::Au(i) => &i.info,
            Self::Bridged(i) => &i.info,
        }
    }

    /// Parameter beim Anlegen der Instanz.
    pub fn params(&self) -> &[ParamInfo] {
        match self {
            Self::Clap(i) => &i.params,
            Self::Lv2(i) => &i.params,
            Self::Vst3(i) => &i.params,
            Self::Au(i) => &i.params,
            Self::Bridged(i) => &i.params,
        }
    }

    /// Griff für Hauptthread-Aufrufe; muss vor der Instanz freigegeben werden.
    pub fn handle(&self) -> PluginHandle {
        match self {
            Self::Clap(i) => PluginHandle::Clap(i.handle()),
            Self::Lv2(i) => PluginHandle::Lv2(i.handle()),
            Self::Vst3(i) => PluginHandle::Vst3(i.handle()),
            Self::Au(i) => PluginHandle::Au(i.handle()),
            Self::Bridged(i) => PluginHandle::Bridged(i.handle()),
        }
    }

    /// Text zu einem Parameterwert, wie das Plugin ihn anzeigt. Nur im Hauptthread.
    pub fn value_text(&self, id: u32, value: f64) -> Option<String> {
        match self {
            Self::Clap(i) => i.value_text(id, value),
            Self::Lv2(_) => None,
            Self::Vst3(_) => None,
            Self::Au(_) => None,
            Self::Bridged(_) => None,
        }
    }

    pub fn set_param(&mut self, id: u32, value: f64) {
        match self {
            Self::Clap(i) => i.set_param(id, value),
            Self::Lv2(i) => i.set_param(id, value),
            Self::Vst3(i) => i.set_param(id, value),
            Self::Au(i) => i.set_param(id, value),
            Self::Bridged(i) => i.set_param(id, value),
        }
    }

    /// Parameteränderung an Frame `time` des nächsten Blocks (Audio-Thread).
    pub fn set_param_at(&mut self, time: u32, id: u32, value: f64) {
        match self {
            Self::Clap(i) => i.set_param_at(time, id, value),
            Self::Lv2(i) => i.set_param_at(time, id, value),
            Self::Vst3(i) => i.set_param_at(time, id, value),
            Self::Au(i) => i.set_param_at(time, id, value),
            Self::Bridged(i) => i.set_param_at(time, id, value),
        }
    }

    pub fn midi(&mut self, time: u32, data: [u8; 3]) {
        match self {
            Self::Clap(i) => i.midi(time, data),
            Self::Lv2(i) => i.midi(time, data),
            Self::Vst3(i) => i.midi(time, data),
            Self::Au(i) => i.midi(time, data),
            Self::Bridged(i) => i.midi(time, data),
        }
    }

    pub fn note(&mut self, time: u32, key: u8, velocity: f32, on: bool) {
        match self {
            Self::Clap(i) => i.note(time, key, velocity, on),
            Self::Lv2(i) => i.note(time, key, velocity, on),
            Self::Vst3(i) => i.note(time, key, velocity, on),
            Self::Au(i) => i.note(time, key, velocity, on),
            Self::Bridged(i) => i.note(time, key, velocity, on),
        }
    }

    /// Einen Stereo-Block an Ort und Stelle rechnen (Audio-Thread).
    pub fn process(&mut self, left: &mut [f32], right: &mut [f32]) {
        match self {
            Self::Clap(i) => i.process(left, right),
            Self::Lv2(i) => i.process(left, right),
            Self::Vst3(i) => i.process(left, right),
            Self::Au(i) => i.process(left, right),
            Self::Bridged(i) => i.process(left, right),
        }
    }

    pub fn stop_processing(&mut self) {
        match self {
            Self::Clap(i) => i.stop_processing(),
            Self::Lv2(i) => i.stop_processing(),
            Self::Vst3(i) => i.stop_processing(),
            Self::Au(i) => i.stop_processing(),
            Self::Bridged(i) => i.stop_processing(),
        }
    }
}

/// Griff auf eine Instanz für Hauptthread-Aufrufe (Zustand, Latenz, Fenster), während die
/// Instanz selbst in der Engine rechnet. Muss vor der Instanz freigegeben werden.
pub enum PluginHandle {
    Clap(ClapHandle),
    Lv2(Lv2Handle),
    Vst3(Vst3Handle),
    Au(AuHandle),
    Bridged(BridgeHandle),
}

impl PluginHandle {
    /// Läuft das Plugin in einem eigenen Prozess (Plugin-Bridge)?
    pub fn is_isolated(&self) -> bool {
        matches!(self, Self::Bridged(_))
    }

    /// Ist der Prozess eines isolierten Plugins abgestürzt oder hängt er? Der Insert schweigt
    /// dann. Plugins im Programm selbst: immer `false`.
    pub fn crashed(&self) -> bool {
        match self {
            Self::Bridged(h) => h.crashed(),
            _ => false,
        }
    }

    pub fn save_state(&self) -> Option<Vec<u8>> {
        match self {
            Self::Clap(h) => h.save_state(),
            Self::Lv2(h) => h.save_state(),
            Self::Vst3(h) => h.save_state(),
            Self::Au(h) => h.save_state(),
            Self::Bridged(h) => h.save_state(),
        }
    }

    pub fn load_state(&self, data: &[u8]) -> bool {
        match self {
            Self::Clap(h) => h.load_state(data),
            Self::Lv2(h) => h.load_state(data),
            Self::Vst3(h) => h.load_state(data),
            Self::Au(h) => h.load_state(data),
            Self::Bridged(h) => h.load_state(data),
        }
    }

    /// Presets des Plugins (heute nur LV2), nach Name sortiert.
    pub fn presets(&self) -> Vec<Lv2Preset> {
        match self {
            Self::Clap(h) => h.presets(),
            Self::Lv2(h) => h.presets(),
            Self::Vst3(_) => Vec::new(),
            Self::Au(_) => Vec::new(),
            Self::Bridged(_) => Vec::new(),
        }
    }

    /// Ein Preset aus [`Self::presets`] direkt im Plugin laden (Formate, deren Presets mehr als
    /// Parameterwerte sind; heute CLAP). Danach liefert [`Self::param_values`] die neuen Werte.
    pub fn load_preset(&self, uri: &str) -> Result<(), String> {
        match self {
            Self::Clap(h) => h.load_preset(uri),
            _ => Err("Presets dieses Formats setzen Parameterwerte".into()),
        }
    }

    /// Aktuelle Parameterwerte (Id, Wert) aus dem Plugin, soweit es sie liefert (heute CLAP).
    pub fn param_values(&self) -> Vec<(u32, f64)> {
        match self {
            Self::Clap(h) => h.param_values(),
            Self::Bridged(h) => h.param_values(),
            _ => Vec::new(),
        }
    }

    /// Parameterwerte (Id, Wert) eines Presets aus [`Self::presets`].
    pub fn preset_values(&self, uri: &str) -> Vec<(u32, f64)> {
        match self {
            Self::Clap(_) => Vec::new(),
            Self::Lv2(h) => h.preset_values(uri),
            Self::Vst3(_) => Vec::new(),
            Self::Au(_) => Vec::new(),
            Self::Bridged(_) => Vec::new(),
        }
    }

    pub fn latency(&self) -> u32 {
        match self {
            Self::Clap(h) => h.latency(),
            Self::Lv2(h) => h.latency(),
            Self::Vst3(h) => h.latency(),
            Self::Au(h) => h.latency(),
            Self::Bridged(h) => h.latency(),
        }
    }

    pub fn take_latency_changed(&self) -> bool {
        match self {
            Self::Clap(h) => h.take_latency_changed(),
            Self::Lv2(h) => h.take_latency_changed(),
            Self::Vst3(h) => h.take_latency_changed(),
            Self::Au(h) => h.take_latency_changed(),
            Self::Bridged(h) => h.take_latency_changed(),
        }
    }

    pub fn has_gui(&self) -> bool {
        match self {
            Self::Clap(h) => h.has_gui(),
            Self::Lv2(h) => h.has_gui(),
            Self::Vst3(h) => h.has_gui(),
            Self::Au(h) => h.has_gui(),
            Self::Bridged(_) => false,
        }
    }

    pub fn gui_open(&self) -> bool {
        match self {
            Self::Clap(h) => h.gui_open(),
            Self::Lv2(h) => h.gui_open(),
            Self::Vst3(h) => h.gui_open(),
            Self::Au(h) => h.gui_open(),
            Self::Bridged(_) => false,
        }
    }

    /// Hat das Plugin seit dem letzten Aufruf geänderten Zustand gemeldet?
    pub fn take_dirty(&self) -> bool {
        match self {
            Self::Clap(h) => h.take_dirty(),
            Self::Lv2(_) => false,
            Self::Vst3(_) => false,
            Self::Au(_) => false,
            Self::Bridged(h) => h.take_dirty(),
        }
    }

    pub fn open_gui(&mut self, title: &str) -> Result<(), String> {
        match self {
            Self::Clap(h) => h.open_gui(title),
            Self::Lv2(h) => h.open_gui(title),
            Self::Vst3(h) => h.open_gui(title),
            Self::Au(h) => h.open_gui(title),
            Self::Bridged(_) => {
                Err("the editor of a plugin in its own process is not available yet".into())
            }
        }
    }

    pub fn close_gui(&mut self) {
        match self {
            Self::Clap(h) => h.close_gui(),
            Self::Lv2(h) => h.close_gui(),
            Self::Vst3(h) => h.close_gui(),
            Self::Au(h) => h.close_gui(),
            Self::Bridged(_) => {}
        }
    }

    /// Fenster bedienen; `true`, solange eines offen ist.
    pub fn idle(&mut self) -> bool {
        match self {
            Self::Clap(h) => h.idle(),
            Self::Lv2(h) => h.idle(),
            Self::Vst3(h) => h.idle(),
            Self::Au(h) => h.idle(),
            Self::Bridged(_) => false,
        }
    }

    /// Reglerbewegungen im Plugin-Fenster seit dem letzten Aufruf (Id, Wert), die der Host
    /// wie eigene Parameteränderungen setzt. CLAP-Plugins melden das über ihren Zustand.
    pub fn take_param_changes(&self) -> Vec<(u32, f64)> {
        match self {
            Self::Clap(_) => Vec::new(),
            Self::Lv2(h) => h.take_param_changes(),
            Self::Vst3(h) => h.take_param_changes(),
            Self::Au(_) => Vec::new(),
            Self::Bridged(h) => h.take_param_changes(),
        }
    }

    /// Parameterwert an das offene Plugin-Fenster geben (Änderungen vom Host).
    pub fn set_ui_value(&self, id: u32, value: f64) {
        match self {
            Self::Clap(_) => {}
            Self::Lv2(h) => h.set_ui_value(id, value),
            Self::Vst3(h) => h.set_ui_value(id, value),
            Self::Au(_) => {}
            Self::Bridged(_) => {}
        }
    }
}
