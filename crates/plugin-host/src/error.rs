//! Fehler des Plugin-Hostings (Issue #265): was beim Laden, beim Fenster oder beim Preset
//! schiefging, mit dem Namen des Plugins. Die Oberfläche übersetzt sie (`crates/ui`,
//! `plugin_error_text`); [`std::fmt::Display`] gibt den deutschen Text als Rückfall. Meldungen
//! des Systems (Bibliothek laden, X11) und der Bridge stehen unverändert in `String`-Feldern.
//!
//! Plugins im eigenen Prozess melden ihre Fehler über die Bridge als JSON dieses Typs
//! ([`PluginError::to_wire`], [`PluginError::from_wire`]), damit sie übersetzbar bleiben.

use std::fmt;

use serde::{Deserialize, Serialize};

/// Ein Fehler des Plugin-Hostings.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum PluginError {
    /// Die Bibliothek ließ sich nicht laden (Meldung des Systems).
    Library(String),
    /// Die Datei ist kein brauchbares Plugin ihres Formats.
    BadFile(BadFile),
    /// Das Plugin `plugin` scheiterte bei `step`.
    Create { plugin: String, step: Step },
    /// Das Format gibt es hier nicht.
    Unavailable(Unavailable),
    /// Das Plugin-Fenster ließ sich nicht öffnen.
    Window(WindowProblem),
    /// Ein Preset ließ sich nicht laden.
    Preset(PresetProblem),
    /// Eine interne Sperre ist nach einem Absturz eines anderen Threads unbrauchbar.
    Locked(Lock),
    /// Plugin im eigenen Prozess (Bridge): technische Meldung, englisch.
    Bridge(String),
}

/// Warum eine Datei kein brauchbares Plugin ist.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum BadFile {
    NoClapEntry,
    EmptyClapEntry,
    IncompatibleClap,
    ClapInitMissing,
    ClapInitFailed,
    NoFactory,
    NoCreate,
    NoVst3Factory,
    EmptyVst3Factory,
    ModuleEntry,
    InitDll,
    InvalidClassId,
}

/// Der Schritt, an dem ein Plugin scheiterte.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum Step {
    /// CLAP: das Plugin ließ sich nicht erzeugen.
    Create,
    Init,
    Activate,
    NoAudioOutput,
    /// VST3 `initialize`.
    Initialize,
    NoProcessor,
    SetupProcessing,
    Component,
    AuNotFound,
    AuCreate,
    AuNoOutputFormat,
    AuInit,
    Lv2NotFound,
    Lv2Instantiate,
    Browser,
}

/// Formate, die es hier nicht gibt.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum Unavailable {
    AudioUnitsMacOnly,
    Vst3System,
    Lv2NoLilv,
    Browser,
}

/// Warum das Plugin-Fenster nicht aufging.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum WindowProblem {
    MainThreadOnly,
    NoWindow,
    IncompleteGui,
    UnsupportedSystem,
    NoMatchingApi,
    CannotCreate,
    CannotShow,
    CannotEmbed,
    CannotEmbedHere,
    NoX11Display,
    NeedsX11,
    /// Das Host-Fenster ließ sich nicht anlegen (Meldung von X11 oder Windows).
    HostWindow(String),
    /// Windows: die Fensterklasse ließ sich nicht anmelden.
    HostClass,
    /// Windows: das Host-Fenster ließ sich nicht anlegen.
    HostCreate,
    Lv2OnlyX11,
    Lv2NoUi,
    Lv2UiNotInLibrary,
    Lv2UiCreate,
    Lv2NoMatchingUi,
    NoController,
    AuNoWindow,
    AuNoContentView,
}

/// Warum ein Preset nicht geladen wurde.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum PresetProblem {
    SetsParams,
    NotClapPreset,
    NotLoaded,
}

/// Welche interne Sperre.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum Lock {
    PluginCache,
    Vst3Modules,
    UiLibraries,
}

impl PluginError {
    /// Für die Bridge: der Fehler als eine Zeile JSON.
    pub fn to_wire(&self) -> String {
        serde_json::to_string(self).unwrap_or_else(|_| self.to_string())
    }

    /// Von der Bridge: JSON dieses Typs, sonst der Text als Bridge-Meldung.
    pub fn from_wire(text: &str) -> Self {
        serde_json::from_str(text).unwrap_or_else(|_| Self::Bridge(text.to_owned()))
    }
}

impl fmt::Display for PluginError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Library(detail) | Self::Bridge(detail) => f.write_str(detail),
            Self::BadFile(b) => write!(f, "{b}"),
            Self::Create { plugin, step } => write!(f, "{plugin}: {step}"),
            Self::Unavailable(u) => write!(f, "{u}"),
            Self::Window(w) => write!(f, "{w}"),
            Self::Preset(p) => write!(f, "{p}"),
            Self::Locked(l) => write!(f, "{l}"),
        }
    }
}

impl fmt::Display for BadFile {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Self::NoClapEntry => "kein clap_entry – keine CLAP-Datei",
            Self::EmptyClapEntry => "clap_entry ist leer",
            Self::IncompatibleClap => "inkompatible CLAP-Version",
            Self::ClapInitMissing => "init fehlt",
            Self::ClapInitFailed => "Plugin-Bibliothek ließ sich nicht initialisieren",
            Self::NoFactory => "Plugin-Fabrik fehlt",
            Self::NoCreate => "create_plugin fehlt",
            Self::NoVst3Factory => "kein GetPluginFactory – keine VST3-Datei",
            Self::EmptyVst3Factory => "VST3-Fabrik ist leer",
            Self::ModuleEntry => "ModuleEntry fehlgeschlagen",
            Self::InitDll => "InitDll fehlgeschlagen",
            Self::InvalidClassId => "ungültige VST3-Klassen-Id",
        })
    }
}

impl fmt::Display for Step {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Self::Create => "Plugin ließ sich nicht erzeugen",
            Self::Init => "init fehlgeschlagen",
            Self::Activate => "Aktivieren fehlgeschlagen",
            Self::NoAudioOutput => "kein Audio-Ausgang",
            Self::Initialize => "initialize fehlgeschlagen",
            Self::NoProcessor => "kein Audioprozessor",
            Self::SetupProcessing => "setupProcessing fehlgeschlagen",
            Self::Component => "Komponente ließ sich nicht erzeugen",
            Self::AuNotFound => "Audio Unit nicht gefunden",
            Self::AuCreate => "Audio Unit ließ sich nicht erzeugen",
            Self::AuNoOutputFormat => "kein passendes Ausgabeformat",
            Self::AuInit => "initialisieren fehlgeschlagen",
            Self::Lv2NotFound => "LV2-Plugin nicht gefunden",
            Self::Lv2Instantiate => "LV2-Plugin ließ sich nicht erzeugen (fehlendes Host-Feature?)",
            Self::Browser => "Plugins gibt es im Browser nicht",
        })
    }
}

impl fmt::Display for Unavailable {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Self::AudioUnitsMacOnly => "Audio Units gibt es nur unter macOS",
            Self::Vst3System => "VST3 auf diesem System noch nicht unterstützt",
            Self::Lv2NoLilv => "LV2 nicht verfügbar (lilv nicht installiert)",
            Self::Browser => "Plugins gibt es im Browser nicht",
        })
    }
}

impl fmt::Display for WindowProblem {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::HostWindow(detail) => write!(f, "Fenster für das Plugin: {detail}"),
            Self::HostClass => f.write_str("Fensterklasse für Plugins ließ sich nicht anmelden"),
            Self::HostCreate => f.write_str("Fenster für das Plugin ließ sich nicht anlegen"),
            Self::MainThreadOnly => f.write_str("Plugin-Fenster nur im Hauptthread"),
            Self::NoWindow => f.write_str("Plugin hat kein eigenes Fenster"),
            Self::IncompleteGui => f.write_str("unvollständige clap.gui-Erweiterung"),
            Self::UnsupportedSystem => {
                f.write_str("Plugin-Fenster werden auf diesem System nicht unterstützt")
            }
            Self::NoMatchingApi => f.write_str("Plugin bietet kein passendes Fenster an"),
            Self::CannotCreate => f.write_str("Plugin konnte sein Fenster nicht anlegen"),
            Self::CannotShow => f.write_str("Plugin konnte sein Fenster nicht zeigen"),
            Self::CannotEmbed => f.write_str("Plugin-Fenster ließ sich nicht einbetten"),
            Self::CannotEmbedHere => f.write_str("Plugin-Fenster lässt sich hier nicht einbetten"),
            Self::NoX11Display => f.write_str("kein X11-Bildschirm"),
            Self::NeedsX11 => f.write_str("Plugin-Fenster brauchen X11 (unter Wayland: XWayland)"),
            Self::Lv2OnlyX11 => f.write_str("LV2-Fenster gibt es bisher nur unter X11"),
            Self::Lv2NoUi => f.write_str("keine LV2-Oberfläche in der Bibliothek"),
            Self::Lv2UiNotInLibrary => f.write_str("Oberfläche nicht in der Bibliothek gefunden"),
            Self::Lv2UiCreate => f.write_str("Plugin-Oberfläche ließ sich nicht erzeugen"),
            Self::Lv2NoMatchingUi => f.write_str("Plugin hat keine passende Oberfläche (X11)"),
            Self::NoController => f.write_str("Plugin hat keinen Controller"),
            Self::AuNoWindow => f.write_str("Audio Unit hat kein Fenster"),
            Self::AuNoContentView => f.write_str("Fenster ohne Inhaltsansicht"),
        }
    }
}

impl fmt::Display for PresetProblem {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Self::SetsParams => "Presets dieses Formats setzen Parameterwerte",
            Self::NotClapPreset => "kein CLAP-Preset",
            Self::NotLoaded => "Plugin konnte das Preset nicht laden",
        })
    }
}

impl fmt::Display for Lock {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Self::PluginCache => "Plugin-Cache gesperrt",
            Self::Vst3Modules => "VST3-Module gesperrt",
            Self::UiLibraries => "UI-Bibliotheken gesperrt",
        })
    }
}

impl std::error::Error for PluginError {}

impl From<WindowProblem> for PluginError {
    fn from(w: WindowProblem) -> Self {
        Self::Window(w)
    }
}

impl From<BadFile> for PluginError {
    fn from(b: BadFile) -> Self {
        Self::BadFile(b)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The fallback is the German text the UI showed before; the wire keeps the type.
    #[test]
    fn display_and_wire() {
        let e = PluginError::Create {
            plugin: "Hall".into(),
            step: Step::Activate,
        };
        assert_eq!(e.to_string(), "Hall: Aktivieren fehlgeschlagen");
        assert_eq!(PluginError::from_wire(&e.to_wire()), e);
        let w = PluginError::Window(WindowProblem::HostWindow("BadMatch".into()));
        assert_eq!(w.to_string(), "Fenster für das Plugin: BadMatch");
        assert_eq!(PluginError::from_wire(&w.to_wire()), w);
        // Text from an older helper or the bridge itself stays a bridge message.
        assert_eq!(
            PluginError::from_wire("the plugin's process does not answer"),
            PluginError::Bridge("the plugin's process does not answer".into())
        );
    }
}
