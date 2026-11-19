//! Controller-Daten in MIDI-Parts (Spec „Editoren“ → Controller-Lanes, Issue #17):
//! Pitchbend, Controller 0–127 und Aftertouch, je Punkt ein Wert zu einem Zeitpunkt.
//! Zwischen zwei Punkten gilt der frühere Wert (Stufen, wie MIDI sie sendet).

use serde::{Deserialize, Serialize};

/// Art eines Controller-Werts.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
pub enum CtrlKind {
    /// Pitchbend, 14 Bit (0–16383, Mitte 8192).
    PitchBend,
    /// Controller (Control Change) mit seiner Nummer 0–127.
    Cc(u8),
    /// Kanal-Aftertouch (Druck), 0–127.
    Aftertouch,
}

/// Haltepedal: wird nicht als Controller aufgenommen, weil es schon in die Notenlängen
/// eingeht.
pub const SUSTAIN_CC: u8 = 64;
/// Höchstzahl Controller-Punkte je Part (Schutz vor riesigen Dateien).
pub const MAX_CTRL_POINTS: usize = 100_000;

impl CtrlKind {
    /// Größter Wert dieser Art.
    pub fn max(self) -> u16 {
        match self {
            CtrlKind::PitchBend => 16_383,
            CtrlKind::Cc(_) | CtrlKind::Aftertouch => 127,
        }
    }

    /// Ruhewert (Pitchbend in der Mitte, sonst 0).
    pub fn rest(self) -> u16 {
        match self {
            CtrlKind::PitchBend => 8_192,
            _ => 0,
        }
    }

    /// Kurzer Name für die Oberfläche.
    pub fn label(self) -> String {
        match self {
            CtrlKind::PitchBend => "Pitchbend".into(),
            CtrlKind::Cc(n) => match n {
                1 => "CC 1 Modulation".into(),
                7 => "CC 7 Lautstärke".into(),
                10 => "CC 10 Panorama".into(),
                11 => "CC 11 Expression".into(),
                n => format!("CC {n}"),
            },
            CtrlKind::Aftertouch => "Aftertouch".into(),
        }
    }

    /// MIDI-Nachricht auf Kanal 1 für `value` (auf den gültigen Bereich begrenzt).
    pub fn midi(self, value: u16) -> [u8; 3] {
        let v = value.min(self.max());
        match self {
            CtrlKind::PitchBend => [0xe0, (v & 0x7f) as u8, (v >> 7) as u8],
            CtrlKind::Cc(n) => [0xb0, n & 0x7f, v as u8],
            CtrlKind::Aftertouch => [0xd0, v as u8, 0],
        }
    }

    /// Liest Art und Wert aus einer MIDI-Nachricht (beliebiger Kanal). Noten und andere
    /// Nachrichten ergeben `None`.
    pub fn from_midi(data: [u8; 3]) -> Option<(CtrlKind, u16)> {
        match data[0] & 0xf0 {
            0xe0 => Some((
                CtrlKind::PitchBend,
                u16::from(data[1] & 0x7f) | (u16::from(data[2] & 0x7f) << 7),
            )),
            0xb0 => Some((CtrlKind::Cc(data[1] & 0x7f), u16::from(data[2] & 0x7f))),
            0xd0 => Some((CtrlKind::Aftertouch, u16::from(data[1] & 0x7f))),
            _ => None,
        }
    }
}

/// Ein Controller-Wert in einem Part; `start` in Ticks relativ zum Part-Anfang.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct CtrlPoint {
    pub kind: CtrlKind,
    pub start: i64,
    pub value: u16,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn midi_round_trip() {
        for (kind, value) in [
            (CtrlKind::PitchBend, 0),
            (CtrlKind::PitchBend, 8_192),
            (CtrlKind::PitchBend, 16_383),
            (CtrlKind::Cc(1), 64),
            (CtrlKind::Cc(127), 127),
            (CtrlKind::Aftertouch, 99),
        ] {
            assert_eq!(CtrlKind::from_midi(kind.midi(value)), Some((kind, value)));
        }
        // Values are clamped, notes are not controllers.
        assert_eq!(CtrlKind::Cc(7).midi(500), [0xb0, 7, 127]);
        assert_eq!(CtrlKind::from_midi([0x90, 60, 100]), None);
        // Any channel is read.
        assert_eq!(
            CtrlKind::from_midi([0xb3, 1, 10]),
            Some((CtrlKind::Cc(1), 10))
        );
    }
}
