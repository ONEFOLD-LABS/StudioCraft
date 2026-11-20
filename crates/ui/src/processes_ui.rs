//! Bearbeitungen eines Clips (Issue #53, Modell in `studiocraft_project::processing`): Liste der
//! Offline-Bearbeitungen des Clips, den ein Event spielt. Einträge lassen sich ein- und
//! ausschalten, umsortieren und entfernen; danach wird aus dem Original-Clip neu gerendert.
//! Jede Änderung ist ein Rückgängig-Schritt.

use egui::Ui;
use studiocraft_engine::offline;
use studiocraft_project::{ClipId, ClipProcess, Derived, EventId, PoolClip, ProcessKind};

use crate::i18n::{tr, trf};
use crate::waveform::Peaks;
use crate::{LoadedClip, ProjectWindow};

/// Eine Änderung an der Liste.
#[derive(Debug, Clone, Copy, PartialEq)]
enum ListEdit {
    Toggle(usize),
    Up(usize),
    Remove(usize),
}

impl ProjectWindow {
    /// Original-Clip und Bearbeitungen des Clips, den Event `id` spielt.
    pub(crate) fn event_processes(&self, id: EventId) -> Option<(ClipId, Vec<ClipProcess>)> {
        let clip = self.project.event(id)?.clip?;
        Some(self.project.clip_origin(clip))
    }

    /// Hängt eine Bearbeitung an die Liste des Events an (Frame-Bereich des Clips) und rendert.
    pub(crate) fn add_process(
        &mut self,
        id: EventId,
        kind: ProcessKind,
        range: (u64, u64),
    ) -> Result<(), String> {
        let (source, mut steps) = self.event_processes(id).ok_or(tr("kein Audio-Event"))?;
        if steps.len() >= studiocraft_project::processing::MAX_PROCESSES {
            return Err(tr("zu viele Bearbeitungen").into());
        }
        steps.push(ClipProcess {
            kind,
            from: range.0.min(range.1),
            to: range.0.max(range.1),
            on: true,
        });
        self.rerender_processes(id, source, steps)
    }

    /// Rendert `steps` auf den Original-Clip `source` und lässt Event `id` den Ergebnis-Clip
    /// spielen (ohne Bearbeitungen wieder das Original). Ein Rückgängig-Schritt.
    pub(crate) fn rerender_processes(
        &mut self,
        id: EventId,
        source: ClipId,
        steps: Vec<ClipProcess>,
    ) -> Result<(), String> {
        let pool = self
            .project
            .pool
            .get(source as usize)
            .cloned()
            .ok_or(tr("Clip fehlt im Pool"))?;
        let before = self.project.clone();
        let target = if steps.is_empty() {
            source
        } else {
            let original = self.clip(source).ok_or(tr("Clip ist nicht geladen"))?;
            let samples = offline::render(&original.samples, &steps);
            let names: Vec<&str> = steps
                .iter()
                .filter(|s| s.on)
                .map(|s| tr(s.kind.label()))
                .collect();
            let name = if names.is_empty() {
                pool.name.clone()
            } else {
                format!("{} ({})", pool.name, names.join(", "))
            };
            let new_id = self.project.add_clip(PoolClip {
                name,
                path: None,
                sample_rate: pool.sample_rate,
                frames: (samples.len() / 2) as u64,
            });
            let samples: std::sync::Arc<[f32]> = samples.into();
            self.clips.insert(
                new_id,
                LoadedClip {
                    peaks: Peaks::new(&samples),
                    samples,
                },
            );
            self.project
                .derived
                .insert(new_id, Derived { source, steps });
            new_id
        };
        if let Some(e) = self.project.event_mut(id) {
            e.clip = Some(target);
        }
        self.checkpoint(before);
        self.dirty = true;
        Ok(())
    }

    /// Liste der Bearbeitungen des Events mit An/Aus, Hoch und Entfernen.
    pub(crate) fn processes_panel(&mut self, ui: &mut Ui, id: EventId) {
        let Some((source, steps)) = self.event_processes(id) else {
            return;
        };
        let rate = self
            .project
            .pool
            .get(source as usize)
            .map_or(48_000, |c| c.sample_rate.max(1));
        let secs = |f: u64| format!("{:.2}", f as f64 / f64::from(rate));
        let mut edit = None;
        ui.label(egui::RichText::new(tr("Bearbeitungen")).strong());
        if steps.is_empty() {
            ui.weak(tr("Keine – das Original klingt"));
        }
        for (k, s) in steps.iter().enumerate() {
            ui.horizontal(|ui| {
                let mut on = s.on;
                let name = match s.kind {
                    ProcessKind::Gain { db } => {
                        trf("Gain {db} dB", &[("db", &format!("{db:+.1}"))])
                    }
                    ProcessKind::Normalize { target_db } => trf(
                        "Normalisieren auf {db} dB",
                        &[("db", &format!("{target_db:.1}"))],
                    ),
                    other => tr(other.label()).to_owned(),
                };
                let up = ui.add_enabled(k > 0, egui::Button::new(tr("hoch")).small());
                up.widget_info(|| {
                    egui::WidgetInfo::labeled(
                        egui::Role::Button,
                        k > 0,
                        trf("Bearbeitung {nr} nach oben", &[("nr", &(k + 1))]),
                    )
                });
                if up.clicked() {
                    edit = Some(ListEdit::Up(k));
                }
                let x = ui.small_button(tr("x"));
                x.widget_info(|| {
                    egui::WidgetInfo::labeled(
                        egui::Role::Button,
                        true,
                        trf("Bearbeitung {nr} entfernen", &[("nr", &(k + 1))]),
                    )
                });
                if x.clicked() {
                    edit = Some(ListEdit::Remove(k));
                }
                if ui
                    .checkbox(&mut on, format!("{}. {name}", k + 1))
                    .on_hover_text(trf(
                        "{a}–{b} s",
                        &[("a", &secs(s.from)), ("b", &secs(s.to))],
                    ))
                    .changed()
                {
                    edit = Some(ListEdit::Toggle(k));
                }
            });
        }
        let Some(edit) = edit else {
            return;
        };
        let mut steps = steps;
        match edit {
            ListEdit::Toggle(k) => {
                if let Some(s) = steps.get_mut(k) {
                    s.on = !s.on;
                }
            }
            ListEdit::Up(k) if k > 0 && k < steps.len() => steps.swap(k - 1, k),
            ListEdit::Remove(k) if k < steps.len() => {
                steps.remove(k);
            }
            _ => return,
        }
        if let Err(e) = self.rerender_processes(id, source, steps) {
            self.message = Some(trf("Bearbeitungen: {e}", &[("e", &e)]));
        }
    }
}
