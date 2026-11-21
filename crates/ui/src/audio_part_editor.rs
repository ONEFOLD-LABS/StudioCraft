//! Part-Editor für Audio-Parts (Issue #184): Doppelklick auf einen Audio-Part öffnet ihn in
//! der unteren Zone. Je Event im Part eine Zeile mit Anfang und Länge (im Projekt, editierbar),
//! Lautstärke, Stumm und Löschen. Jede Änderung ist ein Rückgängig-Schritt.

use egui::{DragValue, Key, TextEdit, Ui};
use studiocraft_project::{EventId, Position};

use crate::ProjectWindow;
use crate::i18n::{tr, trf};

/// Eine Änderung an einem Event im Part.
#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) enum PartEdit {
    /// Neuer Anfang im Projekt (Ticks).
    Start(u64),
    Length(u64),
    Muted(bool),
    Delete,
}

impl ProjectWindow {
    /// Den Audio-Part `id` im Part-Editor (untere Zone) öffnen.
    pub(crate) fn open_audio_part_editor(&mut self, id: EventId) {
        if self.project.event(id).is_some_and(|e| e.is_audio_part()) {
            self.audio_part = Some(id);
            self.show_lower = true;
            self.lower = crate::LowerZone::AudioPart;
        }
    }

    /// Ändert das Event `child` des Parts `part` (ein Rückgängig-Schritt). Ein Part ohne
    /// Events wird gelöscht.
    pub(crate) fn edit_part_child(&mut self, part: EventId, child: usize, edit: PartEdit) {
        let before = self.project.clone();
        let mut emptied = false;
        if let Some(e) = self.project.event_mut(part) {
            let ps = e.start.0;
            if let Some(c) = e.children.get_mut(child) {
                match edit {
                    PartEdit::Start(t) => c.start = Position(t.saturating_sub(ps)),
                    PartEdit::Length(l) => {
                        c.length = l.max(1);
                        c.fade_in = c.fade_in.min(c.length);
                        c.fade_out = c.fade_out.min(c.length - c.fade_in);
                    }
                    PartEdit::Muted(m) => c.muted = m,
                    PartEdit::Delete => {
                        e.children.remove(child);
                    }
                }
            }
            e.children.sort_by_key(|c| c.start.0);
            emptied = e.children.is_empty();
        }
        if emptied {
            self.project.delete(part);
            self.selection.retain(|&s| s != part);
            self.audio_part = None;
        }
        self.checkpoint(before);
    }
}

/// Textfeld, das seinen Wert bei Enter oder Fokusverlust abgibt (Escape verwirft).
fn text_cell(ui: &mut Ui, key: (usize, u8), value: String, label: &str) -> Option<String> {
    let id = ui.make_persistent_id(("audio-part", key));
    let mut text = ui
        .data(|d| d.get_temp::<String>(id))
        .unwrap_or_else(|| value.clone());
    let r = ui.add_sized(
        [110.0, 20.0],
        TextEdit::singleline(&mut text).id(id.with("feld")),
    );
    r.widget_info(|| egui::WidgetInfo::labeled(egui::Role::TextInput, true, label));
    if r.has_focus() {
        ui.data_mut(|d| d.insert_temp(id, text));
        return None;
    }
    ui.data_mut(|d| d.remove_temp::<String>(id));
    let cancelled = ui.input(|i| i.key_pressed(Key::Escape));
    (r.lost_focus() && !cancelled && text != value).then_some(text)
}

/// Der Part-Editor in der unteren Zone.
pub(crate) fn show(w: &mut ProjectWindow, ui: &mut Ui) {
    // Ohne geöffneten Part: der erste gewählte.
    if w.audio_part
        .is_none_or(|p| w.project.event(p).is_none_or(|e| !e.is_audio_part()))
    {
        w.audio_part = w
            .selection
            .iter()
            .copied()
            .find(|&id| w.project.event(id).is_some_and(|e| e.is_audio_part()));
    }
    let Some(part) = w.audio_part else {
        ui.centered_and_justified(|ui| {
            ui.weak(tr(
                "Einen Audio-Part doppelklicken oder wählen, um seine Events hier zu bearbeiten",
            ))
        });
        return;
    };
    let Some(ev) = w.project.event(part).cloned() else {
        return;
    };
    let tempo = w.project.tempo.clone();
    let clip_name = |c: &studiocraft_project::Event| {
        c.clip
            .and_then(|id| w.project.pool.get(id as usize))
            .map_or_else(|| "?".to_owned(), |p| p.name.clone())
    };
    ui.horizontal(|ui| {
        ui.strong(&ev.name);
        ui.weak(trf("{n} Events", &[("n", &ev.children.len())]));
    });
    ui.separator();
    let mut edits: Vec<(usize, PartEdit)> = Vec::new();
    let mut gains: Vec<(usize, f32)> = Vec::new();
    egui::ScrollArea::vertical()
        .id_salt("audio-part-zeilen")
        .auto_shrink([false, false])
        .show(ui, |ui| {
            egui::Grid::new("audio-part-tabelle")
                .num_columns(6)
                .striped(true)
                .show(ui, |ui| {
                    for h in ["Event", "Anfang", "Länge", "Lautstärke", "Stumm", ""] {
                        ui.strong(if h.is_empty() { "" } else { tr(h) });
                    }
                    ui.end_row();
                    for (k, c) in ev.children.iter().enumerate() {
                        ui.label(clip_name(c));
                        let abs = Position(ev.start.0.saturating_add(c.start.0));
                        if let Some(t) =
                            text_cell(ui, (k, 0), tempo.format_position(abs), tr("Anfang"))
                            && let Some(p) = tempo.parse_position(&t)
                        {
                            edits.push((k, PartEdit::Start(p.0)));
                        }
                        if let Some(t) =
                            text_cell(ui, (k, 1), tempo.format_length(c.length), tr("Länge"))
                            && let Some(l) = tempo.parse_length(&t)
                        {
                            edits.push((k, PartEdit::Length(l)));
                        }
                        let mut db = c.gain_db;
                        let r = ui.add(
                            DragValue::new(&mut db)
                                .range(-60.0..=studiocraft_project::EVENT_MAX_DB)
                                .speed(0.1)
                                .suffix(" dB"),
                        );
                        r.widget_info(|| {
                            egui::WidgetInfo::labeled(
                                egui::Role::SpinButton,
                                true,
                                tr("Lautstärke"),
                            )
                        });
                        // Direkt ins Projekt: ein ganzer Zug wird über das Panel-Undo der
                        // unteren Zone ein Schritt (panel_undo.rs).
                        if r.changed() {
                            gains.push((k, db));
                        }
                        let mut muted = c.muted;
                        if ui.checkbox(&mut muted, tr("Stumm")).changed() {
                            edits.push((k, PartEdit::Muted(muted)));
                        }
                        if ui.button(tr("Löschen")).clicked() {
                            edits.push((k, PartEdit::Delete));
                        }
                        ui.end_row();
                    }
                });
        });
    if let Some(e) = w.project.event_mut(part) {
        for (k, db) in gains {
            if let Some(c) = e.children.get_mut(k)
                && db.is_finite()
            {
                c.gain_db = db.clamp(-60.0, studiocraft_project::EVENT_MAX_DB);
            }
        }
    }
    // Löschen zuletzt und von hinten, damit die Zeilennummern stimmen.
    edits.sort_by_key(|(k, e)| (matches!(e, PartEdit::Delete), std::cmp::Reverse(*k)));
    for (k, e) in edits {
        w.edit_part_child(part, k, e);
    }
}
