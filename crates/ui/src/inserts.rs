//! Insert-Effekte im Inspector (Spec „Spurtypen und Inspector“, „Mixer“).
//!
//! Das Projekt hält je Spur die Liste der Inserts (Plugin-Id, Bypass,
//! Parameter). [`ProjectWindow::sync_inserts`] gleicht die Engine damit ab:
//! geänderte Ketten werden neu erzeugt, sonst nur Bypass und Parameter gesendet.
//! So stimmen Engine und Projekt auch nach Undo und beim Öffnen überein.

use std::collections::HashMap;

use base64::Engine as _;
use egui::Ui;
use studiocraft_engine::{Command, Insert, InsertChain};
use studiocraft_plugin_host::{ClapInstance, ParamInfo, PluginHandle, PluginInfo};
use studiocraft_project::InsertSlot;

use crate::ProjectWindow;

const B64: base64::engine::GeneralPurpose = base64::engine::general_purpose::STANDARD;

/// So viele Inserts hat ein Kanal (Spec: Inserts 1–8).
pub(crate) const MAX_INSERTS: usize = 8;
/// Größter Block, den die Engine an ein Plugin gibt.
const MAX_BLOCK: u32 = 8192;

/// Slot-Nummer für das Instrument einer Instrument-Spur (Inserts zählen ab 0).
pub(crate) const INSTRUMENT: usize = usize::MAX;

/// Was zuletzt an die Engine ging, je Spur.
#[derive(Default)]
pub(crate) struct SentChain {
    /// Hauptthread-Griffe je Modell-Slot (für Zustand und Fenster).
    pub(crate) handles: Vec<Option<PluginHandle>>,
    pub(crate) instrument_handle: Option<PluginHandle>,
    pub(crate) instrument: Option<InsertSlot>,
    /// Instrument läuft in der Engine (Plugin vorhanden).
    pub(crate) instrument_ok: bool,
    pub(crate) slots: Vec<InsertSlot>,
    /// Modell-Slot → Index in der Engine-Kette (`None`: Plugin fehlt).
    pub(crate) map: Vec<Option<usize>>,
    /// Kette, die (noch) nicht in der Engine ist – ohne Audio oder bei voller
    /// Befehls-Queue. Hält die Instanzen am Leben, auf die `handles` zeigen.
    /// Muss das letzte Feld sein: Felder werden in dieser Reihenfolge freigegeben,
    /// die Griffe also vor den Instanzen.
    _offline: Option<Box<InsertChain>>,
}

/// Plugin-Liste und Parameterbeschreibungen (Hauptthread).
#[derive(Default)]
pub(crate) struct PluginState {
    /// Ein Plugin hat Änderungen gemeldet, die noch nicht gespeichert sind.
    pub changed: bool,
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

    pub(crate) fn slot(&self, track: usize, k: usize) -> Option<&InsertSlot> {
        let t = self.project.tracks.get(track)?;
        if k == INSTRUMENT {
            t.instrument.as_ref()
        } else {
            t.inserts.get(k)
        }
    }

    fn slot_mut(&mut self, track: usize, k: usize) -> Option<&mut InsertSlot> {
        let t = self.project.tracks.get_mut(track)?;
        if k == INSTRUMENT {
            t.instrument.as_mut()
        } else {
            t.inserts.get_mut(k)
        }
    }

    /// Index des Slots in der Engine-Kette (`None`: Plugin fehlt).
    fn engine_index(&self, track: usize, k: usize) -> Option<usize> {
        let sent = self.plugins.sent.get(track)?;
        if k == INSTRUMENT {
            sent.instrument_ok.then_some(INSTRUMENT)
        } else {
            sent.map.get(k).copied().flatten()
        }
    }

    fn instantiate(&mut self, slot: &InsertSlot) -> Result<ClapInstance, String> {
        let info = self
            .find_plugin(&slot.plugin_id)
            .ok_or_else(|| format!("{}: nicht installiert", slot.name))?;
        let mut inst = ClapInstance::new(&info, self.sample_rate, MAX_BLOCK)?;
        if let Some(state) = slot.state.as_deref().and_then(|s| B64.decode(s).ok())
            && !inst.handle().load_state(&state)
        {
            self.message = Some(format!(
                "{}: gespeicherter Zustand ließ sich nicht laden",
                slot.name
            ));
        }
        for &(id, v) in &slot.params {
            inst.set_param(id, v);
        }
        self.plugins
            .params
            .entry(slot.plugin_id.clone())
            .or_insert_with(|| inst.params.clone());
        Ok(inst)
    }

    /// Engine-Ketten an das Projekt angleichen (nach jeder Änderung aufgerufen).
    pub(crate) fn sync_inserts(&mut self) {
        let n = self.project.tracks.len();
        let mut sent = std::mem::take(&mut self.plugins.sent);
        // Spuren, die es nicht mehr gibt: Kette entfernen.
        for (i, old) in sent.iter().enumerate().skip(n) {
            if !old.slots.is_empty() || old.instrument.is_some() {
                self.send(Command::SetInserts(i, None));
            }
        }
        sent.resize_with(n, SentChain::default);
        let mut missing = Vec::new();
        for (i, have) in sent.iter_mut().enumerate() {
            let want = self.project.tracks[i].inserts.clone();
            let want_instr = self.project.tracks[i].instrument.clone();
            let same_instr = match (&want_instr, &have.instrument) {
                (None, None) => true,
                (Some(a), Some(b)) => a.same_plugin(b),
                _ => false,
            };
            let same = same_instr
                && want.len() == have.slots.len()
                && want.iter().zip(&have.slots).all(|(a, b)| a.same_plugin(b));
            if same {
                // Nur Bypass und Parameter nachziehen.
                let mut pairs: Vec<(Option<usize>, &InsertSlot, &InsertSlot)> = want
                    .iter()
                    .zip(&have.slots)
                    .enumerate()
                    .map(|(k, (w, h))| (have.map[k], w, h))
                    .collect();
                if let (Some(w), Some(h)) = (&want_instr, &have.instrument) {
                    pairs.push((have.instrument_ok.then_some(INSTRUMENT), w, h));
                }
                let mut cmds = Vec::new();
                for (idx, w, h) in pairs {
                    let Some(idx) = idx else { continue };
                    if w.bypass != h.bypass {
                        cmds.push(Command::SetBypass(i, idx, w.bypass));
                    }
                    for &(id, v) in &w.params {
                        if !h.params.contains(&(id, v)) {
                            cmds.push(Command::SetParam(i, idx, id, v));
                        }
                    }
                }
                for c in cmds {
                    self.send(c);
                }
                have.slots = want;
                have.instrument = want_instr;
                continue;
            }
            // Kette neu aufbauen. Vorher die aktuellen Zustände der laufenden Plugins
            // sichern, damit Umbauen (Insert hinzufügen, umsortieren) nichts verliert.
            let mut old: Vec<(String, String)> = Vec::new();
            for (slot, h) in have.slots.iter().zip(&have.handles) {
                if let Some(state) = h.as_ref().and_then(PluginHandle::save_state) {
                    old.push((slot.plugin_id.clone(), B64.encode(state)));
                }
            }
            if let (Some(slot), Some(h)) = (&have.instrument, &have.instrument_handle)
                && let Some(state) = h.save_state()
            {
                old.push((slot.plugin_id.clone(), B64.encode(state)));
            }
            // Griffe freigeben, bevor die alten Instanzen zerstört werden.
            have.handles.clear();
            have.instrument_handle = None;
            let mut want = want;
            let mut want_instr = want_instr;
            for slot in want_instr.iter_mut().chain(want.iter_mut()) {
                if let Some(k) = old.iter().position(|(id, _)| *id == slot.plugin_id) {
                    slot.state = Some(old.remove(k).1);
                }
            }
            {
                let t = &mut self.project.tracks[i];
                for (dst, src) in t.inserts.iter_mut().zip(&want) {
                    dst.state.clone_from(&src.state);
                }
                if let (Some(dst), Some(src)) = (t.instrument.as_mut(), want_instr.as_ref()) {
                    dst.state.clone_from(&src.state);
                }
            }
            let mut chain = InsertChain::default();
            let mut handles = Vec::with_capacity(want.len());
            let mut instrument_handle = None;
            let mut instrument_ok = false;
            if let Some(slot) = &want_instr {
                match self.instantiate(slot) {
                    Ok(inst) => {
                        instrument_handle = Some(inst.handle());
                        chain.instrument = Some(Insert {
                            instance: inst,
                            bypass: slot.bypass,
                            model: INSTRUMENT,
                        });
                        instrument_ok = true;
                    }
                    Err(e) => missing.push(e),
                }
            }
            let mut map = Vec::with_capacity(want.len());
            for slot in &want {
                match self.instantiate(slot) {
                    Ok(inst) => {
                        handles.push(Some(inst.handle()));
                        map.push(Some(chain.slots.len()));
                        chain.slots.push(Insert {
                            instance: inst,
                            bypass: slot.bypass,
                            model: map.len() - 1,
                        });
                    }
                    Err(e) => {
                        missing.push(e);
                        handles.push(None);
                        map.push(None);
                    }
                }
            }
            let chain =
                (!chain.slots.is_empty() || chain.instrument.is_some()).then(|| Box::new(chain));
            let offline = match (&mut self.transport, chain) {
                (Some(t), Some(c)) => match t.try_send(Command::SetInserts(i, Some(c))) {
                    Ok(()) => None,
                    Err(Command::SetInserts(_, c)) => c,
                    Err(_) => None,
                },
                (Some(t), None) => {
                    t.send(Command::SetInserts(i, None));
                    None
                }
                (None, c) => c,
            };
            *have = SentChain {
                _offline: offline,
                handles,
                instrument_handle,
                slots: want,
                map,
                instrument: want_instr,
                instrument_ok,
            };
        }
        self.plugins.sent = sent;
        if !missing.is_empty() {
            self.message = Some(format!("Plugins: {}", missing.join("; ")));
        }
    }

    /// Menü der installierten Instrumente, nach Hersteller. Gibt die Auswahl zurück
    /// (`Some(None)`: Spur ohne Instrument).
    pub(crate) fn instrument_menu(&mut self, ui: &mut Ui) -> Option<Option<PluginInfo>> {
        let instruments: Vec<PluginInfo> = self
            .plugin_list()
            .iter()
            .filter(|p| p.is_instrument())
            .cloned()
            .collect();
        let mut chosen = None;
        if ui.button("Ohne Instrument").clicked() {
            chosen = Some(None);
        }
        if instruments.is_empty() {
            ui.weak("Keine CLAP-Instrumente installiert");
        }
        ui.separator();
        egui::ScrollArea::vertical()
            .max_height(480.0)
            .show(ui, |ui| {
                for p in instruments {
                    let label = if p.vendor.is_empty() {
                        p.name.clone()
                    } else {
                        format!("{} – {}", p.name, p.vendor)
                    };
                    if ui.button(label).clicked() {
                        chosen = Some(Some(p));
                    }
                }
            });
        if chosen.is_some() {
            ui.close();
        }
        chosen
    }

    pub(crate) fn add_audio_track(&mut self) {
        let before = self.project.clone();
        let n = self
            .project
            .tracks
            .iter()
            .filter(|t| t.kind == studiocraft_project::TrackKind::Audio)
            .count();
        let t = self.project.add_track(
            studiocraft_project::TrackKind::Audio,
            format!("Audio {:02}", n + 1),
        );
        self.selected_track = Some(t);
        self.checkpoint(before);
    }

    pub(crate) fn add_instrument_track(&mut self, info: Option<PluginInfo>) {
        let before = self.project.clone();
        let name = info
            .as_ref()
            .map_or("Instrument".to_owned(), |p| p.name.clone());
        let t = self
            .project
            .add_track(studiocraft_project::TrackKind::Instrument, name);
        self.project.tracks[t].instrument = info.map(|p| InsertSlot {
            plugin_id: p.id,
            name: p.name,
            bypass: false,
            params: Vec::new(),
            state: None,
        });
        self.selected_track = Some(t);
        self.checkpoint(before);
    }

    /// Instrument-Bereich des Inspectors: Plugin wählen, ein/aus, Parameter.
    pub(crate) fn instrument_ui(&mut self, ui: &mut Ui, track: usize) {
        match self.project.tracks[track].instrument.clone() {
            Some(slot) => {
                ui.horizontal(|ui| {
                    let mut on = !slot.bypass;
                    if ui
                        .checkbox(&mut on, slot.name.clone())
                        .on_hover_text("Ein/Aus")
                        .changed()
                    {
                        let before = self.project.clone();
                        if let Some(s) = self.project.tracks[track].instrument.as_mut() {
                            s.bypass = !on;
                        }
                        self.checkpoint(before);
                    }
                });
                self.window_button(ui, track, INSTRUMENT, &slot.name);
                ui.menu_button("Instrument wechseln", |ui| {
                    if let Some(choice) = self.instrument_menu(ui) {
                        self.set_instrument(track, choice);
                    }
                });
                self.params_ui(ui, track, INSTRUMENT);
            }
            None => {
                ui.menu_button("Instrument wählen", |ui| {
                    if let Some(choice) = self.instrument_menu(ui) {
                        self.set_instrument(track, choice);
                    }
                });
            }
        }
    }

    fn set_instrument(&mut self, track: usize, info: Option<PluginInfo>) {
        let before = self.project.clone();
        self.project.tracks[track].instrument = info.map(|p| InsertSlot {
            plugin_id: p.id,
            name: p.name,
            bypass: false,
            params: Vec::new(),
            state: None,
        });
        self.checkpoint(before);
    }

    fn handle_mut(&mut self, track: usize, k: usize) -> Option<&mut PluginHandle> {
        let sent = self.plugins.sent.get_mut(track)?;
        if k == INSTRUMENT {
            sent.instrument_handle.as_mut()
        } else {
            sent.handles.get_mut(k)?.as_mut()
        }
    }

    /// Knopf „Fenster“: öffnet/schließt das eigene Fenster des Plugins.
    fn window_button(&mut self, ui: &mut Ui, track: usize, k: usize, name: &str) {
        let title = format!("{} – {}", self.project.tracks[track].name, name);
        let Some(h) = self.handle_mut(track, k) else {
            return;
        };
        if !h.has_gui() {
            return;
        }
        let open = h.gui_open();
        if ui
            .selectable_label(open, "Fenster")
            .on_hover_text("Eigenes Fenster des Plugins")
            .clicked()
        {
            if open {
                h.close_gui();
            } else if let Err(e) = h.open_gui(&title) {
                self.message = Some(format!("{name}: {e}"));
            }
        }
    }

    /// Im UI-Takt: offene Plugin-Fenster bedienen (Timer, Ereignisse) und
    /// Zustandsänderungen einsammeln. `true`, solange Fenster offen sind.
    pub(crate) fn idle_plugins(&mut self) -> bool {
        let mut open = false;
        let mut dirty = false;
        for sent in &mut self.plugins.sent {
            for h in sent
                .handles
                .iter_mut()
                .flatten()
                .chain(sent.instrument_handle.iter_mut())
            {
                open |= h.idle();
                dirty |= h.take_dirty();
            }
        }
        if dirty {
            self.plugins.changed = true;
        }
        open
    }

    /// Aktuelle Zustände aller laufenden Plugins ins Projekt übernehmen (vor dem Speichern).
    pub(crate) fn capture_plugin_states(&mut self) {
        for (i, sent) in self.plugins.sent.iter().enumerate() {
            let Some(track) = self.project.tracks.get_mut(i) else {
                continue;
            };
            for (k, h) in sent.handles.iter().enumerate() {
                if let (Some(h), Some(slot)) = (h, track.inserts.get_mut(k))
                    && let Some(state) = h.save_state()
                {
                    slot.state = Some(B64.encode(state));
                }
            }
            if let (Some(h), Some(slot)) = (&sent.instrument_handle, track.instrument.as_mut())
                && let Some(state) = h.save_state()
            {
                slot.state = Some(B64.encode(state));
            }
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
                self.window_button(ui, track, k, &slot.name);
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

    /// Hängt `p` als neuen Insert an die Spur `track` (ein Undo-Schritt).
    pub(crate) fn add_insert(&mut self, track: usize, p: &PluginInfo) {
        let before = self.project.clone();
        let Some(t) = self.project.tracks.get_mut(track) else {
            return;
        };
        t.inserts.push(InsertSlot {
            plugin_id: p.id.clone(),
            name: p.name.clone(),
            bypass: false,
            params: Vec::new(),
            state: None,
        });
        self.checkpoint(before);
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
                                self.add_insert(track, &p);
                                ui.close();
                            }
                        }
                    });
            });
        }
    }

    /// Allgemeine Parameteransicht (Regler je Parameter). Eigene Plugin-Fenster folgen.
    /// `k` ist der Insert-Slot oder [`INSTRUMENT`].
    pub(crate) fn params_ui(&mut self, ui: &mut Ui, track: usize, k: usize) {
        let Some(slot) = self.slot(track, k) else {
            return;
        };
        let Some(infos) = self.plugins.params.get(&slot.plugin_id).cloned() else {
            return;
        };
        egui::CollapsingHeader::new(format!("Parameter ({})", infos.len()))
            .id_salt(("insert-params", track, k))
            .show(ui, |ui| {
                // Suchfeld; bei großen Plugins (Sampler: tausende Parameter) unverzichtbar.
                let filter_id = egui::Id::unique(("param-filter", track, k));
                let mut filter: String = ui.data(|d| d.get_temp(filter_id)).unwrap_or_default();
                ui.horizontal(|ui| {
                    let l = ui.label("Suche");
                    if ui
                        .text_edit_singleline(&mut filter)
                        .labelled_by(l.id)
                        .changed()
                    {
                        ui.data_mut(|d| d.insert_temp(filter_id, filter.clone()));
                    }
                });
                let needle = filter.to_lowercase();
                let shown: Vec<&ParamInfo> = infos
                    .iter()
                    .filter(|i| needle.is_empty() || i.name.to_lowercase().contains(&needle))
                    .collect();
                // Nur sichtbare Zeilen bauen.
                let row_h = ui.spacing().interact_size.y + ui.spacing().item_spacing.y;
                egui::ScrollArea::vertical()
                    .id_salt(("insert-params-scroll", track, k))
                    .max_height(280.0)
                    .show_rows(ui, row_h, shown.len(), |ui, rows| {
                        for info in &shown[rows] {
                            let Some(slot) = self.slot(track, k) else {
                                return;
                            };
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
                                if let Some(s) = self.slot_mut(track, k) {
                                    s.set_param(info.id, v);
                                }
                                if let Some(idx) = self.engine_index(track, k) {
                                    self.send(Command::SetParam(track, idx, info.id, v));
                                    let sent = self.plugins.sent.get_mut(track);
                                    let s = sent.and_then(|s| {
                                        if k == INSTRUMENT {
                                            s.instrument.as_mut()
                                        } else {
                                            s.slots.get_mut(k)
                                        }
                                    });
                                    if let Some(s) = s {
                                        s.set_param(info.id, v);
                                    }
                                }
                            }
                        }
                    });
            });
    }
}
