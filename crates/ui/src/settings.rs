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
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            theme: ThemePreference::Dark,
        }
    }
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
        let theme = match text
            .split("\"theme\"")
            .nth(1)
            .and_then(|r| r.split('"').nth(1))
        {
            Some("light") => ThemePreference::Light,
            Some("system") => ThemePreference::System,
            _ => ThemePreference::Dark,
        };
        Self { theme }
    }

    pub fn save(&self) -> Result<(), String> {
        let d = dir().ok_or("kein Ort für Einstellungen")?;
        std::fs::create_dir_all(&d).map_err(|e| format!("{}: {e}", d.display()))?;
        let text = format!("{{\n  \"theme\": \"{}\"\n}}\n", theme_name(self.theme));
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
        }
        .save()
        .unwrap();
        assert_eq!(Settings::load().theme, ThemePreference::Light);
        Settings {
            theme: ThemePreference::System,
        }
        .save()
        .unwrap();
        assert_eq!(Settings::load().theme, ThemePreference::System);
        unsafe { std::env::remove_var("STUDIOCRAFT_CONFIG_DIR") };
    }
}
