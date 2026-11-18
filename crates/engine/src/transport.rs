//! Transport: Wiedergabe, Aufnahme, Locatoren und Cycle
//! (Spec-Abschnitt „Transport und Aufnahme“).
//!
//! Alle Positionen sind hier Sample-Frames, denn der Transport läuft im
//! Audio-Thread. Musikalische Positionen (Ticks) werden beim Senden eines
//! Befehls über die Tempo-Map in Frames umgerechnet.

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
#[repr(u8)]
pub enum TransportState {
    #[default]
    Stopped = 0,
    Playing = 1,
    Recording = 2,
}

impl TransportState {
    pub fn from_u8(v: u8) -> Self {
        match v {
            1 => Self::Playing,
            2 => Self::Recording,
            _ => Self::Stopped,
        }
    }
}

#[derive(Debug, Clone, Default)]
pub struct Transport {
    pub state: TransportState,
    /// Projekt-Cursor in Frames.
    pub cursor: u64,
    pub left_locator: u64,
    pub right_locator: u64,
    pub cycle: bool,
    /// Position, an der zuletzt gestartet wurde (für „Zurück zur Startposition“).
    start_position: u64,
}

impl Transport {
    pub fn play(&mut self) {
        if self.state == TransportState::Stopped {
            self.start_position = self.cursor;
        }
        self.state = TransportState::Playing;
    }

    pub fn record(&mut self) {
        if self.state == TransportState::Stopped {
            self.start_position = self.cursor;
        }
        self.state = TransportState::Recording;
    }

    /// Stopp. Ein zweites Stopp im gestoppten Zustand springt zum Anfang.
    pub fn stop(&mut self, return_to_start: bool) {
        if self.state == TransportState::Stopped {
            self.cursor = 0;
            return;
        }
        self.state = TransportState::Stopped;
        if return_to_start {
            self.cursor = self.start_position;
        }
    }

    pub fn is_running(&self) -> bool {
        self.state != TransportState::Stopped
    }

    fn cycle_active(&self) -> bool {
        self.cycle && self.right_locator > self.left_locator
    }

    /// Wie viele Frames ab dem Cursor am Stück gespielt werden können,
    /// bevor der Cycle zurückspringt (höchstens `max`).
    pub fn frames_until_wrap(&self, max: u64) -> u64 {
        if self.cycle_active() && self.cursor < self.right_locator {
            max.min(self.right_locator - self.cursor)
        } else {
            max
        }
    }

    /// Cursor um `frames` vorrücken; im Cycle springt er am rechten Locator
    /// sample-genau auf den linken zurück.
    pub fn advance(&mut self, frames: u64) {
        if !self.is_running() {
            return;
        }
        let was_inside = self.cursor < self.right_locator;
        let mut next = self.cursor + frames;
        if self.cycle_active() && was_inside && next >= self.right_locator {
            let (l, r) = (self.left_locator, self.right_locator);
            next = l + (next - r) % (r - l);
        }
        self.cursor = next;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn double_stop_returns_to_zero() {
        let mut t = Transport {
            cursor: 500,
            ..Default::default()
        };
        t.play();
        t.advance(100);
        t.stop(false);
        assert_eq!(t.cursor, 600);
        t.stop(false);
        assert_eq!(t.cursor, 0);
    }

    #[test]
    fn stop_returns_to_start_position() {
        let mut t = Transport {
            cursor: 500,
            ..Default::default()
        };
        t.play();
        t.advance(1000);
        t.stop(true);
        assert_eq!(t.cursor, 500);
    }

    #[test]
    fn cycle_wraps_at_right_locator() {
        let mut t = Transport {
            left_locator: 1000,
            right_locator: 2000,
            cursor: 1900,
            cycle: true,
            ..Default::default()
        };
        t.play();
        assert_eq!(t.frames_until_wrap(256), 100);
        t.advance(250);
        assert_eq!(t.cursor, 1150);
    }

    #[test]
    fn cursor_after_cycle_end_plays_on() {
        let mut t = Transport {
            left_locator: 1000,
            right_locator: 2000,
            cursor: 3000,
            cycle: true,
            ..Default::default()
        };
        t.play();
        assert_eq!(t.frames_until_wrap(256), 256);
        t.advance(256);
        assert_eq!(t.cursor, 3256);
    }
}
