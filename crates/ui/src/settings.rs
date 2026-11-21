//! Programmeinstellungen (nicht projektbezogen), z. B. das Erscheinungsbild.
//! Liegen als kleine JSON-Datei am üblichen Ort des Systems:
//! Linux/BSD `$XDG_CONFIG_HOME/studiocraft` bzw. `~/.config/studiocraft`,
//! macOS `~/Library/Application Support/StudioCraft`, Windows `%APPDATA%\StudioCraft`.
//! `STUDIOCRAFT_CONFIG_DIR` überschreibt den Ort (Tests).

use crate::i18n::tr;
use std::path::PathBuf;

use egui::ThemePreference;
use studiocraft_engine::DeviceChoice;

#[derive(Debug, Clone, PartialEq)]
pub struct Settings {
    pub theme: ThemePreference,
    /// Beim Stoppen zur Position springen, an der die Wiedergabe begann.
    pub return_on_stop: bool,
    /// Wohin zweimal Stopp springt.
    pub stop_twice: StopTarget,
    /// Audio-Host und -Geräte (Dialog „Audio- und MIDI-Geräte“).
    pub audio: DeviceChoice,
    /// MIDI-Eingänge öffnen.
    pub midi_inputs: bool,
    /// Lage der eigenen Fenster (Mixer, Editor).
    pub windows: WindowRects,
    /// Automatische Sicherung: Abstand in Minuten (0 = aus) und Anzahl der Sicherungen.
    pub autosave_minutes: u32,
    pub autosave_keep: u32,
    /// Sprache der Oberfläche.
    pub language: crate::i18n::Language,
    /// Plugin-Ids, deren neue Inserts im eigenen Prozess laufen (Plugin-Liste).
    pub own_process: Vec<String>,
    /// Einmal am Tag nach einer neuen Version suchen (Hilfe → „Nach Updates suchen“; aus,
    /// bis man es einschaltet).
    pub update_check: bool,
    /// Zeitpunkt der letzten Suche (Sekunden seit 1970).
    pub last_update_check: u64,
}

/// Vorgaben der automatischen Sicherung.
pub const AUTOSAVE_MINUTES: u32 = 5;
pub const AUTOSAVE_KEEP: u32 = 10;

/// Lage eines eigenen Fensters: Position (fehlt, wo das System sie nicht verrät, etwa unter
/// Wayland) und Innengröße in Punkten.
#[derive(Debug, Clone, Copy, PartialEq, Default, serde::Serialize, serde::Deserialize)]
pub struct WindowRect {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub pos: Option<[f32; 2]>,
    pub size: [f32; 2],
}

impl WindowRect {
    /// Fremde Dateiwerte in vernünftige Grenzen bringen; unbrauchbare Größen verwerfen.
    fn sanitized(self) -> Option<Self> {
        let ok = |v: f32, lo: f32, hi: f32| v.is_finite() && (lo..=hi).contains(&v);
        let size = self.size;
        if !(ok(size[0], 100.0, 10_000.0) && ok(size[1], 100.0, 10_000.0)) {
            return None;
        }
        let pos = self
            .pos
            .filter(|p| ok(p[0], -20_000.0, 20_000.0) && ok(p[1], -20_000.0, 20_000.0));
        Some(Self { pos, size })
    }
}

/// Gemerkte Lage der eigenen Fenster.
#[derive(Debug, Clone, Copy, PartialEq, Default, serde::Serialize, serde::Deserialize)]
#[serde(default)]
pub struct WindowRects {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub mixer: Option<WindowRect>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub editor: Option<WindowRect>,
}

/// Ziel von zweimal Stopp (Spec „Stopp-Verhalten“).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum StopTarget {
    /// Position 1.1.1.0.
    #[default]
    Start,
    LeftLocator,
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            theme: ThemePreference::Dark,
            return_on_stop: false,
            stop_twice: StopTarget::Start,
            audio: DeviceChoice::default(),
            midi_inputs: true,
            windows: WindowRects::default(),
            autosave_minutes: AUTOSAVE_MINUTES,
            autosave_keep: AUTOSAVE_KEEP,
            language: crate::i18n::Language::System,
            own_process: Vec::new(),
            update_check: false,
            last_update_check: 0,
        }
    }
}

/// So liegt es in der Datei; fehlende Felder bekommen ihren Standard (ältere Dateien).
#[derive(serde::Serialize, serde::Deserialize)]
#[serde(default)]
struct File {
    theme: String,
    return_on_stop: bool,
    stop_twice: String,
    audio: AudioFile,
    midi_inputs: bool,
    windows: WindowRects,
    autosave_minutes: u32,
    autosave_keep: u32,
    language: String,
    #[serde(skip_serializing_if = "Vec::is_empty")]
    own_process: Vec<String>,
    update_check: bool,
    last_update_check: u64,
}

#[derive(Default, serde::Serialize, serde::Deserialize)]
#[serde(default)]
struct AudioFile {
    #[serde(skip_serializing_if = "Option::is_none")]
    host: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    output: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    input: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    sample_rate: Option<u32>,
    #[serde(skip_serializing_if = "Option::is_none")]
    block_frames: Option<u32>,
}

impl Default for File {
    fn default() -> Self {
        Settings::default().to_file()
    }
}

/// Längste Zeichenkette aus der Datei (Gerätenamen); fremde Eingabe.
const MAX_TEXT: usize = 256;
/// Höchstzahl gemerkter Plugin-Ids und deren Länge.
const MAX_PLUGINS: usize = 10_000;
const MAX_ID: usize = 1024;

fn text(v: Option<String>) -> Option<String> {
    v.map(|s| s.chars().take(MAX_TEXT).collect::<String>())
        .filter(|s| !s.is_empty())
}

/// Ordner der Einstellungen (auch für die Tastaturbelegung).
pub fn config_dir() -> Option<PathBuf> {
    dir()
}

fn dir() -> Option<PathBuf> {
    if let Some(d) = std::env::var_os("STUDIOCRAFT_CONFIG_DIR") {
        return Some(PathBuf::from(d));
    }
    // Tests lesen und schreiben nie die echten Einstellungen: ein Test, der die Sprache im
    // Menü umschaltet, speichert sonst Englisch für alle späteren Fenster.
    if cfg!(test) {
        return None;
    }
    let home = std::env::var_os("HOME").map(PathBuf::from);
    if cfg!(windows) {
        std::env::var_os("APPDATA").map(|a| PathBuf::from(a).join("StudioCraft"))
    } else if cfg!(target_os = "macos") {
        home.map(|h| h.join("Library/Application Support/StudioCraft"))
    } else {
        std::env::var_os("XDG_CONFIG_HOME")
            .map(PathBuf::from)
            .or_else(|| home.map(|h| h.join(".config")))
            .map(|c| c.join("studiocraft"))
    }
}

fn theme_name(t: ThemePreference) -> &'static str {
    match t {
        ThemePreference::Dark => "dark",
        ThemePreference::Light => "light",
        ThemePreference::System => "system",
    }
}

impl Settings {
    fn to_file(&self) -> File {
        File {
            theme: theme_name(self.theme).into(),
            return_on_stop: self.return_on_stop,
            stop_twice: match self.stop_twice {
                StopTarget::Start => "start",
                StopTarget::LeftLocator => "left_locator",
            }
            .into(),
            audio: AudioFile {
                host: self.audio.host.clone(),
                output: self.audio.output.clone(),
                input: self.audio.input.clone(),
                sample_rate: self.audio.sample_rate,
                block_frames: self.audio.block_frames,
            },
            midi_inputs: self.midi_inputs,
            windows: self.windows,
            autosave_minutes: self.autosave_minutes,
            autosave_keep: self.autosave_keep,
            language: self.language.code().into(),
            own_process: self.own_process.clone(),
            update_check: self.update_check,
            last_update_check: self.last_update_check,
        }
    }

    fn from_file(f: File) -> Self {
        let rate = |v: Option<u32>| v.filter(|r| (8_000..=768_000).contains(r));
        Self {
            theme: match f.theme.as_str() {
                "light" => ThemePreference::Light,
                "system" => ThemePreference::System,
                _ => ThemePreference::Dark,
            },
            return_on_stop: f.return_on_stop,
            stop_twice: match f.stop_twice.as_str() {
                "left_locator" => StopTarget::LeftLocator,
                _ => StopTarget::Start,
            },
            audio: DeviceChoice {
                host: text(f.audio.host),
                output: text(f.audio.output),
                input: text(f.audio.input),
                sample_rate: rate(f.audio.sample_rate),
                block_frames: f.audio.block_frames.filter(|b| (16..=8192).contains(b)),
            },
            midi_inputs: f.midi_inputs,
            windows: WindowRects {
                mixer: f.windows.mixer.and_then(WindowRect::sanitized),
                editor: f.windows.editor.and_then(WindowRect::sanitized),
            },
            autosave_minutes: f.autosave_minutes.min(120),
            autosave_keep: f.autosave_keep.clamp(1, 100),
            language: crate::i18n::Language::from_code(&f.language),
            own_process: {
                let mut ids: Vec<String> = f
                    .own_process
                    .into_iter()
                    .filter(|id| !id.is_empty() && id.len() <= MAX_ID)
                    .take(MAX_PLUGINS)
                    .collect();
                ids.sort();
                ids.dedup();
                ids
            },
            update_check: f.update_check,
            last_update_check: f.last_update_check,
        }
    }

    /// Liest die Einstellungen; fehlt die Datei oder ist sie unlesbar, gelten die Vorgaben.
    pub fn load() -> Self {
        dir()
            .and_then(|d| std::fs::read_to_string(d.join("settings.json")).ok())
            .and_then(|text| Self::from_text(&text))
            .unwrap_or_default()
    }

    /// Liest den Inhalt einer Einstellungsdatei (fremde Eingabe: Werte werden begrenzt);
    /// `None`, wenn sie unlesbar ist.
    fn from_text(text: &str) -> Option<Self> {
        serde_json::from_str::<File>(text).ok().map(Self::from_file)
    }

    pub fn save(&self) -> Result<(), String> {
        let d = dir().ok_or(tr("kein Ort für Einstellungen"))?;
        std::fs::create_dir_all(&d).map_err(|e| format!("{}: {e}", d.display()))?;
        let text = serde_json::to_string_pretty(&self.to_file()).map_err(|e| e.to_string())?;
        std::fs::write(d.join("settings.json"), text + "\n")
            .map_err(|e| format!("{}: {e}", d.display()))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A settings file changed at random loads with every value in its limits, or gives the
    /// defaults; it never panics, and the daily update check is never stopped for good.
    #[test]
    fn fuzzed_settings_files() {
        use crate::fuzzkit::{Case, Rng, cases, mutate_bytes};
        let rect = WindowRect {
            pos: Some([10.0, 20.0]),
            size: [800.0, 600.0],
        };
        let full = Settings {
            audio: DeviceChoice {
                host: Some("ALSA".into()),
                output: Some("Karte".into()),
                input: Some("Karte".into()),
                sample_rate: Some(48_000),
                block_frames: Some(256),
            },
            windows: WindowRects {
                mixer: Some(rect),
                editor: Some(rect),
            },
            own_process: vec!["a.clap".into(), "b.vst3".into()],
            update_check: true,
            last_update_check: 1_700_000_000,
            ..Settings::default()
        };
        let seed_doc = serde_json::to_value(full.to_file()).unwrap_or_default();
        let now = crate::update::now_secs();
        let mut loaded = 0;
        let all = cases(0x5e7, 1500);
        let n = all.len();
        for seed in all {
            let _case = Case("settings", seed);
            let mut rng = Rng(seed | 1);
            let mut doc = seed_doc.clone();
            for _ in 0..1 + rng.below(3) {
                crate::fuzzjson::mutate_json(&mut rng, &mut doc);
            }
            let mut bytes = serde_json::to_vec(&doc).unwrap_or_default();
            if rng.below(4) == 0 {
                mutate_bytes(&mut rng, &mut bytes);
            }
            let Some(s) = Settings::from_text(&String::from_utf8_lossy(&bytes)) else {
                continue;
            };
            loaded += 1;
            let a = &s.audio;
            for t in [&a.host, &a.output, &a.input].into_iter().flatten() {
                assert!(!t.is_empty() && t.chars().count() <= MAX_TEXT);
            }
            assert!(a.sample_rate.is_none_or(|r| (8_000..=768_000).contains(&r)));
            assert!(a.block_frames.is_none_or(|b| (16..=8192).contains(&b)));
            for r in [s.windows.mixer, s.windows.editor].into_iter().flatten() {
                assert!(r.size.iter().all(|v| (100.0..=10_000.0).contains(v)));
            }
            assert!(s.autosave_minutes <= 120 && (1..=100).contains(&s.autosave_keep));
            assert!(s.own_process.len() <= MAX_PLUGINS);
            assert!(
                s.own_process.windows(2).all(|w| w[0] < w[1]),
                "sorted, unique"
            );
            assert!(
                crate::update::due(s.last_update_check, now.saturating_add(2 * 24 * 3600)),
                "the update check comes back within two days: {}",
                s.last_update_check
            );
            // Saving and loading again gives the same settings.
            let again = serde_json::to_string(&s.to_file())
                .ok()
                .and_then(|t| Settings::from_text(&t));
            assert_eq!(again.as_ref(), Some(&s));
        }
        assert!(loaded * 10 > n, "only {loaded} of {n} variants loaded");
    }

    #[test]
    fn roundtrip_through_file() {
        let d = std::env::temp_dir().join(format!("sc-settings-{}", std::process::id()));
        // SAFETY: nur dieser Test setzt die Variable.
        unsafe { std::env::set_var("STUDIOCRAFT_CONFIG_DIR", &d) };
        let s = Settings {
            theme: ThemePreference::System,
            return_on_stop: true,
            stop_twice: StopTarget::LeftLocator,
            audio: DeviceChoice {
                host: Some("ALSA".into()),
                output: Some("HDA Intel PCH, ALC892 \"Analog\"".into()),
                input: None,
                sample_rate: Some(96_000),
                block_frames: Some(128),
            },
            midi_inputs: false,
            windows: WindowRects {
                mixer: Some(WindowRect {
                    pos: Some([120.0, 80.0]),
                    size: [900.0, 500.0],
                }),
                editor: None,
            },
            autosave_minutes: 2,
            autosave_keep: 20,
            language: crate::i18n::Language::English,
            own_process: vec!["com.example.a".into(), "com.example.b".into()],
            update_check: true,
            last_update_check: 1_700_000_000,
        };
        s.save().unwrap();
        assert_eq!(Settings::load(), s);
        // Older files with only the theme keep the defaults for the rest.
        std::fs::write(d.join("settings.json"), "{\n  \"theme\": \"light\"\n}\n").unwrap();
        let old = Settings::load();
        assert_eq!(old.theme, ThemePreference::Light);
        assert_eq!(
            (old.autosave_minutes, old.autosave_keep),
            (AUTOSAVE_MINUTES, AUTOSAVE_KEEP)
        );
        assert!(!old.return_on_stop && old.midi_inputs);
        assert!(
            !old.update_check,
            "the update check stays off until switched on"
        );
        assert_eq!(old.audio, DeviceChoice::default());
        // Hostile values fall back.
        std::fs::write(
            d.join("settings.json"),
            format!(
                "{{\"audio\": {{\"sample_rate\": 0, \"block_frames\": 99999999, \"output\": \"{}\"}}}}",
                "x".repeat(5000)
            ),
        )
        .unwrap();
        let bad = Settings::load();
        assert_eq!(bad.audio.sample_rate, None);
        assert_eq!(bad.audio.block_frames, None);
        assert_eq!(bad.audio.output.map(|o| o.len()), Some(MAX_TEXT));
        std::fs::write(d.join("settings.json"), "kaputt").unwrap();
        assert_eq!(Settings::load(), Settings::default());
        unsafe { std::env::remove_var("STUDIOCRAFT_CONFIG_DIR") };
    }

    #[test]
    fn hostile_window_rects_are_dropped_or_fixed() {
        let r = WindowRect {
            pos: Some([f32::NAN, 0.0]),
            size: [800.0, 600.0],
        };
        assert_eq!(r.sanitized().map(|r| r.pos), Some(None));
        let tiny = WindowRect {
            pos: None,
            size: [5.0, 1e9],
        };
        assert_eq!(tiny.sanitized(), None);
    }
}
