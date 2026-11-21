//! Update-Suche und Selbst-Update in der Oberfläche: Einträge im Menü Hilfe und die Leiste
//! unter dem Menü, die eine neue Version meldet (docs/selbst-update.md).

use std::path::PathBuf;
use std::sync::mpsc;

use egui::{RichText, Ui};

use crate::ProjectWindow;
use crate::i18n::{tr, trf};
use crate::menu::Action;
use crate::update;

/// Antwort einer Suche.
type CheckRx = mpsc::Receiver<Result<Option<update::Release>, String>>;

/// Stand der Update-Suche dieses Fensters.
#[derive(Default)]
pub(crate) struct Updates {
    /// Läuft gerade eine Suche (`manual`: vom Menü gestartet, meldet auch „aktuell“).
    checking: Option<(CheckRx, bool)>,
    /// Die gefundene neue Version.
    found: Option<update::Release>,
    /// Läuft gerade das Herunterladen und Prüfen.
    installing: Option<mpsc::Receiver<Result<update::Installed, String>>>,
    /// Was nach dem Installieren zu tun ist.
    done: Option<update::Installed>,
    /// Beim Beenden dieses Programm starten (nach „Neu starten“).
    pub(crate) restart: Option<PathBuf>,
    /// Beim ersten Bild der Sitzung aufgeräumt und gesucht.
    started: bool,
}

fn spawn<T: Send + 'static>(
    ctx: &egui::Context,
    name: &str,
    job: impl FnOnce() -> T + Send + 'static,
) -> Result<mpsc::Receiver<T>, String> {
    let (tx, rx) = mpsc::channel();
    let ctx = ctx.clone();
    std::thread::Builder::new()
        .name(name.into())
        .spawn(move || {
            let _ = tx.send(job());
            ctx.request_repaint();
        })
        .map_err(|e| e.to_string())?;
    Ok(rx)
}

impl ProjectWindow {
    /// Einmal je Sitzung: Reste eines früheren Updates entfernen und, wenn eingeschaltet und
    /// fällig, nach einer neuen Version suchen. Nur in der App (`begin_session`), nicht in Tests.
    pub(crate) fn tick_updates(&mut self, ctx: &egui::Context) {
        if !self.updates.started && self.autosave.dir.is_some() {
            self.updates.started = true;
            update::clean_up();
            self.check_for_update(ctx, false);
        }
        self.poll_updates();
    }

    /// Sucht nach einer neuen Version: höchstens einmal am Tag oder sofort (`manual`).
    pub(crate) fn check_for_update(&mut self, ctx: &egui::Context, manual: bool) {
        if self.updates.checking.is_some() || !(manual || self.update_check) {
            return;
        }
        let now = update::now_secs();
        if !manual && !update::due(self.last_update_check, now) {
            return;
        }
        self.last_update_check = now;
        self.save_settings();
        match spawn(ctx, "studiocraft-update-check", || {
            update::check(&update::agent(), update::RELEASES)
        }) {
            Ok(rx) => self.updates.checking = Some((rx, manual)),
            Err(e) => self.message = Some(e),
        }
    }

    fn poll_updates(&mut self) {
        if let Some((rx, manual)) = &self.updates.checking {
            let manual = *manual;
            match rx.try_recv() {
                Ok(Ok(Some(release))) => {
                    self.updates.found = Some(release);
                    self.updates.checking = None;
                }
                Ok(Ok(None)) => {
                    if manual {
                        self.message = Some(trf(
                            "StudioCraft {version} ist aktuell.",
                            &[("version", &env!("CARGO_PKG_VERSION"))],
                        ));
                    }
                    self.updates.checking = None;
                }
                // Eine tägliche Suche ohne Netz bleibt still bis zum nächsten Tag.
                Ok(Err(e)) => {
                    if manual {
                        self.message = Some(trf("Update-Suche fehlgeschlagen: {e}", &[("e", &e)]));
                    }
                    self.updates.checking = None;
                }
                Err(mpsc::TryRecvError::Empty) => {}
                Err(mpsc::TryRecvError::Disconnected) => self.updates.checking = None,
            }
        }
        if let Some(rx) = &self.updates.installing {
            match rx.try_recv() {
                Ok(Ok(done)) => {
                    self.updates.done = Some(done);
                    self.updates.installing = None;
                }
                Ok(Err(e)) => {
                    self.message = Some(trf("Update nicht installiert: {e}", &[("e", &e)]));
                    self.updates.installing = None;
                }
                Err(mpsc::TryRecvError::Empty) => {}
                Err(mpsc::TryRecvError::Disconnected) => self.updates.installing = None,
            }
        }
    }

    /// Die Einträge im Menü Hilfe.
    pub(crate) fn update_menu(&mut self, ui: &mut Ui) {
        let mut on = self.update_check;
        if ui
            .checkbox(&mut on, tr("Täglich nach Updates suchen"))
            .on_hover_text(tr(
                "Fragt einmal am Tag bei GitHub nach einer neuen Version; sonst wird nichts gesendet.",
            ))
            .changed()
        {
            self.update_check = on;
            self.save_settings();
            if on {
                self.check_for_update(ui.ctx(), false);
            }
        }
        let busy = self.updates.checking.is_some();
        if ui
            .add_enabled(!busy, egui::Button::new(tr("Jetzt nach Updates suchen")))
            .clicked()
        {
            self.check_for_update(ui.ctx(), true);
            ui.close();
        }
    }

    /// Die Leiste, die eine neue Version meldet: installieren (wo StudioCraft das kann) oder
    /// die Release-Seite öffnen.
    pub(crate) fn update_bar(&mut self, ui: &mut Ui) {
        if self.updates.found.is_none() && self.updates.done.is_none() {
            return;
        }
        let t = crate::theme::tokens(ui.ctx());
        if let Some(done) = self.updates.done.clone() {
            let mut restart = false;
            egui::Panel::top("update").show(ui, |ui| {
                ui.horizontal_wrapped(|ui| {
                    let text = match done {
                        update::Installed::Restart(_) => {
                            tr("Das Update ist installiert und gilt nach einem Neustart.")
                        }
                        update::Installed::InstallerStarted => tr(
                            "Das Installationsprogramm läuft; beende StudioCraft, damit es die Dateien ersetzen kann.",
                        ),
                    };
                    ui.label(RichText::new(text).color(t.text));
                    if let update::Installed::Restart(_) = done {
                        restart = ui.button(tr("Neu starten")).clicked();
                    }
                });
            });
            if restart && let update::Installed::Restart(path) = done {
                self.updates.restart = Some(path);
                self.request(ui, Action::Quit);
            }
            return;
        }
        let Some(release) = self.updates.found.clone() else {
            return;
        };
        let install = update::this_install();
        let can_install = update::public_key().is_some()
            && update::assets_for(
                &release,
                &install,
                std::env::consts::OS,
                std::env::consts::ARCH,
            )
            .is_some();
        let mut later = false;
        let mut start = false;
        let installing = self.updates.installing.is_some();
        egui::Panel::top("update").show(ui, |ui| {
            ui.horizontal_wrapped(|ui| {
                ui.label(
                    RichText::new(trf(
                        "StudioCraft {version} ist verfügbar.",
                        &[("version", &release.version)],
                    ))
                    .color(t.text),
                );
                if installing {
                    ui.add(egui::Spinner::new());
                    ui.label(RichText::new(tr("Lade und prüfe die Signatur …")).color(t.text_dim));
                    return;
                }
                if can_install {
                    start = ui
                        .button(tr("Installieren"))
                        .on_hover_text(tr(
                            "Lädt die neue Version, prüft ihre Signatur und ersetzt erst dann diese.",
                        ))
                        .clicked();
                } else if install == update::Install::Managed {
                    ui.label(
                        RichText::new(tr(
                            "Aktualisiere über die Paketverwaltung oder lade es von der Release-Seite.",
                        ))
                        .color(t.text_dim),
                    );
                }
                if ui.link(tr("Release-Seite")).clicked() {
                    ui.ctx().open_url(egui::OpenUrl::new_tab(&release.page));
                }
                later = ui.small_button(tr("Später")).clicked();
            });
        });
        if later {
            self.updates.found = None;
        }
        if start {
            match spawn(ui.ctx(), "studiocraft-update", move || {
                update::install(&update::agent(), &release, &install)
            }) {
                Ok(rx) => self.updates.installing = Some(rx),
                Err(e) => self.message = Some(e),
            }
        }
    }

    /// Beim Beenden nach „Neu starten“: die neue Version starten.
    pub(crate) fn start_restart(&mut self) {
        if let Some(path) = self.updates.restart.take()
            && let Err(e) = std::process::Command::new(&path).spawn()
        {
            self.message = Some(trf("Neustart nicht möglich: {e}", &[("e", &e.to_string())]));
        }
    }

    #[cfg(test)]
    pub(crate) fn show_update(&mut self, release: update::Release) {
        self.updates.found = Some(release);
    }
}
