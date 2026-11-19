//! Audio-Anschlüsse (Spec „Routing und Busse“, Issue #23): Ein- und Ausgangsbusse, die auf
//! Hardware-Kanäle des Audiogeräts zeigen. Spuren wählen Busse statt roher Kanalpaare.

use crate::{Project, TrackInput};

/// Ein Ein- oder Ausgangsbus: Mono oder Stereo ab Hardware-Kanal `first` (ab 0).
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct IoBus {
    pub id: u32,
    pub name: String,
    pub stereo: bool,
    pub first: u16,
    /// Kanalzug im Mixer: Pegel in dB (Eingänge vor der Aufnahme, Ausgänge vor dem Gerät).
    /// Beim Hauptmix gilt stattdessen der Ausgangsfader des Projekts.
    #[serde(default, skip_serializing_if = "is_zero_db")]
    pub gain_db: f32,
    /// Stumm (nur Ausgangsbusse).
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub mute: bool,
}

fn is_zero_db(v: &f32) -> bool {
    *v == 0.0
}

impl IoBus {
    /// Kanäle als Text, ab 1 gezählt: „1/2“ oder „3“.
    pub fn ports_label(&self) -> String {
        if self.stereo {
            format!("{}/{}", self.first + 1, self.first + 2)
        } else {
            format!("{}", self.first + 1)
        }
    }

    pub fn as_input(&self) -> TrackInput {
        TrackInput {
            first: self.first,
            stereo: self.stereo,
        }
    }
}

/// Ein- und Ausgangsbusse eines Projekts. `outputs[0]` ist der Hauptmix (Summe).
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
#[serde(default)]
pub struct AudioIo {
    pub inputs: Vec<IoBus>,
    pub outputs: Vec<IoBus>,
}

/// Höchstzahl Busse je Richtung (Ausgangsbusse brauchen vorab angelegte Puffer).
pub const MAX_IO_BUSES: usize = 8;
/// Höchster Hardware-Kanal, auf den ein Bus zeigen kann.
pub const MAX_HW_CHANNEL: u16 = 255;
/// Id des Hauptmix-Busses.
pub const MAIN_OUT: u32 = 1;

impl Default for AudioIo {
    fn default() -> Self {
        Self {
            inputs: vec![IoBus {
                id: 1,
                name: "Eingang 1/2".into(),
                stereo: true,
                first: 0,
                gain_db: 0.0,
                mute: false,
            }],
            outputs: vec![IoBus {
                id: MAIN_OUT,
                name: "Stereo Out".into(),
                stereo: true,
                first: 0,
                gain_db: 0.0,
                mute: false,
            }],
        }
    }
}

impl AudioIo {
    pub fn is_default(&self) -> bool {
        *self == Self::default()
    }

    pub fn input(&self, id: u32) -> Option<&IoBus> {
        self.inputs.iter().find(|b| b.id == id)
    }

    pub fn output(&self, id: u32) -> Option<&IoBus> {
        self.outputs.iter().find(|b| b.id == id)
    }

    fn next_id(list: &[IoBus]) -> u32 {
        list.iter()
            .map(|b| b.id)
            .max()
            .unwrap_or(0)
            .saturating_add(1)
    }

    /// Neuer Bus am nächsten freien Kanal; `None`, wenn die Höchstzahl erreicht ist.
    pub fn add(&mut self, output: bool, stereo: bool) -> Option<u32> {
        let list = if output {
            &mut self.outputs
        } else {
            &mut self.inputs
        };
        if list.len() >= MAX_IO_BUSES {
            return None;
        }
        let first = list
            .iter()
            .map(|b| b.first + if b.stereo { 2 } else { 1 })
            .max()
            .unwrap_or(0)
            .min(MAX_HW_CHANNEL);
        let id = Self::next_id(list);
        let mut bus = IoBus {
            id,
            name: String::new(),
            stereo,
            first,
            gain_db: 0.0,
            mute: false,
        };
        let kind = if output { "Ausgang" } else { "Eingang" };
        bus.name = format!("{kind} {}", bus.ports_label());
        list.push(bus);
        Some(id)
    }

    /// Gültige Werte (fremde Dateien): Hauptmix vorhanden, Ids eindeutig, Anzahl begrenzt.
    pub fn sanitized(mut self) -> Self {
        for list in [&mut self.inputs, &mut self.outputs] {
            list.truncate(MAX_IO_BUSES);
            let mut seen = std::collections::HashSet::new();
            list.retain(|b| b.id != 0 && seen.insert(b.id));
            for b in list.iter_mut() {
                b.first = b.first.min(MAX_HW_CHANNEL);
                b.name = b.name.chars().take(64).collect();
                b.gain_db = if b.gain_db.is_finite() {
                    b.gain_db.clamp(crate::MIN_DB, crate::MAX_DB)
                } else {
                    0.0
                };
            }
        }
        if !self.outputs.iter().any(|b| b.id == MAIN_OUT) {
            self.outputs.insert(0, Self::default().outputs[0].clone());
        }
        // Hauptmix immer vorn.
        if let Some(i) = self.outputs.iter().position(|b| b.id == MAIN_OUT) {
            let main = self.outputs.remove(i);
            self.outputs.insert(0, main);
        }
        self.outputs.truncate(MAX_IO_BUSES);
        self
    }
}

impl Project {
    /// Aufnahme-Eingang einer Spur: der gewählte Eingangsbus, sonst das alte Kanalpaar.
    pub fn input_of(&self, track: usize) -> TrackInput {
        let Some(t) = self.tracks.get(track) else {
            return TrackInput::default();
        };
        t.input_bus
            .and_then(|id| self.io.input(id))
            .map_or(t.input, IoBus::as_input)
    }

    /// Bus entfernen; Spuren, die ihn nutzen, fallen auf den ersten Eingang bzw. die Summe
    /// zurück. Der Hauptmix bleibt.
    pub fn remove_io_bus(&mut self, output: bool, id: u32) -> bool {
        if output && id == MAIN_OUT {
            return false;
        }
        let list = if output {
            &mut self.io.outputs
        } else {
            &mut self.io.inputs
        };
        let before = list.len();
        list.retain(|b| b.id != id);
        if list.len() == before {
            return false;
        }
        for t in &mut self.tracks {
            if output && t.out_bus == Some(id) {
                t.out_bus = None;
            }
            if !output && t.input_bus == Some(id) {
                t.input_bus = None;
            }
        }
        true
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::TrackKind;

    #[test]
    fn bus_strip_level_and_mute_are_saved_and_sanitized() {
        let mut p = Project::default();
        let id = p.io.add(true, true).unwrap_or_default();
        let json = crate::file::to_json(&p);
        assert!(!json.contains("gain_db"), "defaults stay out of the file");
        for b in &mut p.io.outputs {
            b.gain_db = -3.0;
            b.mute = b.id == id;
        }
        let back = crate::file::from_json(&crate::file::to_json(&p)).map(|p| p.io);
        assert_eq!(back.as_ref().ok(), Some(&p.io));
        let mut io = p.io.clone();
        io.outputs[0].gain_db = f32::NAN;
        io.outputs[1].gain_db = 99.0;
        let io = io.sanitized();
        assert_eq!(io.outputs[0].gain_db, 0.0);
        assert_eq!(io.outputs[1].gain_db, crate::MAX_DB);
    }

    #[test]
    fn busses_resolve_inputs_and_removing_falls_back() {
        let mut p = Project::default();
        let t = p.add_track(TrackKind::Audio, "Gesang");
        assert_eq!(
            p.input_of(t),
            TrackInput {
                first: 0,
                stereo: true
            }
        );
        let mono = p.io.add(false, false).unwrap();
        assert_eq!(p.io.input(mono).unwrap().first, 2, "next free channel");
        assert_eq!(p.io.input(mono).unwrap().name, "Eingang 3");
        p.tracks[t].input_bus = Some(mono);
        assert_eq!(
            p.input_of(t),
            TrackInput {
                first: 2,
                stereo: false
            }
        );
        let out = p.io.add(true, true).unwrap();
        p.tracks[t].out_bus = Some(out);
        assert!(!p.remove_io_bus(true, MAIN_OUT), "main mix stays");
        assert!(p.remove_io_bus(true, out));
        assert_eq!(p.tracks[t].out_bus, None);
        assert!(p.remove_io_bus(false, mono));
        assert_eq!(
            p.input_of(t),
            TrackInput {
                first: 0,
                stereo: true
            }
        );
        for _ in 0..20 {
            p.io.add(true, false);
        }
        assert_eq!(p.io.outputs.len(), MAX_IO_BUSES);
    }

    #[test]
    fn hostile_io_is_cleaned() {
        let io = AudioIo {
            inputs: vec![
                IoBus {
                    id: 0,
                    name: "x".into(),
                    stereo: true,
                    first: 9,
                    gain_db: 0.0,
                    mute: false,
                },
                IoBus {
                    id: 2,
                    name: "y".repeat(500),
                    stereo: true,
                    first: u16::MAX,
                    gain_db: 0.0,
                    mute: false,
                },
                IoBus {
                    id: 2,
                    name: "dup".into(),
                    stereo: true,
                    first: 0,
                    gain_db: 0.0,
                    mute: false,
                },
            ],
            outputs: vec![IoBus {
                id: 5,
                name: "A".into(),
                stereo: true,
                first: 2,
                gain_db: 0.0,
                mute: false,
            }],
        }
        .sanitized();
        assert_eq!(io.inputs.len(), 1);
        assert_eq!(io.inputs[0].first, MAX_HW_CHANNEL);
        assert_eq!(io.inputs[0].name.len(), 64);
        assert_eq!(io.outputs[0].id, MAIN_OUT, "main mix restored in front");
        assert_eq!(io.outputs.len(), 2);
    }
}
