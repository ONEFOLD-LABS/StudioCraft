//! Insert-Effekte im Inspector (Spec „Spurtypen und Inspector“, „Mixer“).
//!
//! Das Projekt hält je Spur die Liste der Inserts (Plugin-Id, Bypass,
//! Parameter). [`ProjectWindow::sync_inserts`] gleicht die Engine damit ab:
//! geänderte Ketten werden neu erzeugt, sonst nur Bypass und Parameter gesendet.
//! So stimmen Engine und Projekt auch nach Undo und beim Öffnen überein.

use std::collections::HashMap;

use egui::Ui;
use studiocraft_engine::{Command, Insert, InsertChain};
use studiocraft_plugin_host::{ClapInstance, ParamInfo, PluginInfo};
use studiocraft_project::InsertSlot;

use crate::ProjectWindow;

/// So viele Inserts hat ein Kanal (Spec: Inserts 1–8).
pub(crate) const MAX_INSERTS: usize = 8;
/// Größter Block, den die Engine an ein Plugin gibt.
const MAX_BLOCK: u32 = 8192;

/// Was zuletzt an die Engine ging, je Spur.
#[derive(Default, Clone)]
pub(crate) struct SentChain {
    pub(crate) slots: Vec<InsertSlot>,
    /// Modell-Slot → Index in der Engine-Kette (`None`: Plugin fehlt).
    pub(crate) map: Vec<Option<usize>>,
}

/// Plugin-Liste und Parameterbeschreibungen (Hauptthread).
#[derive(Default)]
pub(crate) struct PluginState {
    pub list: Option<Vec<PluginInfo>>,
    pub params: HashMap<String, Vec<ParamInfo>>,
    pub sent: Vec<SentChain>,
}

impl ProjectWindow {
    pub(crate) fn plugin_list(&mut self) -> &[PluginInfo] {
        if self.plugins.list.is_none() {
            let (list, errors) =
                studiocraft_plugin_host::scan(&studiocraft_plugin_host::default_paths());
            if !errors.is_empty() {
                self.message = Some(format!("{} Plugin-Datei(en) nicht ladbar", errors.len()));
            }
            self.plugins.list = Some(list);
        }
        self.plugins.list.as_deref().unwrap_or_default()
    }

    fn find_plugin(&mut self, id: &str) -> Option<PluginInfo> {
        self.plugin_list().iter().find(|p| p.id == id).cloned()
    }

    /// Engine-Ketten an das Projekt angleichen (nach jeder Änderung aufgerufen).
    pub(crate) fn sync_inserts(&mut self) {
        let n = self.project.tracks.len();
        let mut sent = std::mem::take(&mut self.plugins.sent);
        // Spuren, die es nicht mehr gibt: Inserts entfernen.
        for (i, old) in sent.iter().enumerate().skip(n) {
            if !old.slots.is_empty() {
                self.send(Command::SetInserts(i, None));
            }
        }
        sent.resize(n, SentChain::default());
        let mut missing = Vec::new();
        for (i, have) in sent.iter_mut().enumerate() {
            let want = self.project.tracks[i].inserts.clone();
            let same = want.len() == have.slots.len()
                && want.iter().zip(&have.slots).all(|(a, b)| a.same_plugin(b));
            if same {
                // Nur Bypass und Parameter nachziehen.
                for (k, (w, h)) in want.iter().zip(&have.slots).enumerate() {
                    let Some(idx) = have.map[k] else { continue };
                    if w.bypass != h.bypass {
                        self.send(Command::SetBypass(i, idx, w.bypass));
                    }
                    for &(id, v) in &w.params {
                        if !h.params.contains(&(id, v)) {
                            self.send(Command::SetParam(i, idx, id, v));
                        }
                    }
                }
                have.slots = want;
                continue;
            }
            // Kette neu aufbauen.
            let mut chain = InsertChain::default();
            let mut map = Vec::with_capacity(want.len());
            for slot in &want {
                let instance = self
                    .find_plugin(&slot.plugin_id)
                    .ok_or_else(|| format!("{}: nicht installiert", slot.name))
                    .and_then(|info| ClapInstance::new(&info, self.sample_rate, MAX_BLOCK));
                match instance {
                    Ok(mut inst) => {
                        for &(id, v) in &slot.params {
                            inst.set_param(id, v);
                        }
                        self.plugins
                            .params
                            .entry(slot.plugin_id.clone())
                            .or_insert_with(|| inst.params.clone());
                        map.push(Some(chain.slots.len()));
                        chain.slots.push(Insert {
                            instance: inst,
                            bypass: slot.bypass,
                        });
                    }
                    Err(e) => {
                        missing.push(e);
                        map.push(None);
                    }
                }
            }
            let chain = (!chain.slots.is_empty()).then(|| Box::new(chain));
            if let Some(t) = &mut self.transport {
                t.send(Command::SetInserts(i, chain));
            }
            *have = SentChain { slots: want, map };
        }
        self.plugins.sent = sent;
        if !missing.is_empty() {
            self.message = Some(format!("Plugins: {}", missing.join("; ")));
        }
    }

    /// Insert-Bereich des Inspectors.
    pub(crate) fn inserts_ui(&mut self, ui: &mut Ui, track: usize) {
        let count = self.project.tracks[track].inserts.len();
        let mut remove = None;
        for k in 0..count {
            let slot = self.project.tracks[track].inserts[k].clone();
            ui.horizontal(|ui| {
                let mut on = !slot.bypass;
                if ui
                    .checkbox(&mut on, format!("{}", k + 1))
                    .on_hover_text("Ein/Aus (Bypass)")
                    .changed()
                {
                    let before = self.project.clone();
                    self.project.tracks[track].inserts[k].bypass = !on;
                    self.checkpoint(before);
                }
                ui.label(&slot.name);
                if ui
                    .small_button("x")
                    .on_hover_text("Insert entfernen")
                    .clicked()
                {
                    remove = Some(k);
                }
            });
            self.params_ui(ui, track, k);
        }
        if let Some(k) = remove {
            let before = self.project.clone();
            self.project.tracks[track].inserts.remove(k);
            self.checkpoint(before);
        }
        if count < MAX_INSERTS {
            ui.menu_button("+ Insert hinzufügen", |ui| self.plugin_menu(ui, track));
        }
    }

    fn plugin_menu(&mut self, ui: &mut Ui, track: usize) {
        let effects: Vec<PluginInfo> = self
            .plugin_list()
            .iter()
            .filter(|p| !p.is_instrument())
            .cloned()
            .collect();
        if effects.is_empty() {
            ui.label("Keine CLAP-Effekte gefunden");
            return;
        }
        let mut by_vendor: Vec<(String, Vec<PluginInfo>)> = Vec::new();
        for p in effects {
            let vendor = if p.vendor.is_empty() {
                "Ohne Hersteller".to_owned()
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
                            if ui.button(&p.name).clicked() {
                                let before = self.project.clone();
                                self.project.tracks[track].inserts.push(InsertSlot {
                                    plugin_id: p.id.clone(),
                                    name: p.name.clone(),
                                    bypass: false,
                                    params: Vec::new(),
                                });
                                self.checkpoint(before);
                                ui.close();
                            }
                        }
                    });
            });
        }
    }

    /// Allgemeine Parameteransicht (Regler je Parameter). Eigene Plugin-Fenster folgen.
    fn params_ui(&mut self, ui: &mut Ui, track: usize, k: usize) {
        let slot = &self.project.tracks[track].inserts[k];
        let Some(infos) = self.plugins.params.get(&slot.plugin_id).cloned() else {
            return;
        };
        egui::CollapsingHeader::new(format!("Parameter ({})", infos.len()))
            .id_salt(("insert-params", track, k))
            .show(ui, |ui| {
                egui::ScrollArea::vertical()
                    .id_salt(("insert-params-scroll", track, k))
                    .max_height(280.0)
                    .show(ui, |ui| {
                        for info in &infos {
                            let slot = &self.project.tracks[track].inserts[k];
                            let mut v = slot
                                .params
                                .iter()
                                .find(|(i, _)| *i == info.id)
                                .map_or(info.value, |p| p.1);
                            let mut slider =
                                egui::Slider::new(&mut v, info.min..=info.max).text(&info.name);
                            if info.stepped {
                                slider = slider.step_by(1.0);
                            }
                            if ui.add(slider).changed() {
                                self.project.tracks[track].inserts[k].set_param(info.id, v);
                                let chain_idx = self
                                    .plugins
                                    .sent
                                    .get(track)
                                    .and_then(|s| s.map.get(k).copied().flatten());
                                if let Some(idx) = chain_idx {
                                    self.send(Command::SetParam(track, idx, info.id, v));
                                    if let Some(s) = self
                                        .plugins
                                        .sent
                                        .get_mut(track)
                                        .and_then(|s| s.slots.get_mut(k))
                                    {
                                        s.set_param(info.id, v);
                                    }
                                }
                            }
                        }
                    });
            });
    }
}
