//! Nummernblock als Transportfeld (Spec „Nummernblock-Belegung“, Issue #3).
//!
//! egui meldet die Ziffern des Nummernblocks wie die der Zahlenreihe (`Key::Num1`), und `*` oder
//! das Komma nur als Text. Die App sieht dagegen, wo eine Taste liegt (winit `KeyLocation`), und
//! legt jeden Druck auf dem Nummernblock in einen [`KeypadTap`]. Das Projektfenster nimmt im
//! selben Frame das passende egui-Ereignis weg, damit die Zahlenreihe Werkzeuge wählt und der
//! Nummernblock den Transport bedient. Ohne Tap (Web, Tests) bleibt alles bei egui.

use std::collections::VecDeque;
use std::sync::{Arc, Mutex};

/// Eine Taste des Nummernblocks.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum KeypadKey {
    /// Ziffer 0–9.
    Digit(u8),
    Enter,
    Add,
    Subtract,
    Multiply,
    Divide,
    /// Komma bzw. Punkt.
    Decimal,
}

/// Ein Druck auf dem Nummernblock.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct KeypadPress {
    pub key: KeypadKey,
    /// Mit Umschalt (Spec: Umschalt + Mal holt gespielte Noten nachträglich).
    pub shift: bool,
    /// Wiederholung durch gehaltene Taste.
    pub repeat: bool,
}

/// Höchstens so viele Drücke warten auf den nächsten Frame; mehr werden verworfen.
const MAX_PENDING: usize = 64;

/// Warteschlange vom Fenster-Ereignis zur Oberfläche; beide laufen auf dem Haupt-Thread.
#[derive(Debug, Clone, Default)]
pub struct KeypadTap(Arc<Mutex<VecDeque<KeypadPress>>>);

impl KeypadTap {
    pub fn new() -> Self {
        Self::default()
    }

    /// Ein Druck auf dem Nummernblock (aus dem Fenster-Ereignis der App).
    pub fn push(&self, press: KeypadPress) {
        if let Ok(mut q) = self.0.lock()
            && q.len() < MAX_PENDING
        {
            q.push_back(press);
        }
    }

    fn drain(&self) -> Vec<KeypadPress> {
        self.0
            .lock()
            .map(|mut q| q.drain(..).collect())
            .unwrap_or_default()
    }
}

/// Nimmt das egui-Ereignis zu `key` aus der Eingabe (das erste passende Drücken und den Text).
fn take_egui_event(i: &mut egui::InputState, key: KeypadKey) {
    use egui::Key;
    let egui_key = match key {
        KeypadKey::Digit(d) => Key::from_name(&d.to_string()),
        KeypadKey::Enter => Some(Key::Enter),
        KeypadKey::Add => Some(Key::Plus),
        KeypadKey::Subtract => Some(Key::Minus),
        KeypadKey::Divide => Some(Key::Slash),
        KeypadKey::Multiply | KeypadKey::Decimal => None,
    };
    let text: &[&str] = match key {
        KeypadKey::Multiply => &["*"],
        KeypadKey::Decimal => &[",", "."],
        KeypadKey::Add => &["+"],
        KeypadKey::Subtract => &["-"],
        KeypadKey::Divide => &["/"],
        KeypadKey::Digit(_) | KeypadKey::Enter => &[],
    };
    let (mut key_done, mut text_done) = (egui_key.is_none(), text.is_empty());
    i.events.retain(|e| match e {
        egui::Event::Key {
            key, pressed: true, ..
        } if !key_done && Some(*key) == egui_key => {
            key_done = true;
            false
        }
        egui::Event::Text(t) if !text_done && text.contains(&t.as_str()) => {
            text_done = true;
            false
        }
        _ => true,
    });
}

impl crate::ProjectWindow {
    /// Nummernblock-Drücke der App für dieses Fenster.
    pub fn set_keypad(&mut self, tap: KeypadTap) {
        self.keypad = Some(tap);
    }

    /// Wartende Drücke abholen, ihre egui-Ereignisse wegnehmen und sie ausführen. Hat ein
    /// Textfeld die Tastatur, bleibt das egui-Ereignis dort und der Transport ruht.
    pub(crate) fn handle_keypad(&mut self, ui: &egui::Ui) {
        let Some(tap) = &self.keypad else {
            return;
        };
        let presses = tap.drain();
        if presses.is_empty() || ui.ctx().egui_wants_keyboard_input() {
            return;
        }
        ui.input_mut(|i| {
            for p in &presses {
                take_egui_event(i, p.key);
            }
        });
        for p in presses {
            self.run_keypad(p);
        }
    }

    /// Belegung nach der Spec: Enter Start, 0 Stopp, * Aufnahme, Komma zum Anfang, 1/2 zu den
    /// Locatoren, 3–9 zu den Markern, −/+ spulen um einen Takt, / Cycle.
    pub(crate) fn run_keypad(&mut self, p: KeypadPress) {
        // Gehaltene Tasten wiederholen nur das Spulen.
        if p.repeat && !matches!(p.key, KeypadKey::Add | KeypadKey::Subtract) {
            return;
        }
        match p.key {
            KeypadKey::Enter => {
                if !self.is_playing() {
                    self.play();
                }
            }
            // Stopp mit dem Stopp-Verhalten aus den Einstellungen (zweimal: Anfang/Locator).
            KeypadKey::Digit(0) => self.stop(),
            KeypadKey::Multiply if p.shift => self.retrospective_record(),
            KeypadKey::Multiply => self.toggle_record(),
            KeypadKey::Decimal => self.locate(0),
            KeypadKey::Digit(1) => self.locate(self.left_locator),
            KeypadKey::Digit(2) => self.locate(self.right_locator),
            KeypadKey::Digit(d) => self.locate_marker(u32::from(d)),
            KeypadKey::Subtract => {
                // Zum Anfang dieses Takts bzw. des vorigen (Taktwechsel beachtet).
                let (bar, start, _) = self.project.tempo.bar_at(self.cursor);
                let to = if self.cursor > start {
                    start
                } else {
                    self.project.tempo.bar_start(bar.saturating_sub(1))
                };
                self.locate(to);
            }
            KeypadKey::Add => {
                let (bar, _, _) = self.project.tempo.bar_at(self.cursor);
                let to = self.project.tempo.bar_start(bar + 1);
                self.locate(to);
            }
            KeypadKey::Divide => self.toggle_cycle(),
        }
    }
}
