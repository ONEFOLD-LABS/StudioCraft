//! Fehler des Plugin-Hostings in der Sprache der Oberfläche (Issue #265). Der Plugin-Host
//! liefert [`PluginError`] mit deutschem `Display` als Rückfall; hier entsteht der angezeigte
//! Text. Meldungen des Systems (Bibliothek laden, X11) und der Bridge bleiben, wie sie sind.

use studiocraft_plugin_host::{
    BadFile, Lock, PluginError, PresetProblem, Step, Unavailable, WindowProblem,
};

use crate::i18n::{tr, trf};

/// Der Text eines Plugin-Fehlers für die Oberfläche.
pub fn plugin_error_text(e: &PluginError) -> String {
    match e {
        PluginError::Library(detail) | PluginError::Bridge(detail) => detail.clone(),
        PluginError::BadFile(b) => bad_file_text(*b).to_owned(),
        PluginError::Create { plugin, step } => format!("{plugin}: {}", step_text(*step)),
        PluginError::Unavailable(u) => unavailable_text(*u).to_owned(),
        PluginError::Window(w) => window_text(w),
        PluginError::Preset(p) => preset_text(*p).to_owned(),
        PluginError::Locked(l) => lock_text(*l).to_owned(),
    }
}

fn bad_file_text(b: BadFile) -> &'static str {
    match b {
        BadFile::NoClapEntry => tr("kein clap_entry – keine CLAP-Datei"),
        BadFile::EmptyClapEntry => tr("clap_entry ist leer"),
        BadFile::IncompatibleClap => tr("inkompatible CLAP-Version"),
        BadFile::ClapInitMissing => tr("init fehlt"),
        BadFile::ClapInitFailed => tr("Plugin-Bibliothek ließ sich nicht initialisieren"),
        BadFile::NoFactory => tr("Plugin-Fabrik fehlt"),
        BadFile::NoCreate => tr("create_plugin fehlt"),
        BadFile::NoVst3Factory => tr("kein GetPluginFactory – keine VST3-Datei"),
        BadFile::EmptyVst3Factory => tr("VST3-Fabrik ist leer"),
        BadFile::ModuleEntry => tr("ModuleEntry fehlgeschlagen"),
        BadFile::InitDll => tr("InitDll fehlgeschlagen"),
        BadFile::InvalidClassId => tr("ungültige VST3-Klassen-Id"),
    }
}

fn step_text(s: Step) -> &'static str {
    match s {
        Step::Create => tr("Plugin ließ sich nicht erzeugen"),
        Step::Init => tr("init fehlgeschlagen"),
        Step::Activate => tr("Aktivieren fehlgeschlagen"),
        Step::NoAudioOutput => tr("kein Audio-Ausgang"),
        Step::Initialize => tr("initialize fehlgeschlagen"),
        Step::NoProcessor => tr("kein Audioprozessor"),
        Step::SetupProcessing => tr("setupProcessing fehlgeschlagen"),
        Step::Component => tr("Komponente ließ sich nicht erzeugen"),
        Step::AuNotFound => tr("Audio Unit nicht gefunden"),
        Step::AuCreate => tr("Audio Unit ließ sich nicht erzeugen"),
        Step::AuNoOutputFormat => tr("kein passendes Ausgabeformat"),
        Step::AuInit => tr("initialisieren fehlgeschlagen"),
        Step::Lv2NotFound => tr("LV2-Plugin nicht gefunden"),
        Step::Lv2Instantiate => tr("LV2-Plugin ließ sich nicht erzeugen (fehlendes Host-Feature?)"),
        Step::Browser => tr("Plugins gibt es im Browser nicht"),
    }
}

fn unavailable_text(u: Unavailable) -> &'static str {
    match u {
        Unavailable::AudioUnitsMacOnly => tr("Audio Units gibt es nur unter macOS"),
        Unavailable::Vst3System => tr("VST3 auf diesem System noch nicht unterstützt"),
        Unavailable::Lv2NoLilv => tr("LV2 nicht verfügbar (lilv nicht installiert)"),
        Unavailable::Browser => tr("Plugins gibt es im Browser nicht"),
    }
}

fn window_text(w: &WindowProblem) -> String {
    use WindowProblem as W;
    let text = match w {
        // Meldung von X11: so, wie sie ist.
        W::HostWindow(detail) => {
            return trf("Fenster für das Plugin: {grund}", &[("grund", detail)]);
        }
        W::HostClass => tr("Fensterklasse für Plugins ließ sich nicht anmelden"),
        W::HostCreate => tr("Fenster für das Plugin ließ sich nicht anlegen"),
        W::MainThreadOnly => tr("Plugin-Fenster nur im Hauptthread"),
        W::NoWindow => tr("Plugin hat kein eigenes Fenster"),
        W::IncompleteGui => tr("unvollständige clap.gui-Erweiterung"),
        W::UnsupportedSystem => tr("Plugin-Fenster werden auf diesem System nicht unterstützt"),
        W::NoMatchingApi => tr("Plugin bietet kein passendes Fenster an"),
        W::CannotCreate => tr("Plugin konnte sein Fenster nicht anlegen"),
        W::CannotShow => tr("Plugin konnte sein Fenster nicht zeigen"),
        W::CannotEmbed => tr("Plugin-Fenster ließ sich nicht einbetten"),
        W::CannotEmbedHere => tr("Plugin-Fenster lässt sich hier nicht einbetten"),
        W::NoX11Display => tr("kein X11-Bildschirm"),
        W::NeedsX11 => tr("Plugin-Fenster brauchen X11 (unter Wayland: XWayland)"),
        W::Lv2OnlyX11 => tr("LV2-Fenster gibt es bisher nur unter X11"),
        W::Lv2NoUi => tr("keine LV2-Oberfläche in der Bibliothek"),
        W::Lv2UiNotInLibrary => tr("Oberfläche nicht in der Bibliothek gefunden"),
        W::Lv2UiCreate => tr("Plugin-Oberfläche ließ sich nicht erzeugen"),
        W::Lv2NoMatchingUi => tr("Plugin hat keine passende Oberfläche (X11)"),
        W::NoController => tr("Plugin hat keinen Controller"),
        W::AuNoWindow => tr("Audio Unit hat kein Fenster"),
        W::AuNoContentView => tr("Fenster ohne Inhaltsansicht"),
    };
    text.to_owned()
}

fn preset_text(p: PresetProblem) -> &'static str {
    match p {
        PresetProblem::SetsParams => tr("Presets dieses Formats setzen Parameterwerte"),
        PresetProblem::NotClapPreset => tr("kein CLAP-Preset"),
        PresetProblem::NotLoaded => tr("Plugin konnte das Preset nicht laden"),
    }
}

fn lock_text(l: Lock) -> &'static str {
    match l {
        Lock::PluginCache => tr("Plugin-Cache gesperrt"),
        Lock::Vst3Modules => tr("VST3-Module gesperrt"),
        Lock::UiLibraries => tr("UI-Bibliotheken gesperrt"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::i18n::{Language, set_language};

    /// English chosen: a plugin-host error from a real call shows in English; German again
    /// in German. System messages inside stay as they are.
    #[test]
    fn plugin_errors_follow_the_interface_language() {
        // A real plugin-host call: an Audio Unit that is not there (outside macOS: none at all).
        let info = studiocraft_plugin_host::PluginInfo {
            format: studiocraft_plugin_host::PluginFormat::Au,
            id: "aufx:none:none".into(),
            name: "Fehlt".into(),
            ..Default::default()
        };
        let err = match studiocraft_plugin_host::PluginInstance::new(&info, 48_000, 512) {
            Ok(_) => panic!("no such Audio Unit"),
            Err(e) => e,
        };
        set_language(Language::English);
        let en = plugin_error_text(&err);
        set_language(Language::German);
        let de = plugin_error_text(&err);
        assert_eq!(de, err.to_string(), "German is the Display text");
        assert_ne!(en, de, "translated: {en}");

        set_language(Language::English);
        let create = plugin_error_text(&PluginError::Create {
            plugin: "Synth".into(),
            step: Step::Activate,
        });
        let window = plugin_error_text(&PluginError::Window(WindowProblem::NeedsX11));
        let x11 = plugin_error_text(&PluginError::Window(WindowProblem::HostWindow(
            "BadMatch".into(),
        )));
        let bridged = plugin_error_text(&PluginError::from_wire(
            &PluginError::Preset(PresetProblem::NotLoaded).to_wire(),
        ));
        set_language(Language::German);
        assert_eq!(create, "Synth: activation failed");
        assert!(window.starts_with("Plugin windows need X11"), "{window}");
        assert_eq!(x11, "Window for the plugin: BadMatch", "X11 text stays");
        assert_eq!(
            bridged, "The plugin could not load the preset",
            "over the bridge"
        );
    }
}
