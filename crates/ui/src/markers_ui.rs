//! Marker in der Oberfläche (Issue #11): Marker-Spur anlegen, Marker am Cursor einfügen,
//! Cycle-Marker aus den Locatoren, zu Markern springen und die Marker-Liste im Inspector.

use crate::i18n::tr;
use egui::Ui;
use studiocraft_project::TrackKind;

use crate::ProjectWindow;

impl ProjectWindow {
    /// Index der Marker-Spur, falls es eine gibt.
    pub(crate) fn marker_track(&self) -> Option<usize> {
        self.project
            .tracks
            .iter()
            .position(|t| t.kind == TrackKind::Marker)
    }

    /// Marker-Spur anlegen (eine je Projekt) und wählen.
    pub(crate) fn add_marker_track(&mut self) -> usize {
        if let Some(t) = self.marker_track() {
            self.selected_track = Some(t);
            return t;
        }
        let t = self.project.add_track(TrackKind::Marker, "Marker");
        self.selected_track = Some(t);
        t
    }

    /// Marker am Cursor einfügen (legt die Marker-Spur bei Bedarf an).
    pub(crate) fn insert_marker(&mut self) {
        let before = self.project.clone();
        if self.marker_track().is_none() {
            self.add_marker_track();
        }
        self.project.add_marker(self.cursor);
        self.checkpoint(before);
    }

    /// Cycle-Marker zwischen den Locatoren.
    pub(crate) fn cycle_marker_from_locators(&mut self) {
        let before = self.project.clone();
        if self.marker_track().is_none() {
            self.add_marker_track();
        }
        if self
            .project
            .add_cycle_marker(self.left_locator, self.right_locator)
            .is_none()
        {
            self.message = Some(tr("Cycle-Marker: die Locatoren liegen aufeinander").into());
        }
        self.checkpoint(before);
    }

    /// Zum Positionsmarker mit Nummer `id`.
    pub(crate) fn locate_marker(&mut self, id: u32) {
        match self.project.marker(id).map(|m| m.start) {
            Some(at) => self.locate(at),
            None => {
                self.message = Some(crate::i18n::trf(
                    "Marker {id} gibt es nicht",
                    &[("id", &id)],
                ))
            }
        }
    }

    /// Marker `i` anwählen: Positionsmarker setzen den Cursor, Cycle-Marker die Locatoren.
    pub(crate) fn select_marker(&mut self, i: usize) {
        let Some(m) = self.project.markers.get(i).cloned() else {
            return;
        };
        match m.end {
            Some(end) => {
                self.left_locator = m.start;
                self.right_locator = end;
                self.send_locators();
                self.locate(m.start);
            }
            None => self.locate(m.start),
        }
    }

    /// Inspector der Marker-Spur: Liste mit Name und Position, Löschen, Einfügen.
    pub(crate) fn markers_ui(&mut self, ui: &mut Ui) {
        ui.horizontal(|ui| {
            if ui.button(tr("Marker einfügen")).clicked() {
                self.insert_marker();
            }
            if ui.button(tr("Cycle aus Locatoren")).clicked() {
                self.cycle_marker_from_locators();
            }
        });
        if self.project.markers.is_empty() {
            ui.weak(tr("Noch keine Marker"));
            return;
        }
        let tempo = self.project.tempo.clone();
        let mut remove = None;
        let mut go = None;
        let mut changed = None;
        egui::Grid::new("marker-liste")
            .num_columns(4)
            .striped(true)
            .show(ui, |ui| {
                for (i, m) in self.project.markers.iter().enumerate() {
                    let id = if m.is_cycle() {
                        format!("C{}", m.id)
                    } else {
                        m.id.to_string()
                    };
                    if ui
                        .small_button(id)
                        .on_hover_text(tr("Hinspringen"))
                        .clicked()
                    {
                        go = Some(i);
                    }
                    let mut name = m.name.clone();
                    let label = crate::i18n::trf("Name von Marker {id}", &[("id", &m.id)]);
                    let r = ui.add(egui::TextEdit::singleline(&mut name).desired_width(90.0));
                    r.widget_info(|| {
                        egui::WidgetInfo::labeled(egui::Role::TextInput, true, label.as_str())
                    });
                    if r.changed() {
                        changed = Some((i, name));
                    }
                    let pos = match m.end {
                        Some(e) => format!(
                            "{} – {}",
                            tempo.format_position(studiocraft_project::Position(m.start)),
                            tempo.format_position(studiocraft_project::Position(e))
                        ),
                        None => tempo.format_position(studiocraft_project::Position(m.start)),
                    };
                    ui.weak(pos);
                    if ui.small_button(tr("Löschen")).clicked() {
                        remove = Some(i);
                    }
                    ui.end_row();
                }
            });
        if let Some((i, name)) = changed
            && let Some(m) = self.project.markers.get_mut(i)
        {
            m.name = name;
            self.dirty = true;
        }
        if let Some(i) = go {
            self.select_marker(i);
        }
        if let Some(i) = remove {
            let before = self.project.clone();
            self.project.remove_marker(i);
            self.checkpoint(before);
        }
    }
}
