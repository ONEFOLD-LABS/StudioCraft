//! Automatische Sicherung und Wiederherstellung nach einem Absturz (Issue #52).
//!
//! In festem Abstand (Einstellungen, Standard 5 Minuten) schreibt StudioCraft das Projekt als
//! nummerierte Sicherung neben die Projektdatei: `Name.backup-07.studiocraft`. So bleiben die
//! relativen Pfade der Audiodateien gültig. Projekte ohne Datei landen im Einstellungsordner
//! unter `autosave/`. Geschrieben wird atomar (temporäre Datei, dann umbenennen) in einem
//! eigenen Thread; die ältesten Sicherungen über der eingestellten Anzahl werden gelöscht.
//! Eine Sicherung markiert das Projekt nie als gespeichert.
//!
//! Absturzerkennung: Beim Start legt die App `running.lock` im Einstellungsordner an und
//! entfernt sie beim Beenden. Liegt sie beim Start noch da, bietet ein Dialog die neueste
//! Sicherung an (ihr Pfad steht in `autosave/last.txt`).

use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use studiocraft_project::{Project, file};

use crate::ProjectWindow;

/// Teil im Dateinamen zwischen Projektname und Nummer.
const MARK: &str = ".backup-";

/// Zustand der automatischen Sicherung.
#[derive(Default)]
pub(crate) struct Autosave {
    /// Abstand in Minuten (0 = aus) und Anzahl der Sicherungen.
    pub minutes: u32,
    pub keep: u32,
    /// Nächster Zeitpunkt; `None` bis zum ersten Frame.
    next: Option<Instant>,
    /// Stand der letzten Sicherung (nur bei Änderungen neu sichern).
    last: Option<Project>,
    /// Laufender Schreib-Thread mit seinem Ergebnis.
    writer: Option<std::thread::JoinHandle<Result<PathBuf, String>>>,
    /// Ordner für Projekte ohne Datei und für `last.txt` (in Tests ein Temp-Ordner).
    pub dir: Option<PathBuf>,
    /// Nach einem Absturz: angebotene Sicherung und die ursprüngliche Projektdatei.
    pub recovery: Option<(PathBuf, Option<PathBuf>)>,
}

/// Nummer einer Sicherung `stem.backup-NN.studiocraft` von `stem`.
fn backup_number(name: &str, stem: &str) -> Option<u32> {
    name.strip_prefix(stem)?
        .strip_prefix(MARK)?
        .strip_suffix(&format!(".{}", file::EXTENSION))?
        .parse()
        .ok()
}

/// Vorhandene Sicherungen von `stem` in `dir`, nach Nummer sortiert.
pub(crate) fn backups(dir: &Path, stem: &str) -> Vec<(u32, PathBuf)> {
    let mut out: Vec<(u32, PathBuf)> = std::fs::read_dir(dir)
        .into_iter()
        .flatten()
        .flatten()
        .filter_map(|e| {
            let name = e.file_name().to_string_lossy().into_owned();
            backup_number(&name, stem).map(|n| (n, e.path()))
        })
        .collect();
    out.sort();
    out
}

/// Schreibt `json` als nächste Sicherung von `stem` in `dir` (atomar) und löscht die ältesten
/// über `keep`. Gibt den Pfad der neuen Sicherung zurück.
pub(crate) fn write_backup(
    dir: &Path,
    stem: &str,
    json: &str,
    keep: u32,
) -> Result<PathBuf, String> {
    std::fs::create_dir_all(dir).map_err(|e| format!("{}: {e}", dir.display()))?;
    let existing = backups(dir, stem);
    let n = existing.last().map_or(1, |(n, _)| n.saturating_add(1));
    let path = dir.join(format!("{stem}{MARK}{n:02}.{}", file::EXTENSION));
    let tmp = path.with_extension(format!("{}.tmp", file::EXTENSION));
    std::fs::write(&tmp, json).map_err(|e| format!("{}: {e}", tmp.display()))?;
    std::fs::rename(&tmp, &path).map_err(|e| format!("{}: {e}", path.display()))?;
    let all = backups(dir, stem);
    let surplus = all.len().saturating_sub(keep.max(1) as usize);
    for (_, old) in all.into_iter().take(surplus) {
        let _ = std::fs::remove_file(old);
    }
    Ok(path)
}

impl Autosave {
    pub(crate) fn new(minutes: u32, keep: u32) -> Self {
        Self {
            minutes,
            keep,
            ..Self::default()
        }
    }
}

impl ProjectWindow {
    /// Ordner und Name der Sicherungen: neben der Projektdatei, sonst im Autosave-Ordner.
    fn backup_target(&self) -> Option<(PathBuf, String)> {
        match &self.project_path {
            Some(p) => {
                let dir = p
                    .parent()
                    .filter(|d| !d.as_os_str().is_empty())
                    .unwrap_or(Path::new("."))
                    .to_path_buf();
                let stem = p.file_stem()?.to_string_lossy().into_owned();
                Some((dir, stem))
            }
            None => Some((self.autosave.dir.clone()?, "Unbenannt".into())),
        }
    }

    /// Projekt so, wie es in die Sicherung kommt: Audiopfade relativ zum Projektordner, wenn
    /// die Dateien darin liegen, sonst absolut (nichts wird kopiert).
    fn backup_json(&self, dir: &Path) -> String {
        let mut p = self.project.clone();
        for clip in &mut p.pool {
            if let Some(path) = &clip.path
                && let Ok(rel) = Path::new(path).strip_prefix(dir)
            {
                clip.path = Some(rel.to_string_lossy().replace('\\', "/"));
            }
        }
        file::to_json(&p)
    }

    /// Je Frame: zur Zeit sichern, wenn sich seit Speichern und letzter Sicherung etwas
    /// geändert hat. Das Schreiben läuft im Hintergrund.
    pub(crate) fn tick_autosave(&mut self, now: Instant) {
        if let Some(h) = &self.autosave.writer
            && h.is_finished()
            && let Some(h) = self.autosave.writer.take()
        {
            match h.join() {
                Ok(Ok(path)) => self.note_last_backup(&path),
                Ok(Err(e)) => self.message = Some(format!("Sicherung fehlgeschlagen: {e}")),
                Err(_) => self.message = Some("Sicherung fehlgeschlagen".into()),
            }
        }
        if self.autosave.minutes == 0 {
            return;
        }
        let interval = Duration::from_secs(u64::from(self.autosave.minutes) * 60);
        let next = *self.autosave.next.get_or_insert(now + interval);
        if now < next || self.autosave.writer.is_some() {
            return;
        }
        self.autosave.next = Some(now + interval);
        self.autosave_now();
    }

    /// Sichert sofort (wenn es seit Speichern und letzter Sicherung Änderungen gibt).
    pub(crate) fn autosave_now(&mut self) {
        if self.saved.as_ref() == Some(&self.project)
            || self.autosave.last.as_ref() == Some(&self.project)
        {
            return;
        }
        let Some((dir, stem)) = self.backup_target() else {
            return;
        };
        let json = self.backup_json(&dir);
        let keep = self.autosave.keep;
        self.autosave.last = Some(self.project.clone());
        self.autosave.writer = Some(std::thread::spawn(move || {
            write_backup(&dir, &stem, &json, keep)
        }));
    }

    /// Wartet auf eine laufende Sicherung (Tests, Beenden).
    pub(crate) fn finish_autosave(&mut self) {
        if let Some(h) = self.autosave.writer.take() {
            match h.join() {
                Ok(Ok(path)) => self.note_last_backup(&path),
                Ok(Err(e)) => self.message = Some(format!("Sicherung fehlgeschlagen: {e}")),
                Err(_) => {}
            }
        }
    }

    /// Merkt sich die neueste Sicherung und die Projektdatei für die Wiederherstellung.
    fn note_last_backup(&mut self, path: &Path) {
        let Some(dir) = &self.autosave.dir else {
            return;
        };
        let original = self
            .project_path
            .as_ref()
            .map(|p| p.to_string_lossy().into_owned())
            .unwrap_or_default();
        let text = format!("{}\n{original}\n", path.display());
        let _ = std::fs::create_dir_all(dir);
        let _ = std::fs::write(dir.join("last.txt"), text);
    }

    /// Beim Start der App: Sicherungsordner setzen, Absturz erkennen (Sperrdatei liegt noch
    /// da) und dann die Sperrdatei anlegen.
    pub fn begin_session(&mut self) {
        let Some(config) = crate::settings::config_dir() else {
            return;
        };
        self.begin_session_in(&config);
    }

    pub(crate) fn begin_session_in(&mut self, config: &Path) {
        let dir = config.join("autosave");
        self.autosave.dir = Some(dir.clone());
        let lock = config.join("running.lock");
        if lock.exists()
            && let Ok(text) = std::fs::read_to_string(dir.join("last.txt"))
        {
            let mut lines = text.lines();
            let backup = lines.next().map(PathBuf::from);
            let original = lines.next().filter(|l| !l.is_empty()).map(PathBuf::from);
            if let Some(b) = backup.filter(|b| b.exists()) {
                self.autosave.recovery = Some((b, original));
            }
        }
        let _ = std::fs::create_dir_all(config);
        let _ = std::fs::write(&lock, std::process::id().to_string());
    }

    /// Beim sauberen Beenden: laufende Sicherung abwarten, Sperrdatei entfernen.
    pub fn end_session(&mut self) {
        self.finish_autosave();
        if let Some(config) = self.autosave.dir.as_ref().and_then(|d| d.parent()) {
            let _ = std::fs::remove_file(config.join("running.lock"));
        }
    }

    /// Öffnet die Sicherung; das Projekt zeigt danach wieder auf seine Datei und bleibt
    /// ungespeichert.
    pub(crate) fn recover(
        &mut self,
        backup: &Path,
        original: Option<PathBuf>,
    ) -> Result<(), String> {
        self.open(backup)?;
        self.project_path = original;
        self.saved = None;
        self.message = Some(format!("Sicherung geöffnet: {}", backup.display()));
        Ok(())
    }

    /// Untermenü Datei → „Automatisch sichern“: Abstand und Anzahl.
    pub(crate) fn autosave_menu(&mut self, ui: &mut egui::Ui) {
        ui.menu_button("Automatisch sichern", |ui| {
            let mut changed = false;
            for (m, label) in [
                (0, "Aus"),
                (1, "Jede Minute"),
                (2, "Alle 2 Minuten"),
                (5, "Alle 5 Minuten"),
                (10, "Alle 10 Minuten"),
                (15, "Alle 15 Minuten"),
            ] {
                changed |= ui
                    .radio_value(&mut self.autosave.minutes, m, label)
                    .changed();
            }
            ui.separator();
            ui.menu_button("Anzahl Sicherungen", |ui| {
                for n in [3u32, 5, 10, 20, 50] {
                    changed |= ui
                        .radio_value(&mut self.autosave.keep, n, n.to_string())
                        .changed();
                }
            });
            if changed {
                self.autosave.next = None;
                self.save_settings();
            }
        });
    }

    /// Dialog nach einem Absturz.
    pub(crate) fn recovery_dialog(&mut self, ctx: &egui::Context) {
        let Some((backup, original)) = self.autosave.recovery.clone() else {
            return;
        };
        let mut choice = None;
        egui::Window::new("Wiederherstellen")
            .collapsible(false)
            .resizable(false)
            .anchor(egui::Align2::CENTER_CENTER, [0.0, 0.0])
            .show(ctx, |ui| {
                ui.label("StudioCraft wurde nicht sauber beendet.");
                ui.label(format!("Neueste Sicherung: {}", backup.display()));
                ui.horizontal(|ui| {
                    if ui.button("Sicherung öffnen").clicked() {
                        choice = Some(true);
                    }
                    if ui.button("Verwerfen").clicked() {
                        choice = Some(false);
                    }
                });
            });
        match choice {
            Some(true) => {
                self.autosave.recovery = None;
                if let Err(e) = self.recover(&backup, original) {
                    self.message = Some(e);
                }
            }
            Some(false) => self.autosave.recovery = None,
            None => {}
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp(name: &str) -> PathBuf {
        let d = std::env::temp_dir().join(format!("sc-autosave-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(&d).unwrap();
        d
    }

    fn window_with_clip() -> ProjectWindow {
        let mut w = ProjectWindow::new(Project::default(), None, String::new());
        let samples: Vec<f32> = (0..9600).map(|i| (i as f32 * 0.05).sin() * 0.3).collect();
        let clip = studiocraft_engine::AudioClip {
            sample_rate: 48_000,
            samples: samples.into(),
        };
        w.import_clip("Take", None, clip, None).unwrap();
        w
    }

    #[test]
    fn changes_are_backed_up_next_to_the_project_without_marking_it_saved() {
        let dir = temp("project");
        let mut w = window_with_clip();
        w.save_as(&dir.join("Song.studiocraft")).unwrap();
        w.autosave = Autosave::new(5, 3);
        w.begin_session_in(&dir.join("config"));
        // Unchanged: nothing to back up.
        w.autosave_now();
        w.finish_autosave();
        assert!(backups(&dir, "Song").is_empty());
        // A change: one backup, the project stays unsaved and the file untouched.
        let before = w.project.clone();
        w.project.tracks[0].name = "Gesang".into();
        w.checkpoint(before);
        w.autosave_now();
        w.finish_autosave();
        let list = backups(&dir, "Song");
        assert_eq!(list.len(), 1);
        assert!(w.saved.as_ref() != Some(&w.project), "still unsaved");
        let text = std::fs::read_to_string(&list[0].1).unwrap();
        let backup = studiocraft_project::file::from_json(&text).unwrap();
        assert_eq!(backup.tracks[0].name, "Gesang");
        assert_eq!(
            backup.pool[0]
                .path
                .as_deref()
                .map(|p| p.starts_with("Audio/")),
            Some(true),
            "audio paths stay relative to the project folder"
        );
        let saved = std::fs::read_to_string(dir.join("Song.studiocraft")).unwrap();
        assert!(!saved.contains("Gesang"), "the project file is not touched");
        // No new change: no second backup.
        w.autosave_now();
        w.finish_autosave();
        assert_eq!(backups(&dir, "Song").len(), 1);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn the_timer_backs_up_once_per_interval() {
        let dir = temp("timer");
        let mut w = window_with_clip();
        w.autosave = Autosave::new(1, 5);
        w.begin_session_in(&dir);
        let t0 = Instant::now();
        w.tick_autosave(t0);
        w.finish_autosave();
        let auto = dir.join("autosave");
        assert!(
            backups(&auto, "Unbenannt").is_empty(),
            "not before the interval"
        );
        w.tick_autosave(t0 + Duration::from_secs(61));
        w.finish_autosave();
        assert_eq!(
            backups(&auto, "Unbenannt").len(),
            1,
            "unsaved projects go to autosave/"
        );
        // Off: nothing more, even with changes.
        w.autosave.minutes = 0;
        w.project.tracks[0].name = "x".into();
        w.tick_autosave(t0 + Duration::from_secs(600));
        w.finish_autosave();
        assert_eq!(backups(&auto, "Unbenannt").len(), 1);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn after_a_crash_the_newest_backup_is_offered_and_opens_unsaved() {
        let dir = temp("crash");
        let config = dir.join("config");
        let mut w = window_with_clip();
        w.save_as(&dir.join("Song.studiocraft")).unwrap();
        w.autosave = Autosave::new(5, 3);
        w.begin_session_in(&config);
        let before = w.project.clone();
        w.project.tracks[0].name = "Nach dem Speichern".into();
        w.checkpoint(before);
        w.autosave_now();
        w.finish_autosave();
        // "Crash": no end_session. The next start finds the lock and offers the backup.
        let mut next = ProjectWindow::new(Project::default(), None, String::new());
        next.begin_session_in(&config);
        let (backup, original) = next.autosave.recovery.clone().expect("recovery offered");
        assert_eq!(
            original.as_deref(),
            Some(dir.join("Song.studiocraft").as_path())
        );
        next.recover(&backup, original).unwrap();
        assert_eq!(next.project.tracks[0].name, "Nach dem Speichern");
        assert_eq!(
            next.project_path.as_deref(),
            Some(dir.join("Song.studiocraft").as_path())
        );
        assert!(next.saved.is_none(), "a recovered backup is not saved");
        // A clean exit removes the lock: no offer on the following start.
        next.end_session();
        let mut third = ProjectWindow::new(Project::default(), None, String::new());
        third.begin_session_in(&config);
        assert!(third.autosave.recovery.is_none());
        third.end_session();
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn backups_are_numbered_and_pruned() {
        let dir = std::env::temp_dir().join(format!("sc-backups-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        for i in 0..5 {
            write_backup(&dir, "Song", &format!("{{\"n\":{i}}}"), 3).unwrap();
        }
        let left: Vec<u32> = backups(&dir, "Song").iter().map(|(n, _)| *n).collect();
        assert_eq!(left, [3, 4, 5]);
        // Other files and other projects stay untouched.
        std::fs::write(dir.join("Song.studiocraft"), "x").unwrap();
        std::fs::write(dir.join("Song 2.backup-01.studiocraft"), "x").unwrap();
        write_backup(&dir, "Song", "{}", 3).unwrap();
        assert!(dir.join("Song.studiocraft").exists());
        assert!(dir.join("Song 2.backup-01.studiocraft").exists());
        assert_eq!(backups(&dir, "Song").len(), 3);
        assert!(!dir.join("Song.backup-06.studiocraft.tmp").exists());
        let _ = std::fs::remove_dir_all(&dir);
    }
}
