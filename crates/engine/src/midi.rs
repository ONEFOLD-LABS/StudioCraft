//! MIDI-Eingang (Keyboard) über midir: ALSA, CoreMIDI, WinMM.
//!
//! Alle Eingänge werden geöffnet (wie „All MIDI Inputs“). Nachrichten gehen
//! über einen Puffer direkt in den Audio-Thread; die Oberfläche ist nicht beteiligt.

use std::sync::{Arc, Mutex};

/// Eine Nachricht vom Keyboard.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MidiMsg {
    Note {
        key: u8,
        velocity: u8,
        on: bool,
    },
    /// Controller, Pitchbend, Aftertouch: roh an das Instrument.
    Other([u8; 3]),
}

/// Controller-Nummer des Haltepedals.
pub const SUSTAIN: u8 = 64;

/// Eine gespielte Note mit Projektposition (Frames). Gemeldet wird jede Note auf dem
/// Keyboard-Ziel, auch ohne Aufnahme (für die nachträgliche Aufnahme).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct RecordedMidi {
    pub pos: u64,
    pub key: u8,
    pub velocity: u8,
    pub on: bool,
    /// Während einer Aufnahme gespielt.
    pub recording: bool,
    /// Während der Transport lief (sonst steht `pos` still).
    pub running: bool,
    /// Frei laufender Frame-Zähler der Engine (für Noten im Stillstand).
    pub clock: u64,
    /// Aufgenommener Controller (Pitchbend, CC außer Haltepedal, Aftertouch) als rohe
    /// MIDI-Nachricht; dann gelten `key`, `velocity` und `on` nicht.
    pub ctrl: Option<[u8; 3]>,
}

/// Wertet eine MIDI-Nachricht aus: Noten (Note-On mit Velocity 0 = Off),
/// Controller, Pitchbend und Aftertouch. Alles andere wird ignoriert.
pub fn parse(msg: &[u8]) -> Option<MidiMsg> {
    let (&status, rest) = msg.split_first()?;
    let (&d1, rest) = rest.split_first()?;
    let d2 = rest.first().copied().unwrap_or(0);
    let (key, velocity) = (d1 & 0x7f, d2 & 0x7f);
    match status & 0xf0 {
        0x90 if velocity > 0 => Some(MidiMsg::Note {
            key,
            velocity,
            on: true,
        }),
        0x90 | 0x80 => Some(MidiMsg::Note {
            key,
            velocity,
            on: false,
        }),
        0xa0 | 0xb0 | 0xe0 => Some(MidiMsg::Other([status, key, velocity])),
        0xd0 => Some(MidiMsg::Other([status, key, 0])),
        _ => None,
    }
}

/// Gemeinsamer Schreibzugang zum Eingangspuffer des Audio-Threads.
pub type MidiSender = Arc<Mutex<rtrb::Producer<MidiMsg>>>;

/// Offene MIDI-Eingänge. Beim Drop werden die Verbindungen geschlossen.
pub struct MidiInputs {
    _connections: Vec<crate::midi_backend::MidiInputConnection<()>>,
    pub ports: Vec<String>,
}

impl MidiInputs {
    /// Öffnet alle vorhandenen Eingänge und leitet Nachrichten an `sender`.
    pub fn open_all(sender: &MidiSender) -> Result<Self, String> {
        let probe =
            crate::midi_backend::MidiInput::new("StudioCraft").map_err(|e| format!("MIDI: {e}"))?;
        let ports = probe.ports();
        let mut connections = Vec::new();
        let mut names = Vec::new();
        for port in &ports {
            let name = probe.port_name(port).unwrap_or_else(|_| "MIDI".into());
            // Durchschleif-Ports (z. B. „Midi Through“) überspringen.
            if name.to_lowercase().contains("through") {
                continue;
            }
            let Ok(input) = crate::midi_backend::MidiInput::new("StudioCraft") else {
                continue;
            };
            let tx = sender.clone();
            let conn = input.connect(
                port,
                "StudioCraft Eingang",
                move |_, bytes, _| {
                    if let Some(m) = parse(bytes)
                        && let Ok(mut p) = tx.lock()
                    {
                        let _ = p.push(m);
                    }
                },
                (),
            );
            if let Ok(c) = conn {
                connections.push(c);
                names.push(name);
            }
        }
        Ok(Self {
            _connections: connections,
            ports: names,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_notes_controllers_pitchbend() {
        let note = |key, velocity, on| Some(MidiMsg::Note { key, velocity, on });
        assert_eq!(parse(&[0x90, 60, 100]), note(60, 100, true));
        assert_eq!(
            parse(&[0x93, 60, 0]),
            note(60, 0, false),
            "Note-On mit 0 = Off"
        );
        assert_eq!(parse(&[0x80, 64, 40]), note(64, 40, false));
        assert_eq!(
            parse(&[0xb0, 64, 127]),
            Some(MidiMsg::Other([0xb0, 64, 127])),
            "Haltepedal"
        );
        assert_eq!(
            parse(&[0xe0, 0, 64]),
            Some(MidiMsg::Other([0xe0, 0, 64])),
            "Pitchbend Mitte"
        );
        assert_eq!(
            parse(&[0xd0, 90]),
            Some(MidiMsg::Other([0xd0, 90, 0])),
            "Aftertouch"
        );
        assert_eq!(parse(&[0xf8, 0, 0]), None, "Clock wird ignoriert");
        assert_eq!(parse(&[0x90]), None);
    }
}

#[cfg(test)]
mod device_tests {
    #[test]
    #[ignore = "braucht ALSA/CoreMIDI; zeigt nur, was gefunden wird"]
    fn open_all_inputs() {
        let (tx, _rx) = rtrb::RingBuffer::new(16);
        let sender = std::sync::Arc::new(std::sync::Mutex::new(tx));
        let m = super::MidiInputs::open_all(&sender).unwrap();
        println!("MIDI-Eingänge: {:?}", m.ports);
    }
}
