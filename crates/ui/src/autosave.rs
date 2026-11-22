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
//!
//! Aufnahmen nach der letzten Sicherung: Nach dem Öffnen der Sicherung bietet ein Dialog die
//! WAV-Dateien im Aufnahmeordner des Projekts an, die jünger als die Sicherung sind und in
//! keinem Clip vorkommen (Takes, die beim Absturz noch nicht gesichert waren). „Als Takes
//! einfügen“ legt sie auf neue Spuren; gelöscht wird nichts.

use crate::engine_errors::TrErr;
use crate::i18n::{tr, trf};
use std::path::{Path, PathBuf};
use std::time::Duration;

use web_time::Instant;

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
    /// Nach dem Wiederherstellen: Aufnahmen, die nach der Sicherung entstanden und in keinem
    /// Clip vorkommen; je Aufnahme die erste Datei und ihre Folgedateien in Reihenfolge.
    pub orphans: Vec<Vec<PathBuf>>,
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
            None => Some((
                self.autosave.dir.clone()?,
                crate::i18n::tr("Unbenannt").into(),
            )),
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
                Ok(Err(e)) => {
                    self.message = Some(crate::i18n::trf(
                        "Sicherung fehlgeschlagen: {e}",
                        &[("e", &e)],
                    ))
                }
                Err(_) => self.message = Some(tr("Sicherung fehlgeschlagen").into()),
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
        // Im Browser gibt es weder Dateisystem noch Threads: keine Sicherung.
        if cfg!(target_arch = "wasm32") {
            return;
        }
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
                Ok(Err(e)) => {
                    self.message = Some(crate::i18n::trf(
                        "Sicherung fehlgeschlagen: {e}",
                        &[("e", &e)],
                    ))
                }
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
        let since = std::fs::metadata(backup)
            .and_then(|m| m.modified())
            .unwrap_or(std::time::UNIX_EPOCH);
        self.autosave.orphans = self.orphaned_takes(since);
        self.message = Some(crate::i18n::trf(
            "Sicherung geöffnet: {path}",
            &[("path", &backup.display())],
        ));
        Ok(())
    }

    /// WAV-Dateien im Aufnahmeordner, jünger als `since` und in keinem Clip des Projekts, je
    /// Aufnahme zusammengefasst: „Stamm.wav“ mit „Stamm-002.wav“, „Stamm-003.wav“ … (lange
    /// Aufnahmen gehen in Folgedateien weiter). Eine Folgedatei ohne ihre erste steht allein.
    pub(crate) fn orphaned_takes(&self, since: std::time::SystemTime) -> Vec<Vec<PathBuf>> {
        let dir = self.recording_dir();
        let used: std::collections::HashSet<PathBuf> = self
            .project
            .pool
            .iter()
            .filter_map(|c| c.path.as_deref())
            .map(|p| {
                let p = Path::new(p);
                if p.is_absolute() {
                    p.to_path_buf()
                } else {
                    dir.parent().unwrap_or(&dir).join(p)
                }
            })
            .collect();
        let mut out: Vec<PathBuf> = std::fs::read_dir(&dir)
            .into_iter()
            .flatten()
            .flatten()
            .map(|e| e.path())
            .filter(|p| p.extension().is_some_and(|e| e.eq_ignore_ascii_case("wav")))
            .filter(|p| !used.contains(p))
            .filter(|p| {
                std::fs::metadata(p)
                    .and_then(|m| m.modified())
                    .is_ok_and(|t| t > since)
            })
            .collect();
        out.sort();
        group_continuations(out)
    }

    /// Fügt die angebotenen Aufnahmen als Takes auf neuen Spuren ein (ein Undo-Schritt).
    /// Mehrkanal-Rohdateien werden je Kanal in eine eigene Datei daneben aufgeteilt; die
    /// Originale bleiben liegen.
    pub(crate) fn insert_orphans(&mut self) {
        let groups = std::mem::take(&mut self.autosave.orphans);
        let before = self.project.clone();
        let mut errors = Vec::new();
        for files in groups {
            if let Err(e) = self.insert_orphan(&files) {
                errors.push(e);
            }
        }
        self.checkpoint(before);
        if !errors.is_empty() {
            self.message = Some(errors.join("; "));
        }
    }

    /// Eine Aufnahme (erste Datei und Folgedateien) als Take: je Datei ein Event, die Events
    /// hintereinander auf einer neuen Spur. Mehrkanal-Rohdateien: eine Spur je Kanal, die
    /// Kanäle in eigene Dateien daneben geschrieben.
    fn insert_orphan(&mut self, files: &[PathBuf]) -> Result<(), String> {
        let rec = studiocraft_engine::input::recording_from_files(files).tr_err()?;
        // Je Spur („Bahn“): die Dateien, die sie nacheinander spielt.
        let lanes: Vec<Vec<PathBuf>> = if rec.channels <= 2 {
            vec![files.to_vec()]
        } else {
            let mut lanes = vec![Vec::new(); usize::from(rec.channels)];
            for path in files {
                let stem = path
                    .file_stem()
                    .unwrap_or_default()
                    .to_string_lossy()
                    .into_owned();
                let dir = path.parent().unwrap_or(Path::new("."));
                for (c, lane) in lanes.iter_mut().enumerate() {
                    let samples = studiocraft_engine::input::extract_file_channels(
                        path,
                        rec.channels,
                        c as u16,
                        false,
                    )
                    .tr_err()?;
                    let file = crate::session::unique_name(
                        dir,
                        // Dateiname: bleibt unübersetzt (steht im Projekt).
                        &format!("{stem} - Kanal {}.wav", c + 1),
                    );
                    studiocraft_engine::export::write_wav_f32(&samples, rec.sample_rate, &file)
                        .tr_err()?;
                    lane.push(file);
                }
            }
            lanes
        };
        for lane in lanes {
            let mut at: Option<(usize, u64)> = None;
            for file in &lane {
                let ev = self.import_audio(file, at)?;
                at = self
                    .project
                    .find(ev)
                    .and_then(|(t, _)| Some((t, self.project.event(ev)?.end())));
            }
        }
        Ok(())
    }

    /// Nach dem Wiederherstellen: Aufnahmen anbieten, die die Sicherung nicht kennt.
    pub(crate) fn orphans_dialog(&mut self, ctx: &egui::Context) {
        if self.autosave.orphans.is_empty() || self.autosave.recovery.is_some() {
            return;
        }
        let mut choice = None;
        egui::Window::new(tr("Aufnahmen nach der Sicherung"))
            .collapsible(false)
            .resizable(false)
            .anchor(egui::Align2::CENTER_CENTER, [0.0, 0.0])
            .show(ctx, |ui| {
                ui.label(tr(
                    "Diese Aufnahmen sind jünger als die Sicherung und in keinem Take:",
                ));
                for files in &self.autosave.orphans {
                    let name = files
                        .first()
                        .and_then(|p| p.file_name())
                        .unwrap_or_default()
                        .to_string_lossy();
                    match files.len() {
                        0 | 1 => ui.label(name),
                        2 => ui.label(trf("{name} (+1 Folgedatei)", &[("name", &name)])),
                        n => ui.label(trf(
                            "{name} (+{n} Folgedateien)",
                            &[("name", &name), ("n", &(n - 1))],
                        )),
                    };
                }
                ui.horizontal(|ui| {
                    if ui.button(tr("Als Takes einfügen")).clicked() {
                        choice = Some(true);
                    }
                    if ui.button(tr("Nicht einfügen")).clicked() {
                        choice = Some(false);
                    }
                });
                ui.weak(tr("Die Dateien bleiben im Aufnahmeordner."));
            });
        match choice {
            Some(true) => self.insert_orphans(),
            Some(false) => self.autosave.orphans.clear(),
            None => {}
        }
    }

    /// Untermenü Datei → „Automatisch sichern“: Abstand und Anzahl.
    pub(crate) fn autosave_menu(&mut self, ui: &mut egui::Ui) {
        ui.menu_button(tr("Automatisch sichern"), |ui| {
            let mut changed = false;
            for (m, label) in [
                (0, tr("Aus")),
                (1, tr("Jede Minute")),
                (2, tr("Alle 2 Minuten")),
                (5, tr("Alle 5 Minuten")),
                (10, tr("Alle 10 Minuten")),
                (15, tr("Alle 15 Minuten")),
            ] {
                changed |= ui
                    .radio_value(&mut self.autosave.minutes, m, label)
                    .changed();
            }
            ui.separator();
            ui.menu_button(tr("Anzahl Sicherungen"), |ui| {
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
        egui::Window::new(tr("Wiederherstellen"))
            .collapsible(false)
            .resizable(false)
            .anchor(egui::Align2::CENTER_CENTER, [0.0, 0.0])
            .show(ctx, |ui| {
                ui.label(tr("StudioCraft wurde nicht sauber beendet."));
                ui.label(crate::i18n::trf(
                    "Neueste Sicherung: {path}",
                    &[("path", &backup.display())],
                ));
                ui.horizontal(|ui| {
                    if ui.button(tr("Sicherung öffnen")).clicked() {
                        choice = Some(true);
                    }
                    if ui.button(tr("Verwerfen")).clicked() {
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

/// Fasst Folgedateien mit ihrer ersten Datei zusammen: „Stamm-NNN.wav“ (drei Ziffern) gehört
/// zu „Stamm.wav“, wenn die in `files` ist; in Nummernfolge. `files` ist sortiert.
fn group_continuations(files: Vec<PathBuf>) -> Vec<Vec<PathBuf>> {
    let part = |p: &Path| -> Option<(PathBuf, u32)> {
        let stem = p.file_stem()?.to_str()?;
        let (base, n) = stem.rsplit_once('-')?;
        (n.len() == 3 && n.bytes().all(|b| b.is_ascii_digit()))
            .then(|| (p.with_file_name(format!("{base}.wav")), n.parse().ok()))
            .and_then(|(first, n)| Some((first, n?)))
            .filter(|&(_, n)| n >= 2)
    };
    let set: std::collections::HashSet<&PathBuf> = files.iter().collect();
    let mut groups: Vec<Vec<PathBuf>> = Vec::new();
    let mut tails: Vec<(PathBuf, u32, PathBuf)> = Vec::new();
    for f in &files {
        match part(f).filter(|(first, _)| set.contains(first)) {
            Some((first, n)) => tails.push((first, n, f.clone())),
            None => groups.push(vec![f.clone()]),
        }
    }
    tails.sort_by_key(|t| t.1);
    for (first, _, f) in tails {
        if let Some(g) = groups.iter_mut().find(|g| g.first() == Some(&first)) {
            g.push(f);
        }
    }
    groups
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

    /// Takes, die nach der letzten Sicherung aufgenommen wurden, werden nach dem
    /// Wiederherstellen angeboten: nur jüngere, in keinem Clip; nichts wird gelöscht.
    #[test]
    fn takes_newer_than_the_backup_are_offered_after_a_crash() {
        let dir = temp("orphans");
        let config = dir.join("config");
        let mut w = window_with_clip();
        w.save_as(&dir.join("Song.studiocraft")).unwrap();
        let audio = dir.join(file::AUDIO_DIR);
        // Älter als die Sicherung: nicht angeboten.
        let wav = |path: &Path, channels: u16, v: f32| {
            let samples = vec![v; 4800 * usize::from(channels)];
            studiocraft_engine::export::write_wav_f32_channels(&samples, channels, 48_000, path)
                .unwrap();
        };
        wav(&audio.join("Alt.wav"), 2, 0.1);
        std::thread::sleep(Duration::from_millis(30));
        w.autosave = Autosave::new(5, 3);
        w.begin_session_in(&config);
        let before = w.project.clone();
        w.project.tracks[0].name = "Geändert".into();
        w.checkpoint(before);
        w.autosave_now();
        w.finish_autosave();
        std::thread::sleep(Duration::from_millis(30));
        // Nach der Sicherung: ein platzierter Take und eine Mehrkanal-Rohdatei (Absturz
        // während der Aufnahme).
        wav(&audio.join("Aufnahme - Mikro.wav"), 2, 0.2);
        wav(&audio.join("Aufnahme (2).wav"), 3, 0.3);
        // Lange Aufnahmen gehen in Folgedateien weiter („Stamm-002.wav“ …).
        wav(&audio.join("Aufnahme (2)-002.wav"), 3, 0.3);
        wav(&audio.join("Lang.wav"), 2, 0.4);
        wav(&audio.join("Lang-003.wav"), 2, 0.4);
        wav(&audio.join("Lang-002.wav"), 2, 0.4);
        // Eine Folgedatei ohne ihre erste Datei steht allein.
        wav(&audio.join("Allein-002.wav"), 2, 0.5);
        std::fs::write(audio.join("Notiz.txt"), "keine Aufnahme").unwrap();

        let mut next = ProjectWindow::new(Project::default(), None, String::new());
        next.begin_session_in(&config);
        let (backup, original) = next.autosave.recovery.clone().expect("recovery offered");
        next.recover(&backup, original).unwrap();
        let f = |name: &str| audio.join(name);
        assert_eq!(
            next.autosave.orphans,
            [
                vec![f("Allein-002.wav")],
                vec![f("Aufnahme (2).wav"), f("Aufnahme (2)-002.wav")],
                vec![f("Aufnahme - Mikro.wav")],
                vec![f("Lang.wav"), f("Lang-002.wav"), f("Lang-003.wav")],
            ],
            "nur jüngere WAV-Dateien ohne Clip, je Aufnahme mit ihren Folgedateien"
        );
        let tracks = next.project.tracks.len();
        let opened = next.message.clone();
        next.insert_orphans();
        assert!(next.autosave.orphans.is_empty());
        // Drei Mono-Spuren aus der Rohdatei, je eine aus den Stereo-Aufnahmen.
        assert_eq!(next.project.tracks.len(), tracks + 6, "{:?}", next.message);
        // Eine lange Aufnahme ist ein Take: ihre Dateien als Events lückenlos hintereinander.
        let lang = next
            .project
            .tracks
            .iter()
            .find(|t| t.name == "Lang")
            .unwrap();
        assert_eq!(lang.events.len(), 3);
        for pair in lang.events.windows(2) {
            assert_eq!(pair[1].start.0, pair[0].end(), "lückenlos");
        }
        let kanal = next
            .project
            .tracks
            .iter()
            .filter(|t| t.name.starts_with("Aufnahme (2) - Kanal"))
            .map(|t| t.events.len())
            .collect::<Vec<_>>();
        assert_eq!(kanal, [2, 2, 2], "je Kanal beide Teile");
        assert_eq!(next.message, opened, "keine Fehler beim Einfügen");
        for f in ["Aufnahme (2).wav", "Aufnahme - Mikro.wav", "Alt.wav"] {
            assert!(audio.join(f).exists(), "{f} bleibt liegen");
        }
        assert!(audio.join("Aufnahme (2) - Kanal 3.wav").exists());
        // Ein Undo-Schritt nimmt alles wieder heraus.
        next.undo();
        assert_eq!(next.project.tracks.len(), tracks);
        next.end_session();
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
