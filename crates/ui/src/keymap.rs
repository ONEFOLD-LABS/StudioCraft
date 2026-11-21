//! Zentrale Befehlsliste mit frei belegbaren Tasten (Spec „Tastaturkonzept“, Issue #1).
//!
//! Jeder Befehl hat eine feste Kennung, einen Namen, eine Kategorie und seine Standardtasten.
//! Belegungen, Konflikte und eigene Änderungen führt das gemeinsame craft-keymap (Issue #63);
//! StudioCraft behält die Befehlsliste, das Ausführen, die Makros und den Dialog. Eigene
//! Belegungen liegen als `keys.json` im Einstellungsordner: geänderte Befehle im Format von
//! craft-keymap (`{"command": "file.save", "keys": ["Cmd+Alt+S"]}`) und die Makros. Dasselbe
//! Format dient als Preset; alte `keys.txt`-Dateien und Presets werden weiter gelesen.

use crate::i18n::tr;
use craft_keymap as ck;

use egui::{Key, KeyboardShortcut, Modifiers};

use crate::menu::Action;
use crate::shortcuts::EditCommand;
use crate::tools::Tool;

/// Ein Befehl, den eine Taste auslösen kann.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub(crate) enum Cmd {
    File(Action),
    Edit(EditCommand),
    Tool(Tool),
    Undo,
    Redo,
    Delete,
    AddAudioTrack,
    Snap,
    ZoomIn,
    ZoomOut,
    PlayStop,
    ToggleCycle,
    /// Gespielte, nicht aufgenommene Noten als Part übernehmen.
    RetrospectiveRecord,
    ToStart,
    Metronome,
    PunchIn,
    PunchOut,
    LocatorsToSelection,
    ToggleMixer,
    /// Strg+E: Editor des gewählten Events im eigenen Fenster.
    OpenEditorWindow,
    ToggleInspector,
    ToggleRightZone,
    /// Dialog „Audio-Anschlüsse“.
    AudioConnections,
    InsertMarker,
    CycleMarkerFromLocators,
    /// Zu Positionsmarker mit dieser Nummer (3–9).
    GotoMarker(u32),
    /// Makro in diesem Platz (1–[`MACROS`]): mehrere Befehle nacheinander.
    Macro(u8),
    /// Benanntes Fensterlayout in diesem Platz abrufen (1–9).
    Layout(u8),
}

/// Anzahl der Makroplätze.
pub(crate) const MACROS: usize = 8;
/// Höchstens so viele Befehle je Makro (auch beim Lesen fremder Dateien).
pub(crate) const MAX_STEPS: usize = 64;
/// Höchstens so viele Zeichen im Namen eines Makros.
pub(crate) const MAX_NAME: usize = 64;

/// Ein Makro: Name und die Befehle, die es nacheinander ausführt (keine Makros darin).
#[derive(Debug, Clone, PartialEq, Default)]
pub(crate) struct Macro {
    pub name: String,
    pub steps: Vec<Cmd>,
}

impl Macro {
    pub fn is_empty(&self) -> bool {
        self.name.is_empty() && self.steps.is_empty()
    }
}

/// Beschreibung eines Befehls.
pub(crate) struct CommandInfo {
    pub cmd: Cmd,
    /// Feste Kennung für Dateien (ändert sich nie).
    pub id: &'static str,
    pub name: &'static str,
    pub category: &'static str,
    pub defaults: &'static [KeyboardShortcut],
}

impl CommandInfo {
    /// Name zum Anzeigen, in der eingestellten Sprache.
    pub fn label(&self) -> &'static str {
        tr(self.name)
    }

    /// Kategorie zum Anzeigen, in der eingestellten Sprache.
    pub fn group(&self) -> &'static str {
        tr(self.category)
    }
}

const fn k(m: Modifiers, key: Key) -> KeyboardShortcut {
    KeyboardShortcut::new(m, key)
}
const N: Modifiers = Modifiers::NONE;
const C: Modifiers = Modifiers::COMMAND;
const A: Modifiers = Modifiers::ALT;
const S: Modifiers = Modifiers::SHIFT;
const CS: Modifiers = Modifiers::COMMAND.plus(Modifiers::SHIFT);
const CA: Modifiers = Modifiers::COMMAND.plus(Modifiers::ALT);

macro_rules! cmd {
    ($cmd:expr, $id:literal, $name:literal, $cat:literal, [$($d:expr),*]) => {
        CommandInfo { cmd: $cmd, id: $id, name: $name, category: $cat, defaults: &[$($d),*] }
    };
}

/// Alle Befehle mit ihrer Standardbelegung (Spec „Standardbelegung“).
pub(crate) const COMMANDS: &[CommandInfo] = &[
    cmd!(
        Cmd::File(Action::New),
        "file.new",
        "Neu",
        "Datei",
        [k(C, Key::N)]
    ),
    cmd!(
        Cmd::File(Action::Open),
        "file.open",
        "Öffnen",
        "Datei",
        [k(C, Key::O)]
    ),
    cmd!(
        Cmd::File(Action::Save),
        "file.save",
        "Speichern",
        "Datei",
        [k(C, Key::S)]
    ),
    cmd!(
        Cmd::File(Action::SaveAs),
        "file.save_as",
        "Speichern unter",
        "Datei",
        [k(CS, Key::S)]
    ),
    cmd!(
        Cmd::File(Action::SaveVersion),
        "file.save_version",
        "Neue Version speichern",
        "Datei",
        [k(CA, Key::S)]
    ),
    cmd!(
        Cmd::File(Action::ImportAudio),
        "file.import_audio",
        "Audio importieren",
        "Datei",
        []
    ),
    cmd!(
        Cmd::File(Action::ImportMidi),
        "file.import_midi",
        "MIDI-Datei importieren",
        "Datei",
        []
    ),
    cmd!(
        Cmd::File(Action::ExportMixdown),
        "file.export_mixdown",
        "Audio-Mixdown exportieren",
        "Datei",
        []
    ),
    cmd!(
        Cmd::File(Action::ExportMidi),
        "file.export_midi",
        "MIDI-Datei exportieren",
        "Datei",
        []
    ),
    cmd!(
        Cmd::Undo,
        "edit.undo",
        "Rückgängig",
        "Bearbeiten",
        [k(C, Key::Z)]
    ),
    cmd!(
        Cmd::Redo,
        "edit.redo",
        "Wiederherstellen",
        "Bearbeiten",
        [k(CS, Key::Z), k(C, Key::Y)]
    ),
    cmd!(
        Cmd::Edit(EditCommand::Cut),
        "edit.cut",
        "Ausschneiden",
        "Bearbeiten",
        [k(C, Key::X)]
    ),
    cmd!(
        Cmd::Edit(EditCommand::Copy),
        "edit.copy",
        "Kopieren",
        "Bearbeiten",
        [k(C, Key::C)]
    ),
    cmd!(
        Cmd::Edit(EditCommand::Paste),
        "edit.paste",
        "Einfügen",
        "Bearbeiten",
        [k(C, Key::V)]
    ),
    cmd!(
        Cmd::Delete,
        "edit.delete",
        "Löschen",
        "Bearbeiten",
        [k(N, Key::Delete), k(N, Key::Backspace)]
    ),
    cmd!(
        Cmd::Edit(EditCommand::Duplicate),
        "edit.duplicate",
        "Duplizieren",
        "Bearbeiten",
        [k(C, Key::D)]
    ),
    cmd!(
        Cmd::Edit(EditCommand::Repeat),
        "edit.repeat",
        "Wiederholen …",
        "Bearbeiten",
        [k(C, Key::K)]
    ),
    cmd!(
        Cmd::Edit(EditCommand::SplitAtCursor),
        "edit.split_at_cursor",
        "Am Cursor teilen",
        "Bearbeiten",
        [k(A, Key::X)]
    ),
    cmd!(
        Cmd::Edit(EditCommand::SelectAll),
        "edit.select_all",
        "Alles auswählen",
        "Bearbeiten",
        [k(C, Key::A)]
    ),
    cmd!(
        Cmd::Snap,
        "edit.snap",
        "Snap an/aus",
        "Bearbeitung",
        [k(N, Key::J)]
    ),
    cmd!(
        Cmd::Edit(EditCommand::Quantize),
        "edit.quantize",
        "Quantisieren",
        "Bearbeitung",
        [k(N, Key::Q)]
    ),
    cmd!(
        Cmd::Edit(EditCommand::Crossfade),
        "edit.crossfade",
        "Crossfade",
        "Bearbeitung",
        [k(N, Key::X)]
    ),
    cmd!(
        Cmd::ZoomIn,
        "nav.zoom_in",
        "Horizontal einzoomen",
        "Navigation",
        [k(N, Key::H)]
    ),
    cmd!(
        Cmd::ZoomOut,
        "nav.zoom_out",
        "Horizontal auszoomen",
        "Navigation",
        [k(N, Key::G)]
    ),
    cmd!(
        Cmd::Edit(EditCommand::ZoomToProject),
        "nav.zoom_to_project",
        "Auf Projekt zoomen",
        "Navigation",
        [k(S, Key::F)]
    ),
    cmd!(
        Cmd::Edit(EditCommand::PrevEvent),
        "nav.prev_event",
        "Vorheriges Event",
        "Navigation",
        [k(N, Key::ArrowLeft)]
    ),
    cmd!(
        Cmd::Edit(EditCommand::NextEvent),
        "nav.next_event",
        "Nächstes Event",
        "Navigation",
        [k(N, Key::ArrowRight)]
    ),
    cmd!(
        Cmd::Edit(EditCommand::PrevTrack),
        "nav.prev_track",
        "Vorherige Spur",
        "Navigation",
        [k(N, Key::ArrowUp)]
    ),
    cmd!(
        Cmd::Edit(EditCommand::NextTrack),
        "nav.next_track",
        "Nächste Spur",
        "Navigation",
        [k(N, Key::ArrowDown)]
    ),
    cmd!(
        Cmd::Edit(EditCommand::OctaveUp),
        "edit.octave_up",
        "Noten eine Oktave höher",
        "Bearbeiten",
        [k(S, Key::ArrowUp)]
    ),
    cmd!(
        Cmd::Edit(EditCommand::OctaveDown),
        "edit.octave_down",
        "Noten eine Oktave tiefer",
        "Bearbeiten",
        [k(S, Key::ArrowDown)]
    ),
    cmd!(
        Cmd::Edit(EditCommand::ToggleFollow),
        "view.follow",
        "Der Wiedergabe folgen",
        "Navigation",
        [k(N, Key::F)]
    ),
    cmd!(
        Cmd::Edit(EditCommand::NudgeLeft),
        "nav.nudge_left",
        "Nudge links",
        "Navigation",
        [k(C, Key::ArrowLeft)]
    ),
    cmd!(
        Cmd::Edit(EditCommand::NudgeRight),
        "nav.nudge_right",
        "Nudge rechts",
        "Navigation",
        [k(C, Key::ArrowRight)]
    ),
    cmd!(
        Cmd::Edit(EditCommand::Shorter),
        "edit.shorter",
        "Länge kürzen",
        "Bearbeitung",
        [k(CS, Key::ArrowLeft)]
    ),
    cmd!(
        Cmd::Edit(EditCommand::Longer),
        "edit.longer",
        "Länge verlängern",
        "Bearbeitung",
        [k(CS, Key::ArrowRight)]
    ),
    cmd!(
        Cmd::Edit(EditCommand::Mute),
        "track.mute",
        "Mute",
        "Spur",
        [k(N, Key::M)]
    ),
    cmd!(
        Cmd::Edit(EditCommand::Solo),
        "track.solo",
        "Solo",
        "Spur",
        [k(N, Key::S)]
    ),
    cmd!(
        Cmd::Edit(EditCommand::RecordArm),
        "track.record_arm",
        "Aufnahme bereit",
        "Spur",
        [k(N, Key::R)]
    ),
    cmd!(
        Cmd::AddAudioTrack,
        "track.add_audio",
        "Audio-Spur hinzufügen",
        "Spur",
        [k(C, Key::T)]
    ),
    cmd!(
        Cmd::Edit(EditCommand::ToggleTransport),
        "window.transport",
        "Transportleiste",
        "Fenster",
        [k(N, Key::F2)]
    ),
    cmd!(
        Cmd::ToggleMixer,
        "window.mixer",
        "Mixer-Fenster",
        "Fenster",
        [k(N, Key::F3)]
    ),
    cmd!(
        Cmd::OpenEditorWindow,
        "window.editor",
        "Editor im eigenen Fenster",
        "Fenster",
        [k(C, Key::E)]
    ),
    cmd!(
        Cmd::ToggleInspector,
        "window.inspector",
        "Inspector",
        "Fenster",
        [k(A, Key::I)]
    ),
    cmd!(
        Cmd::Edit(EditCommand::ToggleLeftZone),
        "window.left_zone",
        "Linke Zone",
        "Fenster",
        [k(A, Key::L)]
    ),
    cmd!(
        Cmd::ToggleRightZone,
        "window.right_zone",
        "Rechte Zone",
        "Fenster",
        [k(A, Key::R)]
    ),
    cmd!(
        Cmd::AudioConnections,
        "view.audio_connections",
        "Audio-Anschlüsse",
        "Fenster",
        [k(N, Key::F4)]
    ),
    cmd!(
        Cmd::PlayStop,
        "transport.play_stop",
        "Start/Stopp",
        "Transport",
        [k(N, Key::Space)]
    ),
    cmd!(
        Cmd::ToggleCycle,
        "transport.cycle",
        "Cycle",
        "Transport",
        [k(N, Key::Slash)]
    ),
    // Standard ist Umschalt + Nummernblock-Mal (keypad.rs); hier frei belegbar.
    cmd!(
        Cmd::RetrospectiveRecord,
        "transport.retro_record",
        "Nachträgliche Aufnahme",
        "Transport",
        []
    ),
    cmd!(
        Cmd::ToStart,
        "transport.to_start",
        "Zum Anfang",
        "Transport",
        [k(N, Key::Home), k(N, Key::Comma)]
    ),
    cmd!(
        Cmd::Metronome,
        "transport.metronome",
        "Metronom",
        "Transport",
        [k(N, Key::C)]
    ),
    cmd!(
        Cmd::PunchIn,
        "transport.punch_in",
        "Punch-In",
        "Transport",
        [k(N, Key::I)]
    ),
    cmd!(
        Cmd::PunchOut,
        "transport.punch_out",
        "Punch-Out",
        "Transport",
        [k(N, Key::O)]
    ),
    cmd!(
        Cmd::LocatorsToSelection,
        "transport.locators_to_selection",
        "Locatoren auf Auswahl",
        "Transport",
        [k(N, Key::P)]
    ),
    cmd!(
        Cmd::Tool(Tool::Select),
        "tool.select",
        "Werkzeug Auswahl",
        "Werkzeuge",
        [k(N, Key::Num1)]
    ),
    cmd!(
        Cmd::Tool(Tool::Range),
        "tool.range",
        "Werkzeug Bereich",
        "Werkzeuge",
        [k(N, Key::Num2)]
    ),
    cmd!(
        Cmd::Tool(Tool::Split),
        "tool.split",
        "Werkzeug Trennen",
        "Werkzeuge",
        [k(N, Key::Num3)]
    ),
    cmd!(
        Cmd::Tool(Tool::Glue),
        "tool.glue",
        "Werkzeug Kleben",
        "Werkzeuge",
        [k(N, Key::Num4)]
    ),
    cmd!(
        Cmd::Tool(Tool::Erase),
        "tool.erase",
        "Werkzeug Löschen",
        "Werkzeuge",
        [k(N, Key::Num5)]
    ),
    cmd!(
        Cmd::Tool(Tool::Zoom),
        "tool.zoom",
        "Werkzeug Zoom",
        "Werkzeuge",
        [k(N, Key::Num6)]
    ),
    cmd!(
        Cmd::Tool(Tool::Mute),
        "tool.mute",
        "Werkzeug Stumm",
        "Werkzeuge",
        [k(N, Key::Num7)]
    ),
    cmd!(
        Cmd::Tool(Tool::Draw),
        "tool.draw",
        "Werkzeug Zeichnen",
        "Werkzeuge",
        [k(N, Key::Num8)]
    ),
    cmd!(
        Cmd::Tool(Tool::Play),
        "tool.play",
        "Werkzeug Wiedergabe",
        "Werkzeuge",
        [k(N, Key::Num9)]
    ),
    cmd!(
        Cmd::Tool(Tool::TimeWarp),
        "tool.time_warp",
        "Werkzeug Zeitkorrektur",
        "Werkzeuge",
        [k(N, Key::Num0)]
    ),
    cmd!(
        Cmd::Tool(Tool::Comp),
        "tool.comp",
        "Werkzeug Comping",
        "Werkzeuge",
        []
    ),
    cmd!(
        Cmd::InsertMarker,
        "marker.insert",
        "Marker einfügen",
        "Marker",
        [k(N, Key::Insert)]
    ),
    cmd!(
        Cmd::CycleMarkerFromLocators,
        "marker.cycle_from_locators",
        "Cycle-Marker aus Locatoren",
        "Marker",
        []
    ),
    // Der Nummernblock 3–9 springt fest zu diesen Markern (keypad.rs); hier ohne Taste,
    // frei belegbar.
    cmd!(
        Cmd::GotoMarker(3),
        "marker.goto_3",
        "Zu Marker 3",
        "Marker",
        []
    ),
    cmd!(
        Cmd::GotoMarker(4),
        "marker.goto_4",
        "Zu Marker 4",
        "Marker",
        []
    ),
    cmd!(
        Cmd::GotoMarker(5),
        "marker.goto_5",
        "Zu Marker 5",
        "Marker",
        []
    ),
    cmd!(
        Cmd::GotoMarker(6),
        "marker.goto_6",
        "Zu Marker 6",
        "Marker",
        []
    ),
    cmd!(
        Cmd::GotoMarker(7),
        "marker.goto_7",
        "Zu Marker 7",
        "Marker",
        []
    ),
    cmd!(
        Cmd::GotoMarker(8),
        "marker.goto_8",
        "Zu Marker 8",
        "Marker",
        []
    ),
    cmd!(
        Cmd::GotoMarker(9),
        "marker.goto_9",
        "Zu Marker 9",
        "Marker",
        []
    ),
    cmd!(
        Cmd::Layout(1),
        "window.layout_1",
        "Layout 1",
        "Fenster",
        [k(CA, Key::Num1)]
    ),
    cmd!(
        Cmd::Layout(2),
        "window.layout_2",
        "Layout 2",
        "Fenster",
        [k(CA, Key::Num2)]
    ),
    cmd!(
        Cmd::Layout(3),
        "window.layout_3",
        "Layout 3",
        "Fenster",
        [k(CA, Key::Num3)]
    ),
    cmd!(
        Cmd::Layout(4),
        "window.layout_4",
        "Layout 4",
        "Fenster",
        [k(CA, Key::Num4)]
    ),
    cmd!(
        Cmd::Layout(5),
        "window.layout_5",
        "Layout 5",
        "Fenster",
        [k(CA, Key::Num5)]
    ),
    cmd!(
        Cmd::Layout(6),
        "window.layout_6",
        "Layout 6",
        "Fenster",
        [k(CA, Key::Num6)]
    ),
    cmd!(
        Cmd::Layout(7),
        "window.layout_7",
        "Layout 7",
        "Fenster",
        [k(CA, Key::Num7)]
    ),
    cmd!(
        Cmd::Layout(8),
        "window.layout_8",
        "Layout 8",
        "Fenster",
        [k(CA, Key::Num8)]
    ),
    cmd!(
        Cmd::Layout(9),
        "window.layout_9",
        "Layout 9",
        "Fenster",
        [k(CA, Key::Num9)]
    ),
    cmd!(Cmd::Macro(1), "macro.1", "Makro 1", "Makros", []),
    cmd!(Cmd::Macro(2), "macro.2", "Makro 2", "Makros", []),
    cmd!(Cmd::Macro(3), "macro.3", "Makro 3", "Makros", []),
    cmd!(Cmd::Macro(4), "macro.4", "Makro 4", "Makros", []),
    cmd!(Cmd::Macro(5), "macro.5", "Makro 5", "Makros", []),
    cmd!(Cmd::Macro(6), "macro.6", "Makro 6", "Makros", []),
    cmd!(Cmd::Macro(7), "macro.7", "Makro 7", "Makros", []),
    cmd!(Cmd::Macro(8), "macro.8", "Makro 8", "Makros", []),
];

pub(crate) fn info(cmd: Cmd) -> Option<&'static CommandInfo> {
    COMMANDS.iter().find(|c| c.cmd == cmd)
}

/// Größte Datei mit Tastaturbefehlen, die gelesen wird.
const MAX_FILE: u64 = 1 << 20;

/// Kennung eines Befehls für craft-keymap.
fn command_ref(cmd: Cmd) -> Option<ck::CommandRef> {
    info(cmd).map(|i| ck::CommandRef::from(i.id))
}

/// egui-Taste als Akkord von craft-keymap. Strg und Cmd sind wie im alten Dateiformat dieselbe
/// Befehlstaste. `None` für Tasten ohne Gegenstück (Kopieren, Einfügen, …).
fn chord(k: KeyboardShortcut) -> Option<ck::Chord> {
    let m = k.modifiers;
    let mods = ck::Mods {
        cmd: m.command || m.ctrl || m.mac_cmd,
        ctrl: false,
        alt: m.alt,
        shift: m.shift,
    };
    Some(ck::Chord {
        mods,
        key: ck::Key::from_egui(k.logical_key)?,
    })
}

/// Akkord als egui-Taste; die Mac-Control-Taste zählt wie die Befehlstaste.
fn shortcut(c: &ck::Chord) -> KeyboardShortcut {
    let m = Modifiers {
        alt: c.mods.alt,
        shift: c.mods.shift,
        command: c.mods.cmd || c.mods.ctrl,
        ..Modifiers::NONE
    };
    KeyboardShortcut::new(m, c.key.to_egui())
}

fn binding(k: KeyboardShortcut) -> Option<ck::Binding> {
    chord(k).map(ck::Binding::new)
}

/// Die Befehlsliste als Katalog für craft-keymap.
fn catalog() -> Vec<ck::Bindable> {
    COMMANDS
        .iter()
        .map(|c| {
            let mut b = ck::Bindable::new(c.id, c.name, ck::Scope::app())
                .with_path(vec![c.category.to_owned(), c.name.to_owned()]);
            for d in c.defaults.iter().filter_map(|d| binding(*d)) {
                b = b.with_default(d);
            }
            b
        })
        .collect()
}

/// Die aktuelle Belegung: Standard plus eigene Änderungen (craft-keymap) und die Makros.
#[derive(Debug, Clone)]
pub(crate) struct Keymap {
    inner: ck::Keymap,
    /// Makroplätze 1–[`MACROS`] (Index 0 = Makro 1).
    pub macros: [Macro; MACROS],
}

impl Default for Keymap {
    fn default() -> Self {
        Self::with_overrides(ck::Overrides::default())
    }
}

impl PartialEq for Keymap {
    fn eq(&self, other: &Self) -> bool {
        self.inner.overrides() == other.inner.overrides() && self.macros == other.macros
    }
}

/// `keys.json`: geänderte Befehle im gemeinsamen Format von craft-keymap und die Makros.
#[derive(serde::Serialize, serde::Deserialize, Default)]
#[serde(default)]
struct KeysFile {
    format: u32,
    #[serde(with = "craft_keymap::keymap::overrides_list")]
    shortcuts: ck::Overrides,
    macros: Vec<MacroEntry>,
}

#[derive(serde::Serialize, serde::Deserialize, Default)]
#[serde(default)]
struct MacroEntry {
    slot: u8,
    name: String,
    steps: Vec<String>,
}

impl Keymap {
    fn with_overrides(overrides: ck::Overrides) -> Self {
        let inner = ck::Keymap::new(catalog(), ck::Scopes::new(), ck::Platform::current())
            .with_overrides(overrides);
        Self {
            inner,
            macros: Default::default(),
        }
    }

    /// Tasten eines Befehls.
    pub fn bindings(&self, cmd: Cmd) -> Vec<KeyboardShortcut> {
        let Some(r) = command_ref(cmd) else {
            return Vec::new();
        };
        self.inner
            .bindings(&r)
            .iter()
            .filter(|b| !b.trigger.double)
            .map(|b| shortcut(&b.trigger.chord))
            .collect()
    }

    /// Setzt die Tasten eines Befehls; andere Befehle behalten ihre.
    pub fn set(&mut self, cmd: Cmd, bindings: Vec<KeyboardShortcut>) {
        let Some(r) = command_ref(cmd) else {
            return;
        };
        if info(cmd).is_some_and(|i| i.defaults == bindings.as_slice()) {
            self.inner.reset(&r);
        } else if bindings.is_empty() {
            self.inner.clear(&r);
        } else {
            let list = bindings.into_iter().filter_map(binding).collect();
            // Keep: ein Befehl verliert hier keine Taste an einen anderen.
            let _ = self.inner.set_bindings(&r, list, ck::Policy::Keep);
        }
    }

    /// Weist `key` dem Befehl `cmd` zu und nimmt sie jedem anderen Befehl weg.
    pub fn assign(&mut self, cmd: Cmd, key: KeyboardShortcut) -> Result<(), String> {
        let unknown = || crate::i18n::trf("{key}: nicht belegbar", &[("key", &display_key(key))]);
        let (Some(r), Some(b)) = (command_ref(cmd), binding(key)) else {
            return Err(unknown());
        };
        self.inner
            .assign(&r, b, ck::Policy::Steal)
            .map(|_| ())
            .map_err(|_| unknown())
    }

    /// Der Befehl, der `key` schon hat (außer `except`).
    pub fn owner(&self, key: KeyboardShortcut, except: Cmd) -> Option<Cmd> {
        COMMANDS
            .iter()
            .map(|c| c.cmd)
            .find(|&c| c != except && self.bindings(c).contains(&key))
    }

    /// Alle Belegungen (für die Tastenverarbeitung).
    pub fn all(&self) -> Vec<(KeyboardShortcut, Cmd)> {
        COMMANDS
            .iter()
            .flat_map(|c| self.bindings(c.cmd).into_iter().map(move |k| (k, c.cmd)))
            .collect()
    }

    pub fn is_default(&self) -> bool {
        self.inner.overrides().is_empty() && self.macros.iter().all(Macro::is_empty)
    }

    /// Makro im Platz `slot` (1–[`MACROS`]).
    pub fn macro_slot(&self, slot: u8) -> Option<&Macro> {
        self.macros.get(usize::from(slot).checked_sub(1)?)
    }

    pub fn macro_slot_mut(&mut self, slot: u8) -> Option<&mut Macro> {
        self.macros.get_mut(usize::from(slot).checked_sub(1)?)
    }

    /// Name für Menüs und Dialog: „Makro 2: Aufräumen“, ohne eigenen Namen „Makro 2“.
    pub fn macro_label(&self, slot: u8) -> String {
        match self.macro_slot(slot).map(|m| m.name.as_str()) {
            Some(name) if !name.is_empty() => {
                crate::i18n::trf("Makro {slot}: {name}", &[("slot", &slot), ("name", &name)])
            }
            _ => crate::i18n::trf("Makro {slot}", &[("slot", &slot)]),
        }
    }

    /// Was in den gespeicherten Belegungen nicht verwendbar war (die Einträge bleiben erhalten).
    fn shortcut_problems(&self) -> Vec<String> {
        self.inner
            .problems()
            .iter()
            .map(|p| match p {
                ck::Problem::UnknownCommand { key } => crate::i18n::trf(
                    "{wo}: unbekannter Befehl {step}",
                    &[("wo", &tr("Tasten")), ("step", key)],
                ),
                ck::Problem::BadBinding { key, text, .. } => crate::i18n::trf(
                    "{cmd}: unbekannte Taste {key}",
                    &[("cmd", key), ("key", text)],
                ),
                _ => tr("Ein Eintrag der Tastaturbefehle ist nicht verwendbar").to_owned(),
            })
            .collect()
    }

    /// Als JSON (`keys.json`, Presets): nur Abweichungen vom Standard und belegte Makros.
    pub fn to_json(&self) -> Result<String, String> {
        let macros = self
            .macros
            .iter()
            .zip(1u8..)
            .filter(|(m, _)| !m.is_empty())
            .map(|(m, slot)| MacroEntry {
                slot,
                name: m.name.clone(),
                steps: m
                    .steps
                    .iter()
                    .filter_map(|c| info(*c).map(|i| i.id.to_owned()))
                    .collect(),
            })
            .collect();
        let file = KeysFile {
            format: 1,
            shortcuts: self.inner.overrides().clone(),
            macros,
        };
        serde_json::to_string_pretty(&file).map_err(|e| e.to_string())
    }

    /// Liest `keys.json`; Unbrauchbares wird gemeldet, der Rest gilt.
    pub fn from_json(text: &str) -> Result<(Self, Vec<String>), String> {
        let file: KeysFile = serde_json::from_str(text).map_err(|e| e.to_string())?;
        let mut map = Self::with_overrides(file.shortcuts);
        let mut problems = map.shortcut_problems();
        for e in file.macros.iter().take(MACROS * 4) {
            let wo = crate::i18n::trf("Makro {slot}", &[("slot", &e.slot)]);
            let Some(m) = map.macro_slot_mut(e.slot) else {
                problems.push(crate::i18n::trf(
                    "unbekanntes Makro {slot}",
                    &[("slot", &e.slot)],
                ));
                continue;
            };
            m.name = e.name.chars().take(MAX_NAME).collect();
            fill_steps(m, e.steps.iter().map(String::as_str), &wo, &mut problems);
        }
        Ok((map, problems))
    }

    /// Liest eine Makrozeile (`macro.N.name` oder `macro.N.steps`); `None`, wenn es keine ist.
    fn macro_line(
        &mut self,
        id: &str,
        value: &str,
        line: usize,
        problems: &mut Vec<String>,
    ) -> Option<()> {
        let rest = id.strip_prefix("macro.")?;
        let (slot, field) = rest.split_once('.')?;
        let Some(m) = slot.parse::<u8>().ok().and_then(|s| self.macro_slot_mut(s)) else {
            problems.push(crate::i18n::trf(
                "Zeile {line}: unbekanntes Makro {slot}",
                &[("line", &line), ("slot", &slot)],
            ));
            return Some(());
        };
        match field {
            "name" => m.name = value.chars().take(MAX_NAME).collect(),
            "steps" => {
                let wo = crate::i18n::trf("Zeile {line}", &[("line", &line)]);
                let steps = value.split(',').map(str::trim).filter(|s| !s.is_empty());
                fill_steps(m, steps, &wo, problems);
            }
            _ => problems.push(crate::i18n::trf(
                "Zeile {line}: unbekanntes Feld {field}",
                &[("line", &line), ("field", &field)],
            )),
        }
        Some(())
    }

    /// Liest das alte Textformat (`keys.txt` bis 0.9, alte Presets): eine Zeile je geändertem
    /// Befehl (`kennung = Strg+Umschalt+S, Strg+Y`) und `macro.N.*`-Zeilen. Unbekannte Zeilen und
    /// Tasten werden übersprungen und gemeldet.
    pub fn from_legacy_text(text: &str) -> (Self, Vec<String>) {
        let mut map = Self::default();
        let mut problems = Vec::new();
        for (n, line) in text.lines().enumerate() {
            let line = line.trim();
            if line.is_empty() || line.starts_with('#') {
                continue;
            }
            let Some((id, keys)) = line.split_once('=') else {
                problems.push(crate::i18n::trf(
                    "Zeile {line}: kein „=“",
                    &[("line", &(n + 1))],
                ));
                continue;
            };
            if map
                .macro_line(id.trim(), keys.trim(), n + 1, &mut problems)
                .is_some()
            {
                continue;
            }
            let Some(c) = COMMANDS.iter().find(|c| c.id == id.trim()) else {
                problems.push(crate::i18n::trf(
                    "{wo}: unbekannter Befehl {step}",
                    &[
                        (
                            "wo",
                            &crate::i18n::trf("Zeile {line}", &[("line", &(n + 1))]),
                        ),
                        ("step", &id.trim()),
                    ],
                ));
                continue;
            };
            let mut b = Vec::new();
            for k in keys.split(',').map(str::trim).filter(|k| !k.is_empty()) {
                match parse_key(k) {
                    Some(k) => b.push(k),
                    None => problems.push(crate::i18n::trf(
                        "Zeile {line}: unbekannte Taste {key}",
                        &[("line", &(n + 1)), ("key", &k)],
                    )),
                }
            }
            map.set(c.cmd, b);
        }
        (map, problems)
    }

    /// Liest ein Preset: JSON oder das alte Textformat.
    pub fn from_preset(text: &str) -> (Self, Vec<String>) {
        if text.trim_start().starts_with('{') {
            Self::from_json(text).unwrap_or_else(|e| (Self::default(), vec![e]))
        } else {
            Self::from_legacy_text(text)
        }
    }
}

/// Füllt die Schritte eines Makros; Makros darin, unbekannte Befehle und zu viele Schritte
/// werden gemeldet (`wo`: Zeile oder Makro).
fn fill_steps<'a>(
    m: &mut Macro,
    steps: impl Iterator<Item = &'a str>,
    wo: &str,
    problems: &mut Vec<String>,
) {
    m.steps.clear();
    for step in steps {
        match COMMANDS.iter().find(|c| c.id == step) {
            Some(c) if matches!(c.cmd, Cmd::Macro(_)) => problems.push(crate::i18n::trf(
                "{wo}: Makro im Makro ({step})",
                &[("wo", &wo), ("step", &step)],
            )),
            Some(_) if m.steps.len() >= MAX_STEPS => {
                problems.push(crate::i18n::trf(
                    "{wo}: mehr als {max} Befehle",
                    &[("wo", &wo), ("max", &MAX_STEPS)],
                ));
                break;
            }
            Some(c) => m.steps.push(c.cmd),
            None => problems.push(crate::i18n::trf(
                "{wo}: unbekannter Befehl {step}",
                &[("wo", &wo), ("step", &step)],
            )),
        }
    }
}

/// Taste als Text des alten Dateiformats: `Strg+Umschalt+S` (nur noch für Tests).
#[cfg(test)]
pub(crate) fn format_key(k: KeyboardShortcut) -> String {
    let m = k.modifiers;
    let mut parts = Vec::new();
    if m.command || m.ctrl {
        parts.push("Strg");
    }
    if m.alt {
        parts.push("Alt");
    }
    if m.shift {
        parts.push("Umschalt");
    }
    let mut s = parts.join("+");
    if !s.is_empty() {
        s.push('+');
    }
    s.push_str(k.logical_key.name());
    s
}

/// Taste zum Anzeigen, in der eingestellten Sprache (deutsch: Strg, Umschalt, Entf, Rücktaste,
/// Leertaste, Pos1, Pfeiltasten). Dateien nutzen die Schreibweise von craft-keymap.
pub(crate) fn display_key(k: KeyboardShortcut) -> String {
    let m = k.modifiers;
    let mut parts: Vec<&str> = Vec::new();
    if m.command || m.ctrl {
        parts.push(tr("Strg"));
    }
    if m.alt {
        parts.push("Alt");
    }
    if m.shift {
        parts.push(tr("Umschalt"));
    }
    parts.push(match k.logical_key {
        Key::Delete => tr("Entf"),
        Key::Backspace => tr("Rücktaste"),
        Key::Space => tr("Leertaste"),
        Key::Home => tr("Pos1"),
        Key::End => tr("Ende"),
        Key::Escape => "Esc",
        Key::Enter => tr("Eingabe"),
        Key::ArrowLeft => tr("Pfeil links"),
        Key::ArrowRight => tr("Pfeil rechts"),
        Key::ArrowUp => tr("Pfeil hoch"),
        Key::ArrowDown => tr("Pfeil runter"),
        Key::Comma => ",",
        Key::Slash => "/",
        _ => k.logical_key.name(),
    });
    parts.join("+")
}

/// Liest eine Taste des alten Dateiformats.
pub(crate) fn parse_key(text: &str) -> Option<KeyboardShortcut> {
    let mut m = Modifiers::NONE;
    let parts: Vec<&str> = text.split('+').map(str::trim).collect();
    let (key, mods) = parts.split_last()?;
    for p in mods {
        match p.to_ascii_lowercase().as_str() {
            "strg" | "ctrl" | "cmd" => m = m.plus(Modifiers::COMMAND),
            "alt" => m = m.plus(Modifiers::ALT),
            "umschalt" | "shift" => m = m.plus(Modifiers::SHIFT),
            _ => return None,
        }
    }
    Some(KeyboardShortcut::new(m, Key::from_name(key)?))
}

/// Liest höchstens [`MAX_FILE`] Bytes.
fn read_capped(path: &std::path::Path) -> std::io::Result<String> {
    use std::io::Read;
    let mut text = String::new();
    std::fs::File::open(path)?
        .take(MAX_FILE)
        .read_to_string(&mut text)?;
    Ok(text)
}

impl Keymap {
    /// Eigene Belegung laden: `keys.json`; gibt es nur die alte `keys.txt`, wird sie einmal
    /// gelesen und als `keys.json` gespeichert (die alte Datei bleibt liegen). Ohne Datei der
    /// Standard. Zurück kommen auch die Probleme beim Lesen.
    pub fn load() -> (Self, Vec<String>) {
        let Some(dir) = crate::settings::config_dir() else {
            return (Self::default(), Vec::new());
        };
        Self::load_from(&dir)
    }

    fn load_from(dir: &std::path::Path) -> (Self, Vec<String>) {
        match read_capped(&dir.join("keys.json")) {
            Ok(t) => return Self::from_json(&t).unwrap_or_else(|e| (Self::default(), vec![e])),
            Err(e) if e.kind() != std::io::ErrorKind::NotFound => {
                return (Self::default(), vec![e.to_string()]);
            }
            Err(_) => {}
        }
        let Ok(old) = read_capped(&dir.join("keys.txt")) else {
            return (Self::default(), Vec::new());
        };
        let (map, mut problems) = Self::from_legacy_text(&old);
        if let Err(e) = map.save_to(dir) {
            problems.push(e);
        }
        (map, problems)
    }

    pub fn save(&self) -> Result<(), String> {
        let dir = crate::settings::config_dir().ok_or(tr("kein Ort für Einstellungen"))?;
        self.save_to(&dir)
    }

    fn save_to(&self, dir: &std::path::Path) -> Result<(), String> {
        std::fs::create_dir_all(dir).map_err(|e| format!("{}: {e}", dir.display()))?;
        let f = dir.join("keys.json");
        std::fs::write(&f, self.to_json()?).map_err(|e| format!("{}: {e}", f.display()))
    }
}

/// Wie `InputState::consume_key`, aber die Modifier müssen genau passen: `R` soll nicht auch
/// bei Alt+R auslösen.
pub(crate) fn consume_exact(i: &mut egui::InputState, k: KeyboardShortcut) -> bool {
    let mut hit = false;
    i.events.retain(|e| {
        let is = matches!(e, egui::Event::Key { key, pressed: true, modifiers, .. }
            if *key == k.logical_key && modifiers.matches_exact(k.modifiers));
        hit |= is;
        !is
    });
    hit
}

/// Zustand des Dialogs „Tastaturbefehle“.
#[derive(Debug, Clone, Default)]
pub(crate) struct KeyDialog {
    pub open: bool,
    pub filter: String,
    /// Wartet auf eine Taste für diesen Befehl.
    pub capturing: Option<Cmd>,
    /// Gedrückte Taste gehört schon einem anderen Befehl: (für, Taste, bisheriger Besitzer).
    pub conflict: Option<(Cmd, KeyboardShortcut, Cmd)>,
    pub message: Option<String>,
    /// Änderungen in `keys.json` und `settings.json` speichern (nur in der App, nicht in Tests).
    pub persist: bool,
}

impl crate::ProjectWindow {
    /// Eigene Belegung aus dem Einstellungsordner laden; Änderungen werden ab jetzt gespeichert.
    pub fn load_keymap(&mut self) {
        let (map, problems) = Keymap::load();
        self.keymap = map;
        self.key_dialog.message = (!problems.is_empty()).then(|| problems.join("; "));
        self.key_dialog.persist = true;
    }

    /// Erste Taste eines Befehls für Menüs, im Stil des Systems; leer ohne Taste.
    pub(crate) fn key_label(&self, ctx: &egui::Context, cmd: Cmd) -> String {
        self.keymap
            .bindings(cmd)
            .first()
            .map(|k| ctx.format_shortcut(k))
            .unwrap_or_default()
    }

    /// Verbraucht die belegten Tasten und führt ihre Befehle aus.
    pub(crate) fn dispatch_keys(&mut self, ui: &egui::Ui) {
        if self.key_dialog.capturing.is_some() {
            return;
        }
        let all = self.keymap.all();
        let hits: Vec<Cmd> = ui.input_mut(|i| {
            all.iter()
                .filter(|(k, _)| consume_exact(i, *k))
                .map(|(_, c)| *c)
                .collect()
        });
        for c in hits {
            self.run_cmd(ui, c);
        }
    }

    /// Führt einen Befehl aus (Taste, Menü, Dialog).
    pub(crate) fn run_cmd(&mut self, ui: &egui::Ui, cmd: Cmd) {
        match cmd {
            Cmd::File(a) => self.request(ui, a),
            Cmd::Edit(e) => self.run_edit_command(e),
            Cmd::Tool(t) => self.tool = t,
            Cmd::Undo => self.undo(),
            Cmd::Redo => self.redo(),
            Cmd::Delete => {
                if self.key_editor.focused {
                    self.delete_selected_notes();
                } else {
                    self.delete_selection();
                }
            }
            Cmd::AddAudioTrack => self.add_audio_track(),
            Cmd::Snap => self.snap = !self.snap,
            Cmd::ZoomIn => self.view.zoom(1.25, None),
            Cmd::ZoomOut => self.view.zoom(0.8, None),
            Cmd::PlayStop => self.toggle_play(),
            Cmd::ToggleCycle => self.toggle_cycle(),
            Cmd::RetrospectiveRecord => self.retrospective_record(),
            Cmd::ToStart => self.locate(0),
            Cmd::Metronome => self.toggle_metronome(),
            Cmd::PunchIn => self.punch_in = !self.punch_in,
            Cmd::PunchOut => self.punch_out = !self.punch_out,
            Cmd::LocatorsToSelection => self.locators_to_selection(),
            // F3: Mixer als eigenes Fenster (Spec „Fenster-Architektur“); in der unteren
            // Zone bleibt er über den Reiter und das Menü Ansicht erreichbar.
            Cmd::ToggleMixer => self.toggle_mixer_window(),
            Cmd::OpenEditorWindow => self.open_editor_window(),
            // Alt+I: Inspector-Spalte; ist die linke Zone zu, öffnet sie mit Inspector.
            Cmd::ToggleInspector => {
                if self.show_inspector {
                    self.show_inspector_column = !self.show_inspector_column;
                } else {
                    self.show_inspector = true;
                    self.show_inspector_column = true;
                }
            }
            Cmd::ToggleRightZone => self.show_right = !self.show_right,
            Cmd::AudioConnections => self.io_dialog = !self.io_dialog,
            Cmd::InsertMarker => self.insert_marker(),
            Cmd::CycleMarkerFromLocators => self.cycle_marker_from_locators(),
            Cmd::GotoMarker(id) => self.locate_marker(id),
            Cmd::Macro(slot) => self.run_macro(ui, slot),
            Cmd::Layout(slot) => self.recall_layout(slot),
        }
    }

    /// Führt die Befehle eines Makros nacheinander aus; Makros darin werden übersprungen.
    pub(crate) fn run_macro(&mut self, ui: &egui::Ui, slot: u8) {
        let steps = self
            .keymap
            .macro_slot(slot)
            .map(|m| m.steps.clone())
            .unwrap_or_default();
        for step in steps {
            if !matches!(step, Cmd::Macro(_)) {
                self.run_cmd(ui, step);
            }
        }
    }

    pub(crate) fn keymap_changed(&mut self) {
        if !self.key_dialog.persist {
            return;
        }
        if let Err(e) = self.keymap.save() {
            self.key_dialog.message = Some(e);
        }
    }

    /// Der Dialog „Tastaturbefehle“: suchen, Tasten setzen und entfernen, Standard, Presets.
    pub(crate) fn key_commands_dialog(&mut self, ctx: &egui::Context) {
        if !self.key_dialog.open {
            return;
        }
        let mut open = true;
        egui::Window::new(tr("Tastaturbefehle"))
            .open(&mut open)
            .default_size([560.0, 520.0])
            .show(ctx, |ui| self.key_commands_ui(ui));
        if !open {
            self.key_dialog.open = false;
            self.key_dialog.capturing = None;
            self.key_dialog.conflict = None;
        }
    }

    fn key_commands_ui(&mut self, ui: &mut egui::Ui) {
        // Taste aufnehmen: das erste gedrückte Kürzel gilt, Escape bricht ab.
        if let Some(cmd) = self.key_dialog.capturing {
            let pressed = ui.input_mut(|i| {
                let k = i.events.iter().find_map(|e| match e {
                    egui::Event::Key {
                        key,
                        pressed: true,
                        modifiers,
                        ..
                    } => Some(KeyboardShortcut::new(*modifiers, *key)),
                    _ => None,
                });
                if k.is_some() {
                    i.events.retain(|e| !matches!(e, egui::Event::Key { .. }));
                }
                k
            });
            if let Some(k) = pressed {
                self.key_dialog.capturing = None;
                // Nur die Modifier, die das Format kennt (Strg/Cmd, Alt, Umschalt).
                let mods = Modifiers {
                    alt: k.modifiers.alt,
                    shift: k.modifiers.shift,
                    command: k.modifiers.command || k.modifiers.ctrl,
                    ..Modifiers::NONE
                };
                let k = KeyboardShortcut::new(mods, k.logical_key);
                if k == KeyboardShortcut::new(Modifiers::NONE, Key::Escape) {
                    // abgebrochen
                } else if let Some(owner) = self.keymap.owner(k, cmd) {
                    self.key_dialog.conflict = Some((cmd, k, owner));
                } else {
                    match self.keymap.assign(cmd, k) {
                        Ok(()) => self.keymap_changed(),
                        Err(e) => self.key_dialog.message = Some(e),
                    }
                }
            }
        }
        ui.horizontal(|ui| {
            let l = ui.label(tr("Suche"));
            ui.add(egui::TextEdit::singleline(&mut self.key_dialog.filter).desired_width(200.0))
                .labelled_by(l.id);
            if ui
                .add_enabled(
                    !self.keymap.is_default(),
                    egui::Button::new(tr("Alle auf Standard")),
                )
                .clicked()
            {
                self.keymap = Keymap::default();
                self.keymap_changed();
            }
            #[cfg(not(target_arch = "wasm32"))]
            {
                if ui.button(tr("Exportieren …")).clicked()
                    && let Some(p) = rfd::FileDialog::new()
                        .add_filter(tr("Tastaturbefehle"), &["json"])
                        .set_file_name("studiocraft-tasten.json")
                        .save_file()
                {
                    self.key_dialog.message = self
                        .keymap
                        .to_json()
                        .and_then(|t| {
                            std::fs::write(&p, t).map_err(|e| format!("{}: {e}", p.display()))
                        })
                        .err();
                }
                if ui.button(tr("Importieren …")).clicked()
                    && let Some(p) = rfd::FileDialog::new()
                        .add_filter(tr("Tastaturbefehle"), &["json", "txt"])
                        .pick_file()
                {
                    match read_capped(&p) {
                        Ok(t) => {
                            let (map, problems) = Keymap::from_preset(&t);
                            self.keymap = map;
                            self.key_dialog.message =
                                (!problems.is_empty()).then(|| problems.join("; "));
                            self.keymap_changed();
                        }
                        Err(e) => self.key_dialog.message = Some(format!("{}: {e}", p.display())),
                    }
                }
            }
        });
        if let Some(cmd) = self.key_dialog.capturing {
            let name = info(cmd).map_or("", CommandInfo::label);
            ui.colored_label(
                crate::theme::tokens(ui.ctx()).accent,
                crate::i18n::trf(
                    "Taste für „{name}“ drücken (Escape: abbrechen)",
                    &[("name", &name)],
                ),
            );
        }
        if let Some((cmd, k, owner)) = self.key_dialog.conflict {
            let owner_name = info(owner).map_or("", CommandInfo::label);
            ui.horizontal(|ui| {
                ui.colored_label(
                    ui.visuals().warn_fg_color,
                    crate::i18n::trf(
                        "{key} gehört schon zu „{name}“.",
                        &[("key", &display_key(k)), ("name", &owner_name)],
                    ),
                );
                if ui.button(tr("Ersetzen")).clicked() {
                    self.key_dialog.conflict = None;
                    match self.keymap.assign(cmd, k) {
                        Ok(()) => self.keymap_changed(),
                        Err(e) => self.key_dialog.message = Some(e),
                    }
                }
                if ui.button(tr("Abbrechen")).clicked() {
                    self.key_dialog.conflict = None;
                }
            });
        }
        if let Some(m) = &self.key_dialog.message {
            ui.colored_label(ui.visuals().warn_fg_color, m);
        }
        self.macros_ui(ui);
        ui.separator();
        let filter = self.key_dialog.filter.to_lowercase();
        let mut action: Option<(Cmd, u8)> = None;
        egui::ScrollArea::vertical().show(ui, |ui| {
            egui::Grid::new("tastaturbefehle")
                .num_columns(5)
                .min_col_width(60.0)
                .striped(true)
                .show(ui, |ui| {
                    let mut category = "";
                    for c in COMMANDS {
                        let keys = self.keymap.bindings(c.cmd);
                        let shown: Vec<String> = keys.iter().map(|k| display_key(*k)).collect();
                        let shown = shown.join(", ");
                        if !filter.is_empty()
                            && !c.label().to_lowercase().contains(&filter)
                            && !c.group().to_lowercase().contains(&filter)
                            && !shown.to_lowercase().contains(&filter)
                        {
                            continue;
                        }
                        if c.category != category {
                            category = c.category;
                            ui.strong(c.group());
                            ui.end_row();
                        }
                        match c.cmd {
                            Cmd::Macro(slot) => ui.label(self.keymap.macro_label(slot)),
                            _ => ui.label(c.label()),
                        };
                        let text = if shown.is_empty() {
                            "–".to_owned()
                        } else {
                            shown.clone()
                        };
                        ui.label(egui::RichText::new(text).monospace());
                        if ui
                            .small_button(tr("Setzen"))
                            .on_hover_text(crate::i18n::trf(
                                "Taste für {name} setzen",
                                &[("name", &c.label())],
                            ))
                            .clicked()
                        {
                            action = Some((c.cmd, 0));
                        }
                        if ui
                            .add_enabled(
                                !keys.is_empty(),
                                egui::Button::new(tr("Entfernen")).small(),
                            )
                            .clicked()
                        {
                            action = Some((c.cmd, 1));
                        }
                        if ui
                            .add_enabled(
                                keys.as_slice() != c.defaults,
                                egui::Button::new(tr("Standard")).small(),
                            )
                            .clicked()
                        {
                            action = Some((c.cmd, 2));
                        }
                        ui.end_row();
                    }
                });
        });
        match action {
            Some((cmd, 0)) => {
                self.key_dialog.capturing = Some(cmd);
                self.key_dialog.conflict = None;
            }
            Some((cmd, 1)) => {
                self.keymap.set(cmd, Vec::new());
                self.keymap_changed();
            }
            Some((cmd, _)) => {
                let d = info(cmd).map(|i| i.defaults.to_vec()).unwrap_or_default();
                self.keymap.set(cmd, d);
                self.keymap_changed();
            }
            None => {}
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ids_are_unique_and_defaults_do_not_clash() {
        let mut ids: Vec<&str> = COMMANDS.iter().map(|c| c.id).collect();
        ids.sort_unstable();
        let n = ids.len();
        ids.dedup();
        assert_eq!(ids.len(), n, "duplicate command id");
        let keys = Keymap::default().all();
        for (i, (a, ca)) in keys.iter().enumerate() {
            for (b, cb) in &keys[i + 1..] {
                assert!(a != b, "{} is bound to {ca:?} and {cb:?}", format_key(*a));
            }
        }
    }

    #[test]
    fn macros_round_trip_and_hostile_entries_are_reported() {
        let mut map = Keymap::default();
        map.assign(Cmd::Macro(2), k(CA, Key::M)).unwrap();
        let m = map.macro_slot_mut(2).unwrap();
        m.name = "Kopieren und schieben".into();
        m.steps = vec![Cmd::Edit(EditCommand::Duplicate), Cmd::ZoomIn];
        assert!(!map.is_default());
        let text = map.to_json().unwrap();
        let v: serde_json::Value = serde_json::from_str(&text).unwrap();
        assert_eq!(
            v["shortcuts"],
            serde_json::json!([{"command": "macro.2", "keys": ["Cmd+Alt+M"]}]),
            "{text}"
        );
        assert_eq!(v["macros"][0]["slot"], 2);
        assert_eq!(v["macros"][0]["name"], "Kopieren und schieben");
        assert_eq!(
            v["macros"][0]["steps"],
            serde_json::json!(["edit.duplicate", "nav.zoom_in"])
        );
        let (back, problems) = Keymap::from_json(&text).unwrap();
        assert!(problems.is_empty(), "{problems:?}");
        assert_eq!(back, map);
        assert_eq!(back.macro_label(2), "Makro 2: Kopieren und schieben");
        assert_eq!(back.macro_label(3), "Makro 3");

        let mut steps = vec!["macro.2", "nope"];
        steps.extend(vec!["nav.zoom_in"; MAX_STEPS + 5]);
        let hostile = serde_json::json!({
            "shortcuts": [
                {"command": "x.y", "keys": ["J"]},
                {"command": "edit.snap", "keys": ["Hyper+J"]},
            ],
            "macros": [
                {"slot": 0, "name": "x"},
                {"slot": 200, "steps": ["nav.zoom_in"]},
                {"slot": 1, "steps": steps},
                {"slot": 3, "name": "a".repeat(500)},
            ],
        })
        .to_string();
        let (map, problems) = Keymap::from_json(&hostile).unwrap();
        assert_eq!(map.macro_slot(1).unwrap().steps.len(), MAX_STEPS);
        assert_eq!(map.macro_slot(3).unwrap().name.chars().count(), MAX_NAME);
        assert!(map.macro_slot(0).is_none() && map.macro_slot(9).is_none());
        // Unknown command, bad key, slot 0, slot 200, nested macro, unknown step, too many.
        assert_eq!(problems.len(), 7, "{problems:?}");
        assert!(Keymap::from_json("{kaputt").is_err());
        assert!(Keymap::from_json(r#"{"macros": 5}"#).is_err());
    }

    #[test]
    fn old_text_files_still_load() {
        let (map, problems) = Keymap::from_preset(
            "# StudioCraft Tastaturbefehle\nedit.snap = \ntransport.metronome = C, J\n\
             macro.2 = Strg+Alt+M\nmacro.2.name = Aufräumen\nmacro.2.steps = edit.duplicate\n",
        );
        assert!(problems.is_empty(), "{problems:?}");
        assert_eq!(map.bindings(Cmd::Snap), []);
        assert_eq!(map.bindings(Cmd::Metronome), [k(N, Key::C), k(N, Key::J)]);
        assert_eq!(map.bindings(Cmd::Macro(2)), [k(CA, Key::M)]);
        assert_eq!(map.macro_label(2), "Makro 2: Aufräumen");
        let (_, problems) =
            Keymap::from_legacy_text("x.y = J\nedit.snap = Hyper+J\nkaputt\nmacro.1.color = rot\n");
        assert_eq!(problems.len(), 4, "{problems:?}");
    }

    #[test]
    fn keys_txt_moves_to_keys_json_once() {
        let dir = std::env::temp_dir().join(format!("sc-keys-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        assert_eq!(Keymap::load_from(&dir), (Keymap::default(), Vec::new()));
        std::fs::write(dir.join("keys.txt"), "transport.metronome = J\n").unwrap();
        let (map, problems) = Keymap::load_from(&dir);
        assert!(problems.is_empty(), "{problems:?}");
        assert_eq!(map.bindings(Cmd::Metronome), [k(N, Key::J)]);
        assert!(dir.join("keys.json").exists(), "migrated");
        assert!(dir.join("keys.txt").exists(), "the old file stays");
        // From now on keys.json wins.
        std::fs::write(dir.join("keys.txt"), "transport.metronome = K\n").unwrap();
        assert_eq!(Keymap::load_from(&dir).0, map);
        std::fs::write(dir.join("keys.json"), "nicht json").unwrap();
        let (map, problems) = Keymap::load_from(&dir);
        assert!(map.is_default() && problems.len() == 1);
        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn keys_print_and_parse_back() {
        let map = Keymap::default();
        for c in COMMANDS {
            assert_eq!(map.bindings(c.cmd), c.defaults, "{}", c.id);
            for k in c.defaults {
                assert_eq!(parse_key(&format_key(*k)), Some(*k), "{}", format_key(*k));
                assert_eq!(chord(*k).map(|c| shortcut(&c)), Some(*k), "{}", c.id);
            }
        }
        assert_eq!(format_key(k(CS, Key::S)), "Strg+Umschalt+S");
        assert!(parse_key("Hyper+Q").is_none());
        assert_eq!(display_key(k(N, Key::Delete)), "Entf");
        assert_eq!(display_key(k(C, Key::ArrowLeft)), "Strg+Pfeil links");
        // Mac Control counts as the command key, as in the old format.
        assert_eq!(chord(k(Modifiers::CTRL, Key::S)), chord(k(C, Key::S)));
    }

    #[test]
    fn assigning_moves_a_key_and_files_keep_only_changes() {
        let mut map = Keymap::default();
        assert_eq!(map.owner(k(N, Key::J), Cmd::Metronome), Some(Cmd::Snap));
        map.assign(Cmd::Metronome, k(N, Key::J)).unwrap();
        assert_eq!(map.bindings(Cmd::Snap), []);
        assert_eq!(map.bindings(Cmd::Metronome), [k(N, Key::C), k(N, Key::J)]);
        let text = map.to_json().unwrap();
        assert!(text.contains("\"transport.metronome\""), "{text}");
        assert!(text.contains("\"edit.snap\""), "{text}");
        assert!(
            !text.contains("file.save"),
            "unchanged commands are not written"
        );
        let (back, problems) = Keymap::from_json(&text).unwrap();
        assert!(problems.is_empty(), "{problems:?}");
        assert_eq!(back, map);
        // Setting the default again drops the override.
        map.set(Cmd::Snap, vec![k(N, Key::J)]);
        map.set(Cmd::Metronome, vec![k(N, Key::C)]);
        assert!(map.is_default());
        // set keeps other commands' keys.
        map.set(Cmd::Metronome, vec![k(N, Key::J)]);
        assert_eq!(map.bindings(Cmd::Snap), [k(N, Key::J)]);
    }
    #[test]
    fn every_command_name_and_group_has_an_english_entry() {
        let mut missing: Vec<&str> = Vec::new();
        for c in COMMANDS {
            for text in [c.name, c.category] {
                if !crate::i18n::has_english(text) && !missing.contains(&text) {
                    missing.push(text);
                }
            }
        }
        for key in [
            "Strg",
            "Umschalt",
            "Entf",
            "Rücktaste",
            "Leertaste",
            "Pos1",
            "Ende",
            "Eingabe",
            "Pfeil links",
            "Pfeil rechts",
            "Pfeil hoch",
            "Pfeil runter",
        ] {
            if !crate::i18n::has_english(key) {
                missing.push(key);
            }
        }
        assert!(
            missing.is_empty(),
            "fehlt in i18n/en.tsv:\n{}",
            missing.join("\n")
        );
    }
    #[test]
    fn keys_show_in_the_ui_language_but_files_stay_the_same() {
        let key = k(CS, Key::ArrowLeft);
        crate::i18n::set_language(crate::i18n::Language::English);
        assert_eq!(display_key(key), "Ctrl+Shift+Left");
        assert_eq!(display_key(k(N, Key::Delete)), "Del");
        assert_eq!(format_key(key), "Strg+Umschalt+Left", "Dateiformat bleibt");
        assert_eq!(parse_key(&format_key(key)), Some(key));
        let save = info(Cmd::File(Action::Save)).unwrap();
        assert_eq!((save.label(), save.name), ("Save", "Speichern"));
        crate::i18n::set_language(crate::i18n::Language::German);
        assert_eq!(display_key(key), "Strg+Umschalt+Pfeil links");
    }
}
