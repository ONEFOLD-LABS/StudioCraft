//! Ausgang und Sends im Inspector (Spec „Mixer“: Sends 1–8, Gruppen, FX-Kanäle).

use crate::i18n::tr;
use egui::Ui;
use studiocraft_project::{
    MAX_DB, MAX_EXTRA_OUTPUTS, MIN_DB, OutTarget, Project, SendSlot, TrackId, TrackKind,
};

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

    pub(crate) fn track_name(&self, id: TrackId) -> String {
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
    /// Eingang einer Audio-Spur: ein Eingangsbus der Audio-Anschlüsse.
    pub(crate) fn input_ui(&mut self, ui: &mut Ui, track: usize) {
        let current = self.project.tracks[track].input_bus;
        let label = current
            .and_then(|id| self.project.io.input(id))
            .map_or_else(
                || crate::names::bus(&self.project.tracks[track].input.label()),
                |b| crate::names::bus(&b.name),
            );
        let mut choice = None;
        ui.horizontal(|ui| {
            let l = ui.label(tr("Eingang"));
            egui::ComboBox::from_id_salt(("eingang", track))
                .selected_text(label)
                .show_ui(ui, |ui| {
                    for b in &self.project.io.inputs {
                        let text = format!("{} ({})", crate::names::bus(&b.name), b.ports_label());
                        if ui.selectable_label(current == Some(b.id), text).clicked() {
                            choice = Some(b.id);
                        }
                    }
                    ui.separator();
                    if ui.button(tr("Audio-Anschlüsse …")).clicked() {
                        self.io_dialog = true;
                    }
                })
                .response
                .labelled_by(l.id);
        });
        if let Some(id) = choice.filter(|&id| Some(id) != current) {
            let before = self.project.clone();
            self.project.tracks[track].input_bus = Some(id);
            self.checkpoint(before);
        }
    }

    pub(crate) fn output_ui(&mut self, ui: &mut Ui, track: usize) {
        let me = self.project.tracks[track].id;
        let current = self.project.tracks[track].output;
        let current_bus = self.project.tracks[track].out_bus;
        let main_name = self
            .project
            .io
            .outputs
            .first()
            .map_or(tr("Summe").to_owned(), |b| crate::names::bus(&b.name));
        let label = match (
            current,
            current_bus.and_then(|id| self.project.io.output(id)),
        ) {
            (Some(id), _) => self.track_name(id),
            (None, Some(b)) => crate::names::bus(&b.name),
            (None, None) => main_name.clone(),
        };
        // Wahl: Gruppe, Hauptmix (None, None) oder weiterer Ausgangsbus (None, Some).
        let mut choice: Option<(Option<TrackId>, Option<u32>)> = None;
        ui.horizontal(|ui| {
            let l = ui.label(tr("Ausgang"));
            egui::ComboBox::from_id_salt(("ausgang", track))
                .selected_text(label)
                .show_ui(ui, |ui| {
                    let main = current.is_none() && current_bus.is_none();
                    if ui.selectable_label(main, main_name.as_str()).clicked() {
                        choice = Some((None, None));
                    }
                    for b in self.project.io.outputs.iter().skip(1) {
                        let on = current.is_none() && current_bus == Some(b.id);
                        if ui
                            .selectable_label(on, crate::names::bus(&b.name))
                            .clicked()
                        {
                            choice = Some((None, Some(b.id)));
                        }
                    }
                    for id in self.route_targets(me, true) {
                        if ui
                            .selectable_label(current == Some(id), self.track_name(id))
                            .clicked()
                        {
                            choice = Some((Some(id), None));
                        }
                    }
                })
                .response
                .labelled_by(l.id);
        });
        if let Some(c) = choice.filter(|c| *c != (current, current_bus)) {
            let before = self.project.clone();
            let t = &mut self.project.tracks[track];
            t.output = c.0;
            t.out_bus = c.1;
            // Der neue Hauptausgang steht nicht zusätzlich in den weiteren Ausgängen.
            let extras = std::mem::take(&mut t.extra_outputs);
            t.extra_outputs = extras
                .into_iter()
                .filter(|o| !t.is_main_output(*o))
                .collect();
            self.checkpoint(before);
        }
        self.extra_outputs_ui(ui, track);
    }

    /// Name eines Ausgangsziels.
    fn out_target_name(&self, o: OutTarget) -> String {
        match o {
            OutTarget::Group(id) => self.track_name(id),
            OutTarget::Bus(id) => self
                .project
                .io
                .output(id)
                .map_or_else(|| "?".to_owned(), |b| crate::names::bus(&b.name)),
        }
    }

    /// Weitere Ausgänge der Spur (Spec „Routing und Busse“: direkt auf mehrere Ziele):
    /// Liste mit Entfernen und „+ Weiterer Ausgang“ mit allen Bussen und Gruppen, die noch
    /// frei sind und keine Schleife bilden.
    fn extra_outputs_ui(&mut self, ui: &mut Ui, track: usize) {
        let t = &self.project.tracks[track];
        let me = t.id;
        let extras = t.extra_outputs.clone();
        let mut remove = None;
        for (k, &o) in extras.iter().enumerate() {
            ui.horizontal(|ui| {
                ui.label(crate::i18n::trf(
                    "Auch an: {name}",
                    &[("name", &self.out_target_name(o))],
                ));
                let r = ui
                    .small_button(tr("x"))
                    .on_hover_text(tr("Weiteren Ausgang entfernen"));
                r.widget_info(|| {
                    egui::WidgetInfo::labeled(
                        egui::Role::Button,
                        true,
                        tr("Weiteren Ausgang entfernen"),
                    )
                });
                if r.clicked() {
                    remove = Some(k);
                }
            });
        }
        if let Some(k) = remove {
            let before = self.project.clone();
            self.project.tracks[track].extra_outputs.remove(k);
            self.checkpoint(before);
        }
        let t = &self.project.tracks[track];
        let candidates: Vec<OutTarget> = self
            .project
            .io
            .outputs
            .iter()
            .map(|b| OutTarget::Bus(b.id))
            .chain(
                self.route_targets(me, true)
                    .into_iter()
                    .map(OutTarget::Group),
            )
            .filter(|o| !t.is_main_output(*o) && !t.extra_outputs.contains(o))
            .collect();
        if t.extra_outputs.len() < MAX_EXTRA_OUTPUTS && !candidates.is_empty() {
            ui.menu_button(tr("+ Weiterer Ausgang"), |ui| {
                for o in candidates {
                    if ui.button(self.out_target_name(o)).clicked() {
                        let before = self.project.clone();
                        self.project.tracks[track].extra_outputs.push(o);
                        self.checkpoint(before);
                        ui.close();
                    }
                }
            });
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
                    .on_hover_text(tr("Send ein/aus"));
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
                    .on_hover_text(tr("Ziel des Sends"));
                if let Some(t) = target.filter(|t| *t != send.target) {
                    self.edit_send(track, k, |s| s.target = t);
                }
                let mut pre = send.pre_fader;
                if ui
                    .toggle_value(&mut pre, tr("Pre"))
                    .on_hover_text(tr("Vor dem Fader abgreifen"))
                    .changed()
                {
                    self.edit_send(track, k, |s| s.pre_fader = pre);
                }
                if ui
                    .small_button(tr("x"))
                    .on_hover_text(tr("Send entfernen"))
                    .clicked()
                {
                    remove = Some(k);
                }
            });
            let mut level = send.level_db;
            let r = ui.add(
                egui::Slider::new(&mut level, MIN_DB..=MAX_DB)
                    .text(crate::i18n::trf("Send {nr}", &[("nr", &(k + 1))]))
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
            self.project.tracks[track].remove_send(k);
            self.checkpoint(before);
        }
        let targets = self.route_targets(me, false);
        if sends.len() < MAX_SENDS && !targets.is_empty() {
            ui.menu_button(tr("+ Send hinzufügen"), |ui| {
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
            ui.weak(tr("Kein FX-Kanal oder Gruppe vorhanden (Menü Projekt)"));
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
