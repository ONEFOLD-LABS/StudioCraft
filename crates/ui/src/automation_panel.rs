//! Abschnitt „Automation“ im Inspector (Spec „Automation“, Issue #34): Automationsspuren der
//! Spur zeigen, stummschalten, entfernen und für weitere Parameter anlegen, auch für die
//! Parameter der Inserts und des Instruments.

use egui::Ui;
use studiocraft_project::AutoParam;

use crate::ProjectWindow;

impl ProjectWindow {
    pub(crate) fn automation_ui(&mut self, ui: &mut Ui, idx: usize) {
        let Some(track) = self.project.tracks.get(idx) else {
            return;
        };
        if !track.read {
            ui.weak("R ist aus: Kurven werden nicht abgespielt.");
        }
        let mut remove = None;
        let mut changed = false;
        for (i, lane) in track.automation.clone().iter().enumerate() {
            ui.horizontal(|ui| {
                let mut visible = lane.visible;
                if ui
                    .checkbox(&mut visible, self.param_name(idx, lane.param))
                    .on_hover_text("In der Event-Anzeige zeigen")
                    .changed()
                {
                    self.project.tracks[idx].automation[i].visible = visible;
                    changed = true;
                }
                let mut muted = lane.muted;
                if crate::icons::state_toggle(
                    ui,
                    &mut muted,
                    "M",
                    crate::theme::tokens(ui.ctx()).mute,
                )
                .on_hover_text("Automationsspur stumm")
                .changed()
                {
                    self.project.tracks[idx].automation[i].muted = muted;
                    changed = true;
                }
                if ui.small_button("Entfernen").clicked() {
                    remove = Some(i);
                }
            });
        }
        if let Some(i) = remove {
            self.project.tracks[idx].automation.remove(i);
            changed = true;
        }
        let mut add = None;
        ui.menu_button("+ Parameter", |ui| {
            for p in [AutoParam::Volume, AutoParam::Pan, AutoParam::Mute] {
                if ui.button(p.label()).clicked() {
                    add = Some(p);
                    ui.close();
                }
            }
            for (slot, name, plugin_id) in self.automatable_plugins(idx) {
                let Some(params) = self.plugins.params.get(&plugin_id).cloned() else {
                    continue;
                };
                ui.menu_button(name, |ui| {
                    egui::ScrollArea::vertical()
                        .max_height(300.0)
                        .show(ui, |ui| {
                            for p in params {
                                if ui.button(&p.name).clicked() {
                                    add = Some(AutoParam::Plugin { slot, id: p.id });
                                    ui.close();
                                }
                            }
                        });
                });
            }
        });
        if let Some(p) = add {
            let before = self.project.clone();
            let lane = self.project.tracks[idx].automation_lane_mut(p);
            lane.visible = true;
            self.checkpoint(before);
        } else if changed {
            self.dirty = true;
        }
    }

    /// Instrument und Inserts einer Spur: (Slot im Modell, Name, Plugin-Id).
    fn automatable_plugins(&self, idx: usize) -> Vec<(u32, String, String)> {
        let Some(track) = self.project.tracks.get(idx) else {
            return Vec::new();
        };
        let mut out = Vec::new();
        if let Some(i) = &track.instrument {
            out.push((u32::MAX, i.name.clone(), i.plugin_id.clone()));
        }
        for (k, s) in track.inserts.iter().enumerate() {
            if let Ok(k) = u32::try_from(k) {
                out.push((k, s.name.clone(), s.plugin_id.clone()));
            }
        }
        out
    }

    /// Name eines Parameters, bei Plugins aus der Parameterbeschreibung.
    pub(crate) fn param_name(&self, idx: usize, param: AutoParam) -> String {
        let AutoParam::Plugin { slot, id } = param else {
            return param.label();
        };
        let Some(track) = self.project.tracks.get(idx) else {
            return param.label();
        };
        let ins = if slot == u32::MAX {
            track.instrument.as_ref()
        } else {
            track.inserts.get(slot as usize)
        };
        ins.and_then(|i| {
            let p = self
                .plugins
                .params
                .get(&i.plugin_id)?
                .iter()
                .find(|p| p.id == id)?;
            Some(format!("{} · {}", i.name, p.name))
        })
        .unwrap_or_else(|| param.label())
    }
}
