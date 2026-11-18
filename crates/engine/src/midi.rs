//! MIDI-Eingang (Keyboard) über midir: ALSA, CoreMIDI, WinMM.
//!
//! Alle Eingänge werden geöffnet (wie „All MIDI Inputs“). Noten gehen über
//! einen Puffer direkt in den Audio-Thread; die Oberfläche ist nicht beteiligt.

use std::sync::{Arc, Mutex};

/// Eine Note vom Keyboard.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct MidiMsg {
    pub key: u8,
    pub velocity: u8,
    pub on: bool,
}

/// Eine während der Aufnahme gespielte Note mit Projektposition (Frames).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RecordedMidi {
    pub pos: u64,
    pub key: u8,
    pub velocity: u8,
    pub on: bool,
}

/// Wertet eine MIDI-Nachricht aus: Note-On/-Off (Note-On mit Velocity 0 = Off).
pub fn parse(msg: &[u8]) -> Option<MidiMsg> {
    let (&status, rest) = msg.split_first()?;
    let (&key, rest) = rest.split_first()?;
    let velocity = rest.first().copied().unwrap_or(0);
    let key = key & 0x7f;
    let velocity = velocity & 0x7f;
    match status & 0xf0 {
        0x90 if velocity > 0 => Some(MidiMsg {
            key,
            velocity,
            on: true,
        }),
        0x90 | 0x80 => Some(MidiMsg {
            key,
            velocity,
            on: false,
        }),
        _ => None,
    }
}

/// Gemeinsamer Schreibzugang zum Eingangspuffer des Audio-Threads.
pub type MidiSender = Arc<Mutex<rtrb::Producer<MidiMsg>>>;

/// Offene MIDI-Eingänge. Beim Drop werden die Verbindungen geschlossen.
pub struct MidiInputs {
    _connections: Vec<midir::MidiInputConnection<()>>,
    pub ports: Vec<String>,
}

impl MidiInputs {
    /// Öffnet alle vorhandenen Eingänge und leitet Noten an `sender`.
    pub fn open_all(sender: &MidiSender) -> Result<Self, String> {
        let probe = midir::MidiInput::new("StudioCraft").map_err(|e| format!("MIDI: {e}"))?;
        let ports = probe.ports();
        let mut connections = Vec::new();
        let mut names = Vec::new();
        for port in &ports {
            let name = probe.port_name(port).unwrap_or_else(|_| "MIDI".into());
            // Eigene Durchschleif-Ports (z. B. „Midi Through“) überspringen.
            if name.to_lowercase().contains("through") {
                continue;
            }
            let Ok(input) = midir::MidiInput::new("StudioCraft") else {
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
    fn parses_note_messages() {
        assert_eq!(
            parse(&[0x90, 60, 100]),
            Some(MidiMsg {
                key: 60,
                velocity: 100,
                on: true
            })
        );
        assert_eq!(
            parse(&[0x93, 60, 0]),
            Some(MidiMsg {
                key: 60,
                velocity: 0,
                on: false
            }),
            "Note-On mit 0 = Off"
        );
        assert_eq!(
            parse(&[0x80, 64, 40]),
            Some(MidiMsg {
                key: 64,
                velocity: 40,
                on: false
            })
        );
        assert_eq!(
            parse(&[0xb0, 64, 127]),
            None,
            "Controller wird (noch) ignoriert"
        );
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
