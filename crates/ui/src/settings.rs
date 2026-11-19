//! Programmeinstellungen (nicht projektbezogen), z. B. das Erscheinungsbild.
//! Liegen als kleine JSON-Datei am üblichen Ort des Systems:
//! Linux/BSD `$XDG_CONFIG_HOME/studiocraft` bzw. `~/.config/studiocraft`,
//! macOS `~/Library/Application Support/StudioCraft`, Windows `%APPDATA%\StudioCraft`.
//! `STUDIOCRAFT_CONFIG_DIR` überschreibt den Ort (Tests).

use std::path::PathBuf;

use egui::ThemePreference;

#[derive(Debug, Clone, PartialEq)]
pub struct Settings {
    pub theme: ThemePreference,
    /// Beim Stoppen zur Position springen, an der die Wiedergabe begann.
    pub return_on_stop: bool,
    /// Wohin zweimal Stopp springt.
    pub stop_twice: StopTarget,
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
        }
    }
}

/// Rohwert eines Schlüssels aus der kleinen Einstellungsdatei (`"key": wert`), ohne Anführungszeichen.
fn field<'a>(text: &'a str, key: &str) -> Option<&'a str> {
    let rest = text.split(&format!("\"{key}\"")).nth(1)?;
    let value = rest.trim_start().strip_prefix(':')?.trim_start();
    let end = value.find([',', '\n', '}']).unwrap_or(value.len());
    Some(value[..end].trim().trim_matches('"'))
}

/// Ordner der Einstellungen (auch für die Tastaturbelegung).
pub(crate) fn config_dir() -> Option<PathBuf> {
    dir()
}

fn dir() -> Option<PathBuf> {
    if let Some(d) = std::env::var_os("STUDIOCRAFT_CONFIG_DIR") {
        return Some(PathBuf::from(d));
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
    /// Liest die Einstellungen; fehlt die Datei oder ist sie unlesbar, gelten die Vorgaben.
    pub fn load() -> Self {
        let Some(text) = dir().and_then(|d| std::fs::read_to_string(d.join("settings.json")).ok())
        else {
            return Self::default();
        };
        let theme = match field(&text, "theme") {
            Some("light") => ThemePreference::Light,
            Some("system") => ThemePreference::System,
            _ => ThemePreference::Dark,
        };
        Self {
            theme,
            return_on_stop: field(&text, "return_on_stop") == Some("true"),
            stop_twice: match field(&text, "stop_twice") {
                Some("left_locator") => StopTarget::LeftLocator,
                _ => StopTarget::Start,
            },
        }
    }

    pub fn save(&self) -> Result<(), String> {
        let d = dir().ok_or("kein Ort für Einstellungen")?;
        std::fs::create_dir_all(&d).map_err(|e| format!("{}: {e}", d.display()))?;
        let stop_twice = match self.stop_twice {
            StopTarget::Start => "start",
            StopTarget::LeftLocator => "left_locator",
        };
        let text = format!(
            "{{\n  \"theme\": \"{}\",\n  \"return_on_stop\": {},\n  \"stop_twice\": \"{stop_twice}\"\n}}\n",
            theme_name(self.theme),
            self.return_on_stop
        );
        std::fs::write(d.join("settings.json"), text).map_err(|e| format!("{}: {e}", d.display()))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn roundtrip_through_file() {
        let d = std::env::temp_dir().join(format!("sc-settings-{}", std::process::id()));
        // SAFETY: nur dieser Test setzt die Variable.
        unsafe { std::env::set_var("STUDIOCRAFT_CONFIG_DIR", &d) };
        Settings {
            theme: ThemePreference::Light,
            ..Settings::default()
        }
        .save()
        .unwrap();
        assert_eq!(Settings::load().theme, ThemePreference::Light);
        let s = Settings {
            theme: ThemePreference::System,
            return_on_stop: true,
            stop_twice: StopTarget::LeftLocator,
        };
        s.save().unwrap();
        assert_eq!(Settings::load(), s);
        // Older files with only the theme keep the defaults for the rest.
        std::fs::write(d.join("settings.json"), "{\n  \"theme\": \"light\"\n}\n").unwrap();
        let old = Settings::load();
        assert_eq!(old.theme, ThemePreference::Light);
        assert!(!old.return_on_stop);
        assert_eq!(old.stop_twice, StopTarget::Start);
        unsafe { std::env::remove_var("STUDIOCRAFT_CONFIG_DIR") };
    }
}
