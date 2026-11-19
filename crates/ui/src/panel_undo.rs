//! Undo für Mixer, Inspector, rechte Zone und eigene Fenster (Spec „Mixer“; Issue #24):
//! Fader, Panorama, Mute/Solo, Sends, Routing, Namen usw. werden dort direkt im Projekt
//! geändert. Statt jedes Bedienelement einzeln anzupassen, gilt eine ganze Geste als ein
//! Rückgängig-Schritt:
//!
//! - Wird die Maus in einer dieser Zonen gedrückt, entsteht **am Anfang des Bildes** (bevor ein
//!   Regler springt) ein Schnappschuss des Projekts – einmal je Geste, nicht je Bild.
//! - Ist die Maus wieder oben und tippt niemand mehr in ein Textfeld, wird aus einer Änderung
//!   genau ein Schritt. Ein ganzer Fader-Zug ist so ein Schritt.
//! - Hat ein Bedienelement schon selbst einen Schritt angelegt (Routing, Inserts), liegt der
//!   Schnappschuss bereits oben im Verlauf; dann entsteht kein zweiter.

use egui::{Context, Pos2, Rect};

use crate::ProjectWindow;

impl ProjectWindow {
    /// Am Anfang eines Bildes: Drücken in Mixer, linker oder rechter Zone (Lage aus dem
    /// letzten Bild) beginnt eine Geste.
    pub(crate) fn panel_edit_start(&mut self, ctx: &Context) {
        let press = ctx.input(|i| {
            i.pointer
                .any_pressed()
                .then(|| i.pointer.press_origin())
                .flatten()
        });
        let z = &self.last_zones;
        let zones: [Option<Rect>; 3] = [z.left, z.lower, z.right];
        if let Some(p) = press
            && zones.iter().flatten().any(|r| r.contains(p))
        {
            self.begin_panel_edit();
        }
        // Auch Tastatur: Bekommt ein Textfeld den Fokus (Tab, Kurzbefehl), beginnt ebenfalls
        // eine Geste. Felder, die selbst einen Schritt anlegen (Info-Zeile), stört das nicht.
        if ctx.egui_wants_keyboard_input() {
            self.begin_panel_edit();
        }
    }

    /// Drücken in einem eigenen Fenster (Mixer, Editor): dort zählt jedes Drücken.
    pub(crate) fn panel_edit_press(&mut self, press: Option<Pos2>) {
        if press.is_some() {
            self.begin_panel_edit();
        }
    }

    fn begin_panel_edit(&mut self) {
        if self.panel_edit_before.is_none() {
            self.panel_edit_before = Some(self.project.clone());
        }
    }

    /// Am Ende eines Bildes: Ist die Geste vorbei, wird aus einer Änderung ein Schritt.
    pub(crate) fn panel_edit_end(&mut self, ctx: &Context) {
        if self.panel_edit_before.is_none() {
            return;
        }
        let busy = ctx.input(|i| i.pointer.any_down())
            || self.window_pointer_down
            || ctx.egui_wants_keyboard_input();
        if busy {
            return;
        }
        if let Some(before) = self.panel_edit_before.take()
            && before != self.project
            && self.undo.last() != Some(&before)
        {
            self.checkpoint(before);
        }
    }
}
