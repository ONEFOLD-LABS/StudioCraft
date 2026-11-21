//! Gemeinsames Verhalten der Dialogfenster: Escape schließt den obersten offenen Dialog.

use egui::{Context, Id, Key, LayerId, Order};

/// Zustand für Escape über alle Dialoge hinweg.
#[derive(Debug, Default)]
pub(crate) struct Dialogs {
    /// Escape dieses Bildes, bis ein Dialog es verbraucht.
    escape: bool,
    /// Hatte im letzten Bild ein Textfeld den Fokus? egui löst den Fokus bei Escape schon zu
    /// Beginn des Bildes; dann verwirft Escape nur die Eingabe.
    text_focus: bool,
    /// Dialoge von unten nach oben, so wie sie zuletzt nach vorn kamen. egui hält die Reihenfolge
    /// der Ebenen privat und lässt geschlossene Fenster darin stehen.
    stack: Vec<Id>,
}

impl Dialogs {
    /// Zu Beginn jedes Bildes, vor allen Fenstern.
    ///
    /// Escape gilt den Dialogen nicht, solange ein Textfeld den Fokus hat, solange ein Menü oder
    /// Popup offen ist (das schließt egui selbst) und solange eine modale Rückfrage offen ist
    /// (sie hat ihr eigenes Escape = Abbrechen).
    pub(crate) fn begin_frame(&mut self, ctx: &Context) {
        self.escape = ctx.input(|i| i.key_pressed(Key::Escape) && i.modifiers.is_none())
            && !self.text_focus
            && !ctx.text_edit_focused()
            && !egui::Popup::is_any_open(ctx)
            && ctx.memory(|m| m.top_modal_layer()).is_none();
    }

    /// Am Ende jedes Bildes, nach allen Fenstern.
    pub(crate) fn end_frame(&mut self, ctx: &Context) {
        self.text_focus = ctx.text_edit_focused();
        self.stack
            .retain(|id| ctx.memory(|m| m.areas().is_visible(&LayerId::new(Order::Middle, *id))));
    }

    /// Vor dem Zeichnen eines offenen Dialogs mit der Fenster-Id `id` aufrufen. Ist er der
    /// oberste offene Dialog und gilt das Escape dieses Bildes, wird er geschlossen (`open` auf
    /// `false`) und das Escape verbraucht, damit nicht noch ein zweiter Dialog zugeht.
    pub(crate) fn escape_closes(&mut self, ctx: &Context, id: Id, open: &mut bool) {
        let layer = LayerId::new(Order::Middle, id);
        let on_top = ctx.memory(|m| m.areas().top_layer_id(Order::Middle)) == Some(layer);
        if on_top || !self.stack.contains(&id) {
            self.stack.retain(|d| *d != id);
            self.stack.push(id);
        }
        let visible =
            |d: &Id| ctx.memory(|m| m.areas().is_visible(&LayerId::new(Order::Middle, *d)));
        let top = self
            .stack
            .iter()
            .rev()
            .find(|d| **d == id || visible(d))
            .copied();
        if self.escape && top == Some(id) {
            *open = false;
            self.escape = false;
            self.stack.retain(|d| *d != id);
        }
    }
}

/// Fenster-Id eines Dialogs aus seinem Titel, genau so gebildet wie in `Window::new(title)`.
pub(crate) fn window_id(title: &str) -> Id {
    use egui::IntoAtoms;
    Id::unique(title.into_atoms().text())
}
