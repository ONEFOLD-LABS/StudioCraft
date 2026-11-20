//! The assistant's settings in the settings folder: `assistant.json` (endpoint, model, the
//! hosts the person agreed to send their project to) and, only when asked, the API key in
//! `assistant.key`, readable by the user alone. `STUDIOCRAFT_AI_KEY` gives a key without
//! storing it.

use std::path::{Path, PathBuf};

use serde_json::{Value, json};

/// Ollama's address: works without a key, and nothing leaves the computer.
pub const DEFAULT_URL: &str = "http://localhost:11434/v1";
/// Largest settings file read.
const MAX_FILE: u64 = 64 << 10;

#[derive(Clone, Debug, PartialEq)]
pub struct Settings {
    pub url: String,
    pub model: String,
    /// The key, from the key file or the environment; empty when none.
    pub key: String,
    /// Keep the key in `assistant.key`.
    pub remember_key: bool,
    /// Hosts the person agreed to send project data to.
    pub consented: Vec<String>,
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            url: DEFAULT_URL.into(),
            model: String::new(),
            key: String::new(),
            remember_key: false,
            consented: Vec::new(),
        }
    }
}

impl Settings {
    /// The settings from the settings folder, or the defaults.
    pub fn load() -> Self {
        let mut s = studiocraft_ui::settings::config_dir()
            .map(|d| Self::load_from(&d))
            .unwrap_or_default();
        if s.key.is_empty()
            && let Some(k) =
                std::env::var_os("STUDIOCRAFT_AI_KEY").and_then(|k| k.into_string().ok())
        {
            s.key = k.trim().to_owned();
        }
        s
    }

    fn load_from(dir: &Path) -> Self {
        let mut s = Self::default();
        if let Some(v) =
            read(&dir.join("assistant.json")).and_then(|t| serde_json::from_str::<Value>(&t).ok())
        {
            if let Some(u) = v["url"].as_str() {
                s.url = u.to_owned();
            }
            if let Some(m) = v["model"].as_str() {
                s.model = m.to_owned();
            }
            s.remember_key = v["remember_key"].as_bool().unwrap_or(false);
            s.consented = v["consented"]
                .as_array()
                .into_iter()
                .flatten()
                .filter_map(|h| h.as_str().map(str::to_owned))
                .take(100)
                .collect();
        }
        if s.remember_key {
            s.key = read(&key_path(dir)).unwrap_or_default().trim().to_owned();
        }
        s
    }

    /// Store the settings; the key only with `remember_key`, else the key file is removed.
    pub fn save(&self) -> Result<(), String> {
        let dir = studiocraft_ui::settings::config_dir().ok_or("settings folder unknown")?;
        self.save_to(&dir)
    }

    fn save_to(&self, dir: &Path) -> Result<(), String> {
        std::fs::create_dir_all(dir).map_err(|e| format!("{}: {e}", dir.display()))?;
        let v = json!({
            "url": self.url.trim(),
            "model": self.model.trim(),
            "remember_key": self.remember_key,
            "consented": self.consented,
        });
        let text = serde_json::to_string_pretty(&v).map_err(|e| e.to_string())?;
        let file = dir.join("assistant.json");
        std::fs::write(&file, text).map_err(|e| format!("{}: {e}", file.display()))?;
        let key_file = key_path(dir);
        if self.remember_key && !self.key.trim().is_empty() {
            write_private(&key_file, self.key.trim())
        } else {
            match std::fs::remove_file(&key_file) {
                Err(e) if e.kind() != std::io::ErrorKind::NotFound => {
                    Err(format!("{}: {e}", key_file.display()))
                }
                _ => Ok(()),
            }
        }
    }
}

fn key_path(dir: &Path) -> PathBuf {
    dir.join("assistant.key")
}

fn read(path: &Path) -> Option<String> {
    let meta = std::fs::metadata(path).ok()?;
    if meta.len() > MAX_FILE {
        return None;
    }
    std::fs::read_to_string(path).ok()
}

/// Write `text` to `path` so that only the user can read it (mode 0600 on Unix; on Windows the
/// settings folder in the user's profile is private already).
fn write_private(path: &Path, text: &str) -> Result<(), String> {
    use std::io::Write;
    let err = |e: std::io::Error| format!("{}: {e}", path.display());
    let mut opts = std::fs::OpenOptions::new();
    opts.write(true).create(true).truncate(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::{OpenOptionsExt, PermissionsExt};
        opts.mode(0o600);
        // An older file may have wider rights: narrow them before writing the key.
        if path.exists() {
            std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o600)).map_err(err)?;
        }
    }
    let mut f = opts.open(path).map_err(err)?;
    f.write_all(text.as_bytes()).map_err(err)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp(name: &str) -> PathBuf {
        let d = std::env::temp_dir().join(format!(
            "studiocraft-assistant-settings-{name}-{}",
            std::process::id()
        ));
        let _ = std::fs::remove_dir_all(&d);
        d
    }

    #[test]
    fn settings_round_trip_and_the_key_only_when_asked() {
        let dir = temp("round");
        assert_eq!(Settings::load_from(&dir), Settings::default());

        let mut s = Settings {
            url: "https://api.example.com/v1".into(),
            model: "m1".into(),
            key: "sk-1".into(),
            remember_key: false,
            consented: vec!["api.example.com".into()],
        };
        s.save_to(&dir).unwrap();
        assert!(!key_path(&dir).exists());
        let text = std::fs::read_to_string(dir.join("assistant.json")).unwrap();
        assert!(!text.contains("sk-1"));
        let back = Settings::load_from(&dir);
        assert_eq!(back.key, "");
        assert_eq!(back.model, "m1");
        assert_eq!(back.consented, ["api.example.com"]);

        s.remember_key = true;
        s.save_to(&dir).unwrap();
        assert_eq!(Settings::load_from(&dir).key, "sk-1");
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mode = std::fs::metadata(key_path(&dir))
                .unwrap()
                .permissions()
                .mode();
            assert_eq!(mode & 0o777, 0o600);
        }

        // Forgetting the key removes the file.
        s.remember_key = false;
        s.save_to(&dir).unwrap();
        assert!(!key_path(&dir).exists());
    }

    #[test]
    fn broken_settings_fall_back_to_defaults() {
        let dir = temp("broken");
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("assistant.json"), "{nope").unwrap();
        assert_eq!(Settings::load_from(&dir), Settings::default());
        std::fs::write(
            dir.join("assistant.json"),
            r#"{"url": 5, "consented": "x"}"#,
        )
        .unwrap();
        assert_eq!(Settings::load_from(&dir), Settings::default());
    }
}
