//! Ausgang und Sends im Inspector (Spec „Mixer“: Sends 1–8, Gruppen, FX-Kanäle).

use egui::Ui;
use studiocraft_project::{MAX_DB, MIN_DB, Project, SendSlot, TrackId, TrackKind};

use crate::{ProjectWindow, mixer::db_label};

/// So viele Sends hat ein Kanal.
const MAX_SENDS: usize = 8;

impl ProjectWindow {
    pub(crate) fn add_bus_track(&mut self, kind: TrackKind) {
        let before = self.project.clone();
        let n = self
            .project
            .tracks
            .iter()
            .filter(|t| t.kind == kind)
            .count();
        let name = match kind {
            TrackKind::Fx => format!("FX {:02}", n + 1),
            _ => format!("Gruppe {:02}", n + 1),
        };
        let t = self.project.add_track(kind, name);
        self.selected_track = Some(t);
        self.checkpoint(before);
    }

    fn track_name(&self, id: TrackId) -> String {
        self.project
            .track_index(id)
            .map_or("?".into(), |i| self.project.tracks[i].name.clone())
    }

    /// Busse, auf die `from` leiten darf (ohne Schleife). `groups_only` für den Ausgang.
    fn route_targets(&self, from: TrackId, groups_only: bool) -> Vec<TrackId> {
        self.project
            .tracks
            .iter()
            .filter(|t| {
                if groups_only {
                    t.kind == TrackKind::Group
                } else {
                    Project::is_bus(t.kind)
                }
            })
            .map(|t| t.id)
            .filter(|&id| self.project.can_route(from, id))
            .collect()
    }

    /// Ausgang der Spur: Summe oder eine Gruppe.
    pub(crate) fn output_ui(&mut self, ui: &mut Ui, track: usize) {
        let me = self.project.tracks[track].id;
        let current = self.project.tracks[track].output;
        let label = current.map_or("Summe".into(), |id| self.track_name(id));
        let mut choice = None;
        ui.horizontal(|ui| {
            let l = ui.label("Ausgang");
            egui::ComboBox::from_id_salt(("ausgang", track))
                .selected_text(label)
                .show_ui(ui, |ui| {
                    if ui.selectable_label(current.is_none(), "Summe").clicked() {
                        choice = Some(None);
                    }
                    for id in self.route_targets(me, true) {
                        if ui
                            .selectable_label(current == Some(id), self.track_name(id))
                            .clicked()
                        {
                            choice = Some(Some(id));
                        }
                    }
                })
                .response
                .labelled_by(l.id);
        });
        if let Some(c) = choice.filter(|c| *c != current) {
            let before = self.project.clone();
            self.project.tracks[track].output = c;
            self.checkpoint(before);
        }
    }

    /// Sends der Spur.
    pub(crate) fn sends_ui(&mut self, ui: &mut Ui, track: usize) {
        let me = self.project.tracks[track].id;
        let sends = self.project.tracks[track].sends.clone();
        let mut remove = None;
        for (k, send) in sends.iter().enumerate() {
            ui.horizontal(|ui| {
                let mut on = send.on;
                let cb = ui
                    .checkbox(&mut on, format!("{}", k + 1))
                    .on_hover_text("Send ein/aus");
                if cb.changed() {
                    self.edit_send(track, k, |s| s.on = on);
                }
                // Ziel wählen (ohne Schleifen).
                let mut target = None;
                egui::ComboBox::from_id_salt(("send-ziel", track, k))
                    .width(90.0)
                    .selected_text(self.track_name(send.target))
                    .show_ui(ui, |ui| {
                        let mut targets = self.route_targets(me, false);
                        if !targets.contains(&send.target) {
                            targets.insert(0, send.target);
                        }
                        for id in targets {
                            if ui
                                .selectable_label(id == send.target, self.track_name(id))
                                .clicked()
                            {
                                target = Some(id);
                            }
                        }
                    })
                    .response
                    .labelled_by(cb.id)
                    .on_hover_text("Ziel des Sends");
                if let Some(t) = target.filter(|t| *t != send.target) {
                    self.edit_send(track, k, |s| s.target = t);
                }
                let mut pre = send.pre_fader;
                if ui
                    .toggle_value(&mut pre, "Pre")
                    .on_hover_text("Vor dem Fader abgreifen")
                    .changed()
                {
                    self.edit_send(track, k, |s| s.pre_fader = pre);
                }
                if ui
                    .small_button("x")
                    .on_hover_text("Send entfernen")
                    .clicked()
                {
                    remove = Some(k);
                }
            });
            let mut level = send.level_db;
            let r = ui.add(
                egui::Slider::new(&mut level, MIN_DB..=MAX_DB)
                    .text(format!("Send {}", k + 1))
                    .custom_formatter(|v, _| db_label(v as f32)),
            );
            if r.changed() {
                // Pegel wie Fader: wirkt sofort, ohne eigenen Undo-Schritt.
                self.project.tracks[track].sends[k].level_db = level;
                self.dirty = true;
            }
        }
        if let Some(k) = remove {
            let before = self.project.clone();
            self.project.tracks[track].sends.remove(k);
            self.checkpoint(before);
        }
        let targets = self.route_targets(me, false);
        if sends.len() < MAX_SENDS && !targets.is_empty() {
            ui.menu_button("+ Send hinzufügen", |ui| {
                for id in targets {
                    if ui.button(self.track_name(id)).clicked() {
                        let before = self.project.clone();
                        self.project.tracks[track].sends.push(SendSlot {
                            target: id,
                            level_db: 0.0,
                            pre_fader: false,
                            on: true,
                        });
                        self.checkpoint(before);
                        ui.close();
                    }
                }
            });
        } else if targets.is_empty() && sends.is_empty() {
            ui.weak("Kein FX-Kanal oder Gruppe vorhanden (Menü Projekt)");
        }
    }

    fn edit_send(&mut self, track: usize, k: usize, f: impl FnOnce(&mut SendSlot)) {
        let before = self.project.clone();
        if let Some(s) = self.project.tracks[track].sends.get_mut(k) {
            f(s);
        }
        self.checkpoint(before);
    }
}
