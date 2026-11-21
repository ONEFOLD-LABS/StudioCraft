//! Bearbeitungen eines Clips (Issue #53, Modell in `studiocraft_project::processing`): Liste der
//! Offline-Bearbeitungen des Clips, den ein Event spielt. Einträge lassen sich ein- und
//! ausschalten, umsortieren und entfernen; danach wird aus dem Original-Clip neu gerendert.
//! Jede Änderung ist ein Rückgängig-Schritt.
//!
//! Plugins (Teil 2): Ein Eintrag kann ein Effekt-Plugin mit seinen Einstellungen sein. Beim
//! Rendern wird es instanziiert, rechnet den Bereich blockweise in einem eigenen Thread (wie das
//! Rendern an Ort und Stelle) und wird danach im Hauptthread freigegeben. Seine Latenz wird
//! ausgeglichen; die Länge des Clips bleibt.

use egui::Ui;
use studiocraft_engine::offline;
use studiocraft_project::{
    ClipId, ClipProcess, Derived, EventId, InsertSlot, PoolClip, ProcessKind,
};

use crate::i18n::{tr, trf};
use crate::waveform::Peaks;
use crate::{LoadedClip, ProjectWindow};

/// Blockgröße beim Offline-Rechnen durch ein Plugin (höchstens die beim Anlegen genannte).
const BLOCK: usize = 4096;

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
        steps.push(ClipProcess::new(kind, range.0, range.1));
        self.rerender_processes(id, source, steps)
    }

    /// Hängt ein Effekt-Plugin mit den Einstellungen aus `slot` an die Liste an und rendert.
    pub(crate) fn add_plugin_process(
        &mut self,
        id: EventId,
        slot: InsertSlot,
        range: (u64, u64),
    ) -> Result<(), String> {
        let (source, mut steps) = self.event_processes(id).ok_or(tr("kein Audio-Event"))?;
        if steps.len() >= studiocraft_project::processing::MAX_PROCESSES {
            return Err(tr("zu viele Bearbeitungen").into());
        }
        steps.push(ClipProcess {
            plugin: Some(InsertSlot {
                bypass: false,
                ..slot
            }),
            ..ClipProcess::new(ProcessKind::Plugin, range.0, range.1)
        });
        self.rerender_processes(id, source, steps)
    }

    /// Rendert alle eingeschalteten Einträge nacheinander auf eine Kopie von `source`.
    fn render_steps(&mut self, source: &[f32], steps: &[ClipProcess]) -> Result<Vec<f32>, String> {
        let mut out = source.to_vec();
        for s in steps.iter().filter(|s| s.on) {
            match (&s.kind, &s.plugin) {
                (ProcessKind::Plugin, Some(slot)) => {
                    let slot = slot.clone();
                    self.plugin_offline(&mut out, &slot, (s.from, s.to))?;
                }
                (ProcessKind::Plugin, None) => {}
                _ => offline::apply(&mut out, s),
            }
        }
        Ok(out)
    }

    /// Rechnet den Frame-Bereich `range` von `samples` (Stereo) durch das Plugin aus `slot`.
    fn plugin_offline(
        &mut self,
        samples: &mut [f32],
        slot: &InsertSlot,
        range: (u64, u64),
    ) -> Result<(), String> {
        let n = samples.len() / 2;
        let a = usize::try_from(range.0).unwrap_or(n).min(n);
        let b = usize::try_from(range.1).unwrap_or(n).min(n).max(a);
        if b == a {
            return Ok(());
        }
        let mut inst = self.instantiate(slot)?;
        // Faster than real time: a plugin in its own process waits for every block.
        inst.set_offline(true);
        let latency = inst.handle().latency() as usize;
        // Eingang mit Stille für die Latenz verlängert; die ersten `latency` Frames der Ausgabe
        // fallen weg.
        let len = b - a + latency;
        let mut left = vec![0.0f32; len];
        let mut right = vec![0.0f32; len];
        for (k, f) in samples[a * 2..b * 2].chunks_exact(2).enumerate() {
            left[k] = f[0];
            right[k] = f[1];
        }
        let work = move || {
            for (l, r) in left.chunks_mut(BLOCK).zip(right.chunks_mut(BLOCK)) {
                inst.process(l, r);
            }
            inst.stop_processing();
            (inst, left, right)
        };
        #[cfg(not(target_arch = "wasm32"))]
        let (inst, left, right) = std::thread::Builder::new()
            .name("studiocraft-offline".into())
            .spawn(work)
            .map_err(|e| format!("Offline-Thread: {e}"))?
            .join()
            .map_err(|_| tr("Plugin-Bearbeitung abgebrochen").to_owned())?;
        #[cfg(target_arch = "wasm32")]
        let (inst, left, right) = work();
        // Instanz im Hauptthread freigeben. Im Browser gibt es keine Plugins (der Typ dort hat
        // kein Drop), daher dort ohne Wirkung.
        #[allow(clippy::drop_non_drop)]
        drop(inst);
        for (k, f) in samples[a * 2..b * 2].chunks_exact_mut(2).enumerate() {
            let (l, r) = (left[k + latency], right[k + latency]);
            f[0] = if l.is_finite() { l } else { 0.0 };
            f[1] = if r.is_finite() { r } else { 0.0 };
        }
        Ok(())
    }

    /// Menü „Plugin anwenden“: die Inserts der eigenen Spur (mit ihren Einstellungen) und alle
    /// Effekt-Plugins (Standardeinstellungen). Wendet das gewählte auf `range` an.
    pub(crate) fn plugin_process_menu(&mut self, ui: &mut Ui, id: EventId, range: (u64, u64)) {
        let mut chosen: Option<InsertSlot> = None;
        let own: Vec<InsertSlot> = self
            .project
            .find(id)
            .map(|(t, _)| self.project.tracks[t].inserts.clone())
            .unwrap_or_default();
        if !own.is_empty() {
            ui.label(egui::RichText::new(tr("Inserts der Spur")).strong());
            for s in own {
                if ui.button(&s.name).clicked() {
                    chosen = Some(s);
                    ui.close();
                }
            }
            ui.separator();
        }
        let effects: Vec<studiocraft_plugin_host::PluginInfo> = self
            .plugin_list()
            .iter()
            .filter(|p| !p.is_instrument())
            .cloned()
            .collect();
        if effects.is_empty() {
            ui.weak(tr("Keine Effekte gefunden"));
        }
        // Nach Hersteller gruppiert wie beim Einfügen eines Inserts.
        let mut by_vendor: Vec<(String, Vec<studiocraft_plugin_host::PluginInfo>)> = Vec::new();
        for p in effects {
            let vendor = if p.vendor.is_empty() {
                tr("Ohne Hersteller").to_owned()
            } else {
                p.vendor.clone()
            };
            match by_vendor.iter_mut().find(|(v, _)| *v == vendor) {
                Some((_, list)) => list.push(p),
                None => by_vendor.push((vendor, vec![p])),
            }
        }
        by_vendor.sort_by_key(|(v, _)| v.to_lowercase());
        for (vendor, list) in by_vendor {
            ui.menu_button(format!("{vendor} ({})", list.len()), |ui| {
                egui::ScrollArea::vertical()
                    .max_height(480.0)
                    .show(ui, |ui| {
                        for p in list {
                            if ui.button(p.display_name()).clicked() {
                                chosen = Some(self.new_slot(&p));
                                ui.close();
                            }
                        }
                    });
            });
        }
        if let Some(slot) = chosen
            && let Err(e) = self.add_plugin_process(id, slot, range)
        {
            self.message = Some(trf("Bearbeitungen: {e}", &[("e", &e)]));
        }
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
            let original = self
                .clip(source)
                .ok_or(tr("Clip ist nicht geladen"))?
                .samples
                .clone();
            let samples = self.render_steps(&original, &steps)?;
            let names: Vec<&str> = steps
                .iter()
                .filter(|s| s.on)
                .map(|s| match s.kind {
                    ProcessKind::Plugin => s.name(),
                    k => tr(k.label()),
                })
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
                // Lange Namen brechen in der schmalen Liste um, statt am Rand abzureißen.
                ui.style_mut().wrap_mode = Some(egui::TextWrapMode::Wrap);
                let mut on = s.on;
                let name = match s.kind {
                    ProcessKind::Gain { db } => {
                        trf("Gain {db} dB", &[("db", &format!("{db:+.1}"))])
                    }
                    ProcessKind::Normalize { target_db } => trf(
                        "Normalisieren auf {db} dB",
                        &[("db", &format!("{target_db:.1}"))],
                    ),
                    ProcessKind::Plugin => s.name().to_owned(),
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

#[cfg(test)]
mod tests {
    use super::*;
    use studiocraft_engine::AudioClip;
    use studiocraft_project::{Project, processing::MAX_PROCESSES};

    /// Ein Event mit einem Sinus (Spitze 0,25) über eine Sekunde.
    fn window() -> (ProjectWindow, EventId, ClipId) {
        let mut w = ProjectWindow::new(Project::default(), None, String::new());
        let clip = AudioClip {
            sample_rate: 48_000,
            samples: (0..48_000)
                .flat_map(|i| {
                    let v = (i as f32 * 0.05).sin() * 0.25;
                    [v, v]
                })
                .collect::<Vec<f32>>()
                .into(),
        };
        let id = w.import_clip("Take", None, clip, None).unwrap();
        let source = w.project.event(id).unwrap().clip.unwrap();
        w.clear_history();
        (w, id, source)
    }

    fn peak(w: &ProjectWindow, id: EventId) -> f32 {
        let clip = w.project.event(id).unwrap().clip.unwrap();
        w.clip(clip)
            .unwrap()
            .samples
            .iter()
            .fold(0.0, |m, v| m.max(v.abs()))
    }

    fn db(v: f32) -> f32 {
        20.0 * v.log10()
    }

    /// Die Liste rechnet immer vom Original aus, in ihrer Reihenfolge, nur eingeschaltete
    /// Einträge; das Original bleibt unverändert, jede Änderung ist ein Undo-Schritt.
    #[test]
    fn the_list_renders_from_the_original_in_order() {
        let (mut w, id, source) = window();
        let original = w.clip(source).unwrap().samples.clone();
        let full = (0, 48_000);
        w.add_process(id, ProcessKind::Gain { db: 6.0 }, full)
            .unwrap();
        w.add_process(id, ProcessKind::Normalize { target_db: -6.0 }, full)
            .unwrap();
        assert!(
            (db(peak(&w, id)) + 6.0).abs() < 0.05,
            "Gain, dann normalisiert"
        );

        // Umsortieren: erst normalisieren, dann +6 dB → Vollaussteuerung.
        let (src, mut steps) = w.event_processes(id).unwrap();
        assert_eq!(src, source);
        steps.swap(0, 1);
        w.rerender_processes(id, src, steps).unwrap();
        assert!(db(peak(&w, id)).abs() < 0.05, "{}", db(peak(&w, id)));

        // Ausgeschaltet zählt ein Eintrag nicht.
        let (src, mut steps) = w.event_processes(id).unwrap();
        steps[1].on = false;
        w.rerender_processes(id, src, steps).unwrap();
        assert!((db(peak(&w, id)) + 6.0).abs() < 0.05);
        let (_, steps) = w.event_processes(id).unwrap();
        assert_eq!(steps.len(), 2, "ausgeschaltet bleibt in der Liste");

        // Alles entfernt: das Event spielt wieder den Original-Clip.
        w.rerender_processes(id, source, Vec::new()).unwrap();
        assert_eq!(w.project.event(id).unwrap().clip, Some(source));
        assert_eq!(
            w.clip(source).unwrap().samples,
            original,
            "Original unverändert"
        );

        // Jeder der fünf Schritte lässt sich zurücknehmen.
        w.undo();
        assert!((db(peak(&w, id)) + 6.0).abs() < 0.05);
        let lengths: Vec<usize> = (0..4)
            .map(|_| {
                w.undo();
                w.clip(w.project.event(id).unwrap().clip.unwrap())
                    .unwrap()
                    .samples
                    .len()
            })
            .collect();
        assert_eq!(lengths, [96_000; 4], "die Länge bleibt");
        assert_eq!(w.project.event(id).unwrap().clip, Some(source));
    }

    /// Ein Plugin, das sich nicht laden lässt: Fehler, und am Projekt ändert sich nichts.
    #[test]
    fn a_plugin_that_fails_leaves_the_project_unchanged() {
        let (mut w, id, _) = window();
        w.add_process(id, ProcessKind::FadeIn, (0, 4800)).unwrap();
        let before = w.project.clone();
        let depth = w.history_depth();
        let slot = InsertSlot {
            plugin_id: "gibt-es-nicht".into(),
            name: "Fehlt".into(),
            bypass: false,
            params: Vec::new(),
            state: None,
            isolated: false,
        };
        assert!(w.add_plugin_process(id, slot, (0, 24_000)).is_err());
        assert_eq!(w.project, before);
        assert_eq!(w.history_depth(), depth, "kein Undo-Schritt");
        // Ein Plugin-Eintrag ohne Plugin wird übersprungen, statt zu scheitern.
        let (src, mut steps) = w.event_processes(id).unwrap();
        steps.push(ClipProcess::new(ProcessKind::Plugin, 0, 24_000));
        w.rerender_processes(id, src, steps).unwrap();
    }

    #[test]
    fn the_list_has_a_limit() {
        let (mut w, id, _) = window();
        for _ in 0..MAX_PROCESSES {
            w.add_process(id, ProcessKind::InvertPhase, (0, 10))
                .unwrap();
        }
        assert!(
            w.add_process(id, ProcessKind::InvertPhase, (0, 10))
                .is_err()
        );
        assert_eq!(w.event_processes(id).unwrap().1.len(), MAX_PROCESSES);
    }
}
