//! Gemeinsames Verhalten der Dialogfenster: Escape schließt den obersten offenen Dialog, beim
//! Öffnen bekommt der Dialog den Tastaturfokus und beim Schließen geht er zurück. Modale Dialoge
//! (kurze Formulare zum Bestätigen oder Abbrechen) halten Tab in sich und sperren die Bedienung
//! dahinter; die Engine läuft weiter, gesperrt ist nur die Oberfläche.

use egui::{Context, FocusDirection, Id, Key, LayerId, Order};

/// Wie sich ein Dialog gegenüber dem Rest des Fensters verhält.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Kind {
    /// Tab bleibt im Dialog, dahinter ist nichts bedienbar, bis er zu ist.
    Modal,
    /// Arbeitsfenster: nur der Fokus beim Öffnen und Schließen.
    Free,
}

/// Ein offener Dialog: wann er aufging und wer vorher den Fokus hatte.
#[derive(Debug)]
struct Open {
    id: Id,
    /// Bilder seit dem Öffnen.
    frames: u32,
    /// Widget mit Fokus vor dem Öffnen; bekommt ihn beim Schließen zurück.
    before: Option<Id>,
    /// In diesem Bild gezeichnet.
    seen: bool,
    modal: bool,
}

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
    /// Offene Dialoge für Fokus und Modalität.
    open: Vec<Open>,
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
            && ctx.memory(|m| m.top_modal_layer()).is_none_or(|l| {
                self.open
                    .iter()
                    .any(|o| backdrop(o.id) == l || o.id == l.id)
            });
    }

    /// Ist ein modaler Dialog offen? Dann wirken keine Tastenkürzel: Im Dialog hat ein Knopf den
    /// Fokus, die Leertaste löst ihn aus und soll nicht zugleich Start/Stopp sein. Die Engine
    /// läuft weiter.
    pub(crate) fn modal_open(&self) -> bool {
        self.open.iter().any(|o| o.modal)
    }

    /// Am Ende jedes Bildes, nach allen Fenstern.
    pub(crate) fn end_frame(&mut self, ctx: &Context) {
        self.text_focus = ctx.text_edit_focused();
        // Nicht mehr gezeichnet: der Dialog ist zu, der Fokus geht zurück.
        let mut closed = Vec::new();
        self.open.retain_mut(|o| {
            let keep = std::mem::take(&mut o.seen);
            if !keep {
                closed.push(o.before);
            }
            keep
        });
        for before in closed {
            ctx.memory_mut(|m| match before {
                Some(id) => m.request_focus(id),
                None => m.stop_text_input(),
            });
        }
        self.stack
            .retain(|id| ctx.memory(|m| m.areas().is_visible(&LayerId::new(Order::Middle, *id))));
    }

    /// Vor dem Zeichnen eines offenen Dialogs mit der Fenster-Id `id` aufrufen. Ist er der
    /// oberste offene Dialog und gilt das Escape dieses Bildes, wird er geschlossen (`open` auf
    /// `false`) und das Escape verbraucht, damit nicht noch ein zweiter Dialog zugeht.
    pub(crate) fn escape_closes(&mut self, ctx: &Context, id: Id, open: &mut bool) {
        self.show(ctx, id, open, Kind::Free);
    }

    /// Vor dem Zeichnen eines offenen Dialogs aufrufen: Escape wie [`Self::escape_closes`], dazu
    /// der Fokus beim Öffnen und, bei [`Kind::Modal`], die Sperre dahinter.
    pub(crate) fn show(&mut self, ctx: &Context, id: Id, open: &mut bool, kind: Kind) {
        let layer = LayerId::new(Order::Middle, id);
        self.focus(ctx, id, layer, kind);
        // Ein modaler Dialog hängt als Unterebene über seiner Sperrfläche; oben ist dann diese.
        let top_layer = ctx.memory(|m| m.areas().top_layer_id(Order::Middle));
        let on_top = top_layer == Some(layer) || top_layer == Some(backdrop(id));
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

    /// Fokus beim Öffnen: Im ersten Bild wird die Ebene des Dialogs zur Modal-Ebene (egui wertet
    /// sie ab dem nächsten Bild aus), im zweiten springt der Fokus auf das erste Element, das in
    /// diesem Bild nur im Dialog erreichbar ist. Freie Dialoge sind danach wieder frei.
    fn focus(&mut self, ctx: &Context, id: Id, layer: LayerId, kind: Kind) {
        let o = match self.open.iter().position(|o| o.id == id) {
            Some(i) => &mut self.open[i],
            None => {
                let before = ctx.memory(|m| m.focused());
                self.open.push(Open {
                    id,
                    frames: 0,
                    before,
                    seen: false,
                    modal: kind == Kind::Modal,
                });
                let last = self.open.len() - 1;
                &mut self.open[last]
            }
        };
        o.seen = true;
        if kind == Kind::Modal {
            // Wie egui::Modal: eine unsichtbare Fläche über dem ganzen Fenster fängt Klicks ab,
            // der Dialog liegt als Unterebene direkt darüber.
            let shield = backdrop(id);
            egui::Area::new(shield.id)
                .order(Order::Middle)
                .fixed_pos(ctx.content_rect().min)
                .show(ctx, |ui| {
                    ui.allocate_rect(ctx.content_rect(), egui::Sense::click_and_drag());
                });
            ctx.set_sublayer(shield, layer);
            ctx.memory_mut(|m| m.set_modal_layer(shield));
        } else if o.frames == 0 {
            ctx.memory_mut(|m| m.set_modal_layer(layer));
        }
        if o.frames == 1 {
            ctx.memory_mut(|m| {
                m.surrender_focus(m.focused().unwrap_or(Id::NULL));
                m.move_focus(FocusDirection::Next);
            });
        }
        o.frames = o.frames.saturating_add(1);
    }
}

/// Ebene der Sperrfläche hinter einem modalen Dialog.
fn backdrop(id: Id) -> LayerId {
    LayerId::new(Order::Middle, id.with("modal-sperre"))
}

/// Fenster-Id eines Dialogs aus seinem Titel, genau so gebildet wie in `Window::new(title)`.
pub(crate) fn window_id(title: &str) -> Id {
    use egui::IntoAtoms;
    Id::unique(title.into_atoms().text())
}
