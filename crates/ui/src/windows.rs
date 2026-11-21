//! Eigene Fenster (Spec „Fenster-Architektur“): F3 öffnet den Mixer, Strg+E den Editor des
//! gewählten Events (Key-Editor für MIDI-Parts, Sample-Editor für Audio) als eigenes
//! Fenster. Jedes Fenster merkt sich Position und Größe in den Einstellungen; ohne
//! Unterstützung für mehrere Fenster (Tests, Web) erscheinen sie eingebettet.
//!
//! Solange ein Inhalt im eigenen Fenster liegt, zeigt die untere Zone an seiner Stelle nur
//! einen Hinweis, damit nichts doppelt gezeichnet wird.

use crate::i18n::tr;
use egui::{Context, Ui, ViewportBuilder, ViewportClass, ViewportId};

use crate::settings::{WindowRect, WindowRects};
use crate::{LowerZone, ProjectWindow};

/// Welcher Editor im eigenen Fenster läuft.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum EditorWindow {
    Key,
    Sample,
    Score,
}

/// Zustand der eigenen Fenster.
#[derive(Debug, Default)]
pub(crate) struct Windows {
    pub mixer: bool,
    pub editor: Option<EditorWindow>,
    /// Aktuelle Lage (wird laufend nachgeführt).
    pub rects: WindowRects,
    /// Lage beim Öffnen: daraus wird das Fenster gebaut, damit es nicht gegen das
    /// Verschieben arbeitet.
    open_mixer: Option<WindowRect>,
    open_editor: Option<WindowRect>,
    /// Lage, die zuletzt in die Einstellungen geschrieben wurde.
    saved: WindowRects,
}

impl Windows {
    pub fn new(rects: WindowRects) -> Self {
        Self {
            rects,
            saved: rects,
            ..Self::default()
        }
    }
}

const MIXER_SIZE: [f32; 2] = [900.0, 420.0];
const EDITOR_SIZE: [f32; 2] = [900.0, 480.0];

fn builder(title: &str, rect: Option<WindowRect>, default: [f32; 2]) -> ViewportBuilder {
    let size = rect.map_or(default, |r| r.size);
    let mut b = ViewportBuilder::default()
        .with_title(format!("{title} – StudioCraft"))
        .with_inner_size(size);
    if let Some(p) = rect.and_then(|r| r.pos) {
        b = b.with_position(p);
    }
    b
}

impl ProjectWindow {
    /// F3: Mixer als eigenes Fenster öffnen bzw. schließen.
    pub(crate) fn toggle_mixer_window(&mut self) {
        self.windows.mixer = !self.windows.mixer;
        if self.windows.mixer {
            self.windows.open_mixer = self.windows.rects.mixer;
        }
    }

    /// Strg+E: Editor des gewählten Events im eigenen Fenster (MIDI-Part: Key-Editor,
    /// Audio-Event: Sample-Editor). Ohne Auswahl nimmt er den Part, der gerade im
    /// Key-Editor steht.
    pub(crate) fn open_editor_window(&mut self) {
        let lower = (self.show_lower, self.lower);
        let id = self.selection.first().copied().or(self.key_editor.part);
        let Some(ev) = id.and_then(|i| self.project.event(i)) else {
            self.message = Some(tr("Erst ein Event oder einen Part auswählen").into());
            return;
        };
        let (id, audio) = (ev.id, ev.clip.is_some());
        let kind = if audio {
            self.open_sample_editor(id);
            EditorWindow::Sample
        } else {
            self.open_key_editor_from_selection(id);
            EditorWindow::Key
        };
        // Die Editoren schalten beim Öffnen die untere Zone um; das rückgängig machen.
        (self.show_lower, self.lower) = lower;
        self.windows.editor = Some(kind);
        self.windows.open_editor = self.windows.rects.editor;
    }

    /// Noten-Editor des aktiven bzw. gewählten MIDI-Parts im eigenen Fenster.
    pub(crate) fn open_score_window(&mut self) {
        if self.list_part().is_none() {
            self.message = Some(tr("Erst einen MIDI-Part auswählen").into());
            return;
        }
        self.windows.editor = Some(EditorWindow::Score);
        self.windows.open_editor = self.windows.rects.editor;
    }

    /// Steht `zone` gerade im eigenen Fenster?
    pub(crate) fn in_own_window(&self, zone: LowerZone) -> bool {
        match zone {
            LowerZone::Mixer => self.windows.mixer,
            LowerZone::Editor => self.windows.editor == Some(EditorWindow::Key),
            LowerZone::SampleEditor => self.windows.editor == Some(EditorWindow::Sample),
            LowerZone::List => false,
            LowerZone::Score => self.windows.editor == Some(EditorWindow::Score),
        }
    }

    /// Hinweis in der unteren Zone, solange ihr Inhalt im eigenen Fenster liegt.
    pub(crate) fn own_window_placeholder(&mut self, ui: &mut Ui, zone: LowerZone) {
        ui.add_space(12.0);
        ui.weak(tr("Läuft im eigenen Fenster."));
        if ui.button(tr("Zurückholen")).clicked() {
            match zone {
                LowerZone::Mixer => self.windows.mixer = false,
                _ => self.windows.editor = None,
            }
        }
    }

    /// Zeichnet die offenen eigenen Fenster; einmal je Bild nach dem Hauptfenster.
    pub(crate) fn show_windows(&mut self, ctx: &Context) {
        self.window_pointer_down = false;
        if self.windows.mixer {
            let b = builder(
                crate::i18n::tr("Mixer"),
                self.windows.open_mixer,
                MIXER_SIZE,
            );
            let (open, rect) = ctx.show_viewport_immediate(
                ViewportId::from_hash_of("mixer-fenster"),
                b,
                |ui, class| self.window_body(ui, class, LowerZone::Mixer),
            );
            self.windows.mixer = open;
            if let Some(r) = rect {
                self.windows.rects.mixer = Some(r);
            }
        }
        if let Some(kind) = self.windows.editor {
            let (title, zone) = match kind {
                EditorWindow::Key => (tr("Key-Editor"), LowerZone::Editor),
                EditorWindow::Sample => (tr("Sample-Editor"), LowerZone::SampleEditor),
                EditorWindow::Score => (tr("Noten-Editor"), LowerZone::Score),
            };
            let b = builder(title, self.windows.open_editor, EDITOR_SIZE);
            let (open, rect) = ctx.show_viewport_immediate(
                ViewportId::from_hash_of("editor-fenster"),
                b,
                |ui, class| self.window_body(ui, class, zone),
            );
            if !open {
                self.windows.editor = None;
            }
            if let Some(r) = rect {
                self.windows.rects.editor = Some(r);
            }
        }
        // Lage speichern, sobald sie sich geändert hat und nicht mehr gezogen wird.
        let dragging = ctx.input(|i| i.pointer.any_down());
        if !dragging && self.windows.rects != self.windows.saved {
            self.windows.saved = self.windows.rects;
            self.save_settings();
        }
    }

    /// Inhalt eines eigenen Fensters. Gibt zurück, ob es offen bleibt, und seine Lage.
    fn window_body(
        &mut self,
        ui: &mut Ui,
        class: ViewportClass,
        zone: LowerZone,
    ) -> (bool, Option<WindowRect>) {
        let mut open = true;
        let body = |w: &mut Self, ui: &mut Ui, open: &mut bool| {
            ui.horizontal(|ui| {
                if ui.button(tr("In die untere Zone")).clicked() {
                    *open = false;
                    w.show_lower = true;
                    w.lower = zone;
                }
            });
            ui.separator();
            match zone {
                LowerZone::Mixer => crate::mixer::show(w, ui),
                LowerZone::Editor => crate::keyeditor::show(w, ui),
                LowerZone::SampleEditor => crate::sampleeditor::show(w, ui),
                LowerZone::List => crate::listeditor::show(w, ui),
                LowerZone::Score => crate::score::show(w, ui),
            }
        };
        if class == ViewportClass::EmbeddedWindow {
            body(self, ui, &mut open);
            return (open, None);
        }
        // Tasten, die in diesem Fenster gedrückt werden, kommen nur in seiner Eingabe an.
        self.handle_window_keys(ui);
        // Drücken hier beginnt eine Geste für den Rückgängig-Verlauf (panel_undo.rs).
        let (press, down) = ui.input(|i| {
            (
                i.pointer
                    .any_pressed()
                    .then(|| i.pointer.press_origin())
                    .flatten(),
                i.pointer.any_down(),
            )
        });
        self.panel_edit_press(press);
        self.window_pointer_down |= down;
        egui::CentralPanel::default().show(ui, |ui| body(self, ui, &mut open));
        let (close, outer, inner) = ui.input(|i| {
            let v = i.viewport();
            (v.close_requested(), v.outer_rect, v.inner_rect)
        });
        if close {
            open = false;
        }
        let rect = inner.map(|r| WindowRect {
            pos: outer.map(|o| [o.min.x, o.min.y]),
            size: [r.width(), r.height()],
        });
        (open, rect)
    }
}
