//! Leitet Nummernblock-Tasten an das Projektfenster weiter (Issue #3).
//!
//! eframe läuft auf einer eigenen winit-Ereignisschleife; dieser Wrapper sieht jedes
//! Tastenereignis vor eframe, legt Drücke auf dem Nummernblock in den [`KeypadTap`] und gibt
//! alles unverändert weiter.

use studiocraft_ui::keypad::{KeypadKey, KeypadPress, KeypadTap};
use winit::application::ApplicationHandler;
use winit::event::{DeviceEvent, DeviceId, ElementState, StartCause, WindowEvent};
use winit::event_loop::ActiveEventLoop;
use winit::keyboard::{KeyCode, KeyLocation, ModifiersState, PhysicalKey};
use winit::window::WindowId;

/// Taste des Nummernblocks zu einem physischen Tastencode.
pub fn keypad_key(code: KeyCode) -> Option<KeypadKey> {
    Some(match code {
        KeyCode::Numpad0 => KeypadKey::Digit(0),
        KeyCode::Numpad1 => KeypadKey::Digit(1),
        KeyCode::Numpad2 => KeypadKey::Digit(2),
        KeyCode::Numpad3 => KeypadKey::Digit(3),
        KeyCode::Numpad4 => KeypadKey::Digit(4),
        KeyCode::Numpad5 => KeypadKey::Digit(5),
        KeyCode::Numpad6 => KeypadKey::Digit(6),
        KeyCode::Numpad7 => KeypadKey::Digit(7),
        KeyCode::Numpad8 => KeypadKey::Digit(8),
        KeyCode::Numpad9 => KeypadKey::Digit(9),
        KeyCode::NumpadEnter => KeypadKey::Enter,
        KeyCode::NumpadAdd => KeypadKey::Add,
        KeyCode::NumpadSubtract => KeypadKey::Subtract,
        KeyCode::NumpadMultiply => KeypadKey::Multiply,
        KeyCode::NumpadDivide => KeypadKey::Divide,
        KeyCode::NumpadDecimal | KeyCode::NumpadComma => KeypadKey::Decimal,
        _ => return None,
    })
}

/// eframe-Anwendung plus Nummernblock-Abgriff.
pub struct KeypadApp<A> {
    pub inner: A,
    pub tap: KeypadTap,
    modifiers: ModifiersState,
}

impl<A> KeypadApp<A> {
    pub fn new(inner: A, tap: KeypadTap) -> Self {
        Self {
            inner,
            tap,
            modifiers: ModifiersState::empty(),
        }
    }

    fn observe(&mut self, event: &WindowEvent) {
        match event {
            WindowEvent::ModifiersChanged(m) => self.modifiers = m.state(),
            WindowEvent::KeyboardInput { event, .. }
                if event.state == ElementState::Pressed
                    && event.location == KeyLocation::Numpad =>
            {
                // Mit Strg oder Alt bleibt die Taste ein normales Kürzel.
                if self.modifiers.control_key()
                    || self.modifiers.alt_key()
                    || self.modifiers.super_key()
                {
                    return;
                }
                if let PhysicalKey::Code(code) = event.physical_key
                    && let Some(key) = keypad_key(code)
                {
                    self.tap.push(KeypadPress {
                        key,
                        shift: self.modifiers.shift_key(),
                        repeat: event.repeat,
                    });
                }
            }
            _ => {}
        }
    }
}

impl<T: 'static, A: ApplicationHandler<T>> ApplicationHandler<T> for KeypadApp<A> {
    fn resumed(&mut self, event_loop: &ActiveEventLoop) {
        self.inner.resumed(event_loop);
    }

    fn window_event(&mut self, event_loop: &ActiveEventLoop, id: WindowId, event: WindowEvent) {
        self.observe(&event);
        self.inner.window_event(event_loop, id, event);
    }

    fn new_events(&mut self, event_loop: &ActiveEventLoop, cause: StartCause) {
        self.inner.new_events(event_loop, cause);
    }

    fn user_event(&mut self, event_loop: &ActiveEventLoop, event: T) {
        self.inner.user_event(event_loop, event);
    }

    fn device_event(&mut self, event_loop: &ActiveEventLoop, id: DeviceId, event: DeviceEvent) {
        self.inner.device_event(event_loop, id, event);
    }

    fn about_to_wait(&mut self, event_loop: &ActiveEventLoop) {
        self.inner.about_to_wait(event_loop);
    }

    fn suspended(&mut self, event_loop: &ActiveEventLoop) {
        self.inner.suspended(event_loop);
    }

    fn exiting(&mut self, event_loop: &ActiveEventLoop) {
        self.inner.exiting(event_loop);
    }

    fn memory_warning(&mut self, event_loop: &ActiveEventLoop) {
        self.inner.memory_warning(event_loop);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn keypad_codes_map_and_main_row_does_not() {
        assert_eq!(keypad_key(KeyCode::Numpad1), Some(KeypadKey::Digit(1)));
        assert_eq!(
            keypad_key(KeyCode::NumpadMultiply),
            Some(KeypadKey::Multiply)
        );
        assert_eq!(keypad_key(KeyCode::NumpadComma), Some(KeypadKey::Decimal));
        assert_eq!(keypad_key(KeyCode::Digit1), None);
        assert_eq!(keypad_key(KeyCode::Enter), None);
    }
}
