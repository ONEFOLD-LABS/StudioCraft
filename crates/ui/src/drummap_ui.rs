//! Fenster „Drum-Map bearbeiten“ (Issue #161): Zeilen umbenennen, Tonhöhe und Ausgangsnote
//! setzen, Länge und Raster je Zeile, Zeilen hinzufügen, löschen und umsortieren; die Map als
//! Datei speichern und laden. Die Map einer anderen Spur lässt sich im Inspector übernehmen.
//!
//! Rückgängig: Eine Geste im Fenster (Klick, Ziehen, eine Texteingabe) ist ein Schritt.

use egui::{DragValue, Key, RichText, TextEdit, Ui};
use studiocraft_project::{DRUM_MAP_EXTENSION, DrumMap, Project};

use crate::ProjectWindow;
use crate::i18n::{tr, trf};

/// Notenwerte für Länge und Raster einer Zeile: (Ticks, Beschriftung).
const VALUES: [(u64, &str); 7] = [
    (960, "1/4"),
    (480, "1/8"),
    (240, "1/16"),
    (120, "1/32"),
    (60, "1/64"),
    (320, "1/8T"),
    (160, "1/16T"),
];

/// Zustand des Fensters.
#[derive(Debug, Default)]
pub(crate) struct DrumMapEditor {
    /// Spur, deren Map bearbeitet wird.
    pub track: Option<usize>,
    /// Projektstand zu Beginn der laufenden Geste.
    before: Option<Project>,
}

fn value_label(ticks: u64) -> String {
    VALUES
        .iter()
        .find(|(t, _)| *t == ticks)
        .map_or_else(|| format!("{ticks} Ticks"), |(_, l)| (*l).to_owned())
}

/// Textfeld, das beim Verlassen oder mit Enter übernimmt (Escape verwirft).
fn text_cell(
    ui: &mut Ui,
    id: egui::Id,
    value: &str,
    width: f32,
    label: egui::Id,
) -> Option<String> {
    let mut text = ui
        .data(|d| d.get_temp::<String>(id))
        .unwrap_or_else(|| value.to_owned());
    let r = ui
        .add(
            TextEdit::singleline(&mut text)
                .id(id.with("feld"))
                .desired_width(width),
        )
        .labelled_by(label);
    if r.has_focus() {
        ui.data_mut(|d| d.insert_temp(id, text));
        return None;
    }
    ui.data_mut(|d| d.remove_temp::<String>(id));
    let cancelled = ui.input(|i| i.key_pressed(Key::Escape));
    (r.lost_focus() && !cancelled && text != value).then_some(text)
}

/// Eine Änderung an der Map.
enum Change {
    Name(String),
    RowName(usize, String),
    Pitch(usize, u8),
    Output(usize, Option<u8>),
    Length(usize, u64),
    Quantize(usize, u64),
    Move(usize, bool),
    Remove(usize),
    Add,
    Reset,
}

impl ProjectWindow {
    /// Öffnet das Fenster für die Map von `track`.
    pub(crate) fn open_drum_map_editor(&mut self, track: usize) {
        self.drum_map_editor.track = Some(track);
    }

    fn apply_drum_change(&mut self, track: usize, change: Change) {
        let Some(map) = self
            .project
            .tracks
            .get_mut(track)
            .and_then(|t| t.drum_map.as_mut())
        else {
            return;
        };
        match change {
            Change::Name(n) => map.name = n,
            Change::RowName(i, n) => {
                if let Some(r) = map.rows.get_mut(i) {
                    r.name = n;
                }
            }
            Change::Pitch(i, p) => {
                map.set_pitch(i, p);
            }
            Change::Output(i, o) => {
                if let Some(r) = map.rows.get_mut(i) {
                    r.output = o.filter(|&o| o != r.pitch);
                }
            }
            Change::Length(i, l) => {
                if let Some(r) = map.rows.get_mut(i) {
                    r.length = l.max(1);
                }
            }
            Change::Quantize(i, q) => {
                if let Some(r) = map.rows.get_mut(i) {
                    r.quantize = q;
                }
            }
            Change::Move(i, up) => {
                map.move_row(i, up);
            }
            Change::Remove(i) => {
                if i < map.rows.len() {
                    map.rows.remove(i);
                }
            }
            Change::Add => {
                let near = map.rows.first().map_or(36, |r| r.pitch.saturating_add(1));
                map.add_row(near.min(127));
            }
            Change::Reset => *map = DrumMap::general_midi(),
        }
        map.sanitize();
        self.dirty = true;
    }

    /// Speichert die Map von `track` als Datei.
    fn save_drum_map(&mut self, track: usize) -> Result<(), String> {
        let Some(map) = self
            .project
            .tracks
            .get(track)
            .and_then(|t| t.drum_map.clone())
        else {
            return Ok(());
        };
        let Some(path) = crate::menu::dialog()
            .add_filter(tr("Drum-Map"), &[DRUM_MAP_EXTENSION])
            .set_file_name(format!(
                "{}.{DRUM_MAP_EXTENSION}",
                crate::session::sanitize(&map.name)
            ))
            .save_file()
        else {
            return Ok(());
        };
        let text = map.to_file_text()?;
        std::fs::write(&path, text).map_err(|e| format!("{}: {e}", path.display()))
    }

    /// Lädt eine Map aus einer Datei für `track` (ein Rückgängig-Schritt).
    fn load_drum_map(&mut self, track: usize) -> Result<(), String> {
        let Some(path) = crate::menu::dialog()
            .add_filter(tr("Drum-Map"), &[DRUM_MAP_EXTENSION])
            .pick_file()
        else {
            return Ok(());
        };
        let too_big = std::fs::metadata(&path)
            .is_ok_and(|m| m.len() > studiocraft_project::drummap::MAX_DRUM_MAP_FILE as u64);
        if too_big {
            return Err(format!("{}: {}", path.display(), tr("Datei zu groß")));
        }
        let text =
            std::fs::read_to_string(&path).map_err(|e| format!("{}: {e}", path.display()))?;
        let map = DrumMap::from_file_text(&text).map_err(|e| format!("{}: {e}", path.display()))?;
        self.set_drum_map(track, Some(map));
        Ok(())
    }

    /// Setzt die Map einer Spur (ein Rückgängig-Schritt).
    pub(crate) fn set_drum_map(&mut self, track: usize, map: Option<DrumMap>) {
        let before = self.project.clone();
        if let Some(t) = self.project.tracks.get_mut(track) {
            t.drum_map = map;
        }
        self.checkpoint(before);
    }

    /// Zeichnet das Fenster, solange es offen ist.
    pub(crate) fn drum_map_window(&mut self, ctx: &egui::Context) {
        let Some(track) = self.drum_map_editor.track else {
            return;
        };
        let Some(map) = self
            .project
            .tracks
            .get(track)
            .and_then(|t| t.drum_map.clone())
        else {
            self.drum_map_editor.track = None;
            return;
        };
        let track_name = self.project.tracks[track].name.clone();
        let mut open = true;
        let mut changes = Vec::new();
        let (mut save, mut load) = (false, false);
        self.dialogs
            .escape_closes(ctx, egui::Id::unique("drum-map-editor"), &mut open);
        let r = egui::Window::new(trf("Drum-Map: {spur}", &[("spur", &track_name)]))
            .id(egui::Id::unique("drum-map-editor"))
            .open(&mut open)
            .default_size([640.0, 420.0])
            .show(ctx, |ui| {
                ui.horizontal(|ui| {
                    let l = ui.label(tr("Name"));
                    if let Some(n) =
                        text_cell(ui, ui.scope_id().with("name"), &map.name, 140.0, l.id)
                    {
                        changes.push(Change::Name(n));
                    }
                    if ui.button(tr("Zeile hinzufügen")).clicked() {
                        changes.push(Change::Add);
                    }
                    if ui.button(tr("Auf GM zurücksetzen")).clicked() {
                        changes.push(Change::Reset);
                    }
                    if ui.button(tr("Speichern …")).clicked() {
                        save = true;
                    }
                    if ui.button(tr("Laden …")).clicked() {
                        load = true;
                    }
                });
                ui.separator();
                let headers = [
                    tr("Instrument"),
                    tr("Note"),
                    tr("Ausgang"),
                    tr("Länge"),
                    tr("Raster"),
                ];
                let widths = [150.0, 70.0, 70.0, 70.0, 90.0];
                let mut ids = [ui.scope_id(); 5];
                ui.horizontal(|ui| {
                    for (k, h) in headers.iter().enumerate() {
                        ids[k] = ui
                            .add_sized(
                                [widths[k], 18.0],
                                egui::Label::new(RichText::new(*h).strong()),
                            )
                            .id;
                    }
                });
                let count = map.rows.len();
                egui::ScrollArea::vertical()
                    .auto_shrink([false, false])
                    .show_rows(ui, 24.0, count, |ui, range| {
                        for i in range {
                            let row = &map.rows[i];
                            ui.horizontal(|ui| {
                                let id = ui.scope_id().with(("zeile", row.pitch));
                                if let Some(n) = text_cell(
                                    ui,
                                    id,
                                    &crate::names::drum_row(row.pitch, &row.name),
                                    widths[0] - 8.0,
                                    ids[0],
                                ) {
                                    changes.push(Change::RowName(i, n));
                                }
                                let note = |v: f64, _| crate::keyeditor::note_name(v as u8);
                                let mut p = row.pitch;
                                if ui
                                    .add_sized(
                                        [widths[1], 20.0],
                                        DragValue::new(&mut p)
                                            .range(0..=127)
                                            .custom_formatter(note),
                                    )
                                    .labelled_by(ids[1])
                                    .changed()
                                {
                                    changes.push(Change::Pitch(i, p));
                                }
                                let mut o = row.output.unwrap_or(row.pitch);
                                if ui
                                    .add_sized(
                                        [widths[2], 20.0],
                                        DragValue::new(&mut o)
                                            .range(0..=127)
                                            .custom_formatter(note),
                                    )
                                    .labelled_by(ids[2])
                                    .on_hover_text(tr(
                                        "Gespielte Tonhöhe, wenn sie von der Zeile abweicht",
                                    ))
                                    .changed()
                                {
                                    changes.push(Change::Output(i, Some(o)));
                                }
                                egui::ComboBox::from_id_salt(("länge", row.pitch))
                                    .width(widths[3] - 8.0)
                                    .selected_text(value_label(row.length))
                                    .show_ui(ui, |ui| {
                                        for (t, l) in VALUES {
                                            if ui.selectable_label(row.length == t, l).clicked() {
                                                changes.push(Change::Length(i, t));
                                            }
                                        }
                                    })
                                    .response
                                    .labelled_by(ids[3]);
                                egui::ComboBox::from_id_salt(("raster", row.pitch))
                                    .width(widths[4] - 8.0)
                                    .selected_text(if row.quantize == 0 {
                                        tr("Werkzeugzeile").to_owned()
                                    } else {
                                        value_label(row.quantize)
                                    })
                                    .show_ui(ui, |ui| {
                                        if ui
                                            .selectable_label(
                                                row.quantize == 0,
                                                tr("Werkzeugzeile"),
                                            )
                                            .clicked()
                                        {
                                            changes.push(Change::Quantize(i, 0));
                                        }
                                        for (t, l) in VALUES {
                                            if ui.selectable_label(row.quantize == t, l).clicked() {
                                                changes.push(Change::Quantize(i, t));
                                            }
                                        }
                                    })
                                    .response
                                    .labelled_by(ids[4]);
                                if ui
                                    .add_enabled(i > 0, egui::Button::new(tr("Hoch")))
                                    .on_hover_text(tr("Nach oben"))
                                    .clicked()
                                {
                                    changes.push(Change::Move(i, true));
                                }
                                if ui
                                    .add_enabled(i + 1 < count, egui::Button::new(tr("Runter")))
                                    .on_hover_text(tr("Nach unten"))
                                    .clicked()
                                {
                                    changes.push(Change::Move(i, false));
                                }
                                if ui
                                    .button(tr("Löschen"))
                                    .on_hover_text(tr("Zeile löschen"))
                                    .clicked()
                                {
                                    changes.push(Change::Remove(i));
                                }
                            });
                        }
                    });
            });
        // Rückgängig: eine Geste (Drücken bis Loslassen, Texteingabe bis zum Verlassen) ist ein
        // Schritt.
        let rect = r.map(|r| r.response.rect);
        let pressed_here = ctx.input(|i| {
            i.pointer.any_pressed()
                && i.pointer
                    .press_origin()
                    .is_some_and(|p| rect.is_some_and(|r| r.contains(p)))
        });
        if (pressed_here || !changes.is_empty()) && self.drum_map_editor.before.is_none() {
            self.drum_map_editor.before = Some(self.project.clone());
        }
        for c in changes {
            self.apply_drum_change(track, c);
        }
        let busy = ctx.input(|i| i.pointer.any_down()) || ctx.egui_wants_keyboard_input();
        if !busy && let Some(before) = self.drum_map_editor.before.take() {
            self.checkpoint(before);
        }
        let result = if save {
            self.save_drum_map(track)
        } else if load {
            self.load_drum_map(track)
        } else {
            Ok(())
        };
        if let Err(e) = result {
            self.message = Some(e);
        }
        if !open {
            self.drum_map_editor.track = None;
        }
    }
}
