//! Reine Datentypen des Plugin-Hostings, auf allen Zielen (auch im Browser, wo es keine
//! Plugins gibt): Beschreibungen, Parameter, Presets, Scan-Einstellungen.

use std::{path::PathBuf, time::Duration};

/// Plugin-Format.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum PluginFormat {
    #[default]
    Clap,
    Lv2,
    Vst3,
    /// Audio Unit (nur macOS).
    Au,
}

/// Beschreibung eines gefundenen Plugins.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct PluginInfo {
    pub format: PluginFormat,
    pub id: String,
    pub name: String,
    pub vendor: String,
    pub version: String,
    /// CLAP-Features, z. B. `audio-effect`, `instrument`, `equalizer`.
    pub features: Vec<String>,
    /// Pfad der `.clap`-Datei bzw. des Bundles.
    pub path: PathBuf,
}

impl PluginInfo {
    /// Name für Listen; LV2-Plugins mit Formatkennung (gleiche Plugins gibt es oft in
    /// mehreren Formaten).
    pub fn display_name(&self) -> String {
        match self.format {
            PluginFormat::Clap => self.name.clone(),
            PluginFormat::Lv2 => format!("{} (LV2)", self.name),
            PluginFormat::Vst3 => format!("{} (VST3)", self.name),
            PluginFormat::Au => format!("{} (AU)", self.name),
        }
    }

    pub fn is_instrument(&self) -> bool {
        self.features.iter().any(|f| f == "instrument")
    }
}

/// Ein Parameter eines Plugins.
#[derive(Debug, Clone, PartialEq)]
pub struct ParamInfo {
    pub id: u32,
    pub name: String,
    pub module: String,
    pub min: f64,
    pub max: f64,
    pub default: f64,
    /// Wert beim Anlegen der Instanz.
    pub value: f64,
    pub stepped: bool,
}

/// Höchste Latenz, die der Host ausgleicht (frames; 2 s bei 48 kHz). Mehr meldet nur ein
/// fehlerhaftes Plugin.
pub const MAX_LATENCY: u32 = 96_000;

/// Ein Preset eines Plugins: URI und Anzeigename.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Preset {
    pub uri: String,
    pub name: String,
}

/// Befehlszeilen-Schalter des Kindprozesses: `<programm> --studiocraft-plugin-scan <datei>`.
pub const SCAN_FLAG: &str = "--studiocraft-plugin-scan";

/// Einstellungen für `scan_isolated`.
#[derive(Debug, Clone)]
pub struct ScanOptions {
    /// Programm, das `run_scan_child` bei [`SCAN_FLAG`] aufruft (meist das eigene).
    pub exe: PathBuf,
    /// Cache-Datei; `None`: kein Cache, alles wird geprüft.
    pub cache: Option<PathBuf>,
    /// So lange darf ein Kindprozess je Datei brauchen.
    pub timeout: Duration,
    /// Höchstzahl gleichzeitiger Kindprozesse.
    pub parallel: usize,
    /// Auch LV2-Plugins aufnehmen (lilv findet sie über seine eigenen Pfade).
    pub lv2: bool,
}

impl ScanOptions {
    pub fn new(exe: PathBuf, cache: Option<PathBuf>) -> Self {
        Self {
            exe,
            cache,
            timeout: Duration::from_secs(20),
            parallel: std::thread::available_parallelism()
                .map_or(2, |n| n.get())
                .clamp(1, 8),
            lv2: true,
        }
    }
}

/// Ergebnis von `scan_isolated`.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct ScanReport {
    /// Gefundene Plugins, nach Name sortiert.
    pub plugins: Vec<PluginInfo>,
    /// Dateien, die sich nicht laden ließen (ohne Absturz), mit Meldung.
    pub errors: Vec<String>,
    /// Gesperrte Dateien (Absturz oder Zeitüberschreitung beim Scan) mit Grund.
    pub blocked: Vec<(PathBuf, String)>,
}
