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
                                chosen = Some(InsertSlot {
                                    plugin_id: p.id.clone(),
                                    name: p.name.clone(),
                                    bypass: false,
                                    params: Vec::new(),
                                    state: None,
                                    isolated: false,
                                });
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
