//! Transport: Wiedergabe, Aufnahme, Locatoren und Cycle
//! (Spec-Abschnitt „Transport und Aufnahme“).

use studiocraft_project::Position;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum TransportState {
    #[default]
    Stopped,
    Playing,
    Recording,
}

#[derive(Debug, Clone, Default)]
pub struct Transport {
    pub state: TransportState,
    pub cursor: Position,
    pub left_locator: Position,
    pub right_locator: Position,
    pub cycle: bool,
    /// Position, an der zuletzt gestartet wurde (für „Zurück zur Startposition“).
    start_position: Position,
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
            self.cursor = Position(0);
            return;
        }
        self.state = TransportState::Stopped;
        if return_to_start {
            self.cursor = self.start_position;
        }
    }

    /// Cursor um `ticks` vorrücken; im Cycle springt er am rechten Locator zurück.
    pub fn advance(&mut self, ticks: u64) {
        if self.state == TransportState::Stopped {
            return;
        }
        let mut next = self.cursor.0 + ticks;
        let (l, r) = (self.left_locator.0, self.right_locator.0);
        if self.cycle && r > l && self.cursor.0 < r && next >= r {
            next = l + (next - r) % (r - l);
        }
        self.cursor = Position(next);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn double_stop_returns_to_zero() {
        let mut t = Transport { cursor: Position(500), ..Default::default() };
        t.play();
        t.advance(100);
        t.stop(false);
        assert_eq!(t.cursor, Position(600));
        t.stop(false);
        assert_eq!(t.cursor, Position(0));
    }

    #[test]
    fn stop_returns_to_start_position() {
        let mut t = Transport { cursor: Position(500), ..Default::default() };
        t.play();
        t.advance(1000);
        t.stop(true);
        assert_eq!(t.cursor, Position(500));
    }

    #[test]
    fn cycle_wraps_at_right_locator() {
        let mut t = Transport {
            left_locator: Position(1000),
            right_locator: Position(2000),
            cursor: Position(1900),
            cycle: true,
            ..Default::default()
        };
        t.play();
        t.advance(250);
        assert_eq!(t.cursor, Position(1150));
    }
}
