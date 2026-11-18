//! Insert-Effekte einer Spur (Spec „Mixer“: Inserts im Kanalzug).

use std::fmt;

use studiocraft_plugin_host::ClapInstance;

pub struct Insert {
    pub instance: ClapInstance,
    pub bypass: bool,
}

/// Die Inserts einer Spur in Signalfluss-Reihenfolge.
#[derive(Default)]
pub struct InsertChain {
    /// Instrument-Spuren: das Instrument vor den Inserts.
    pub instrument: Option<Insert>,
    pub slots: Vec<Insert>,
}

impl fmt::Debug for InsertChain {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_list()
            .entries(self.slots.iter().map(|s| (&s.instance.info.name, s.bypass)))
            .finish()
    }
}

impl InsertChain {
    pub fn is_active(&self) -> bool {
        self.instrument.as_ref().is_some_and(|i| !i.bypass) || self.slots.iter().any(|s| !s.bypass)
    }

    /// Insert `slot`, oder das Instrument bei `usize::MAX`.
    pub fn slot_mut(&mut self, slot: usize) -> Option<&mut Insert> {
        if slot == usize::MAX {
            self.instrument.as_mut()
        } else {
            self.slots.get_mut(slot)
        }
    }

    /// Note an das Instrument (falls vorhanden). Echtzeitsicher.
    pub fn note(&mut self, time: u32, key: u8, velocity: f32, on: bool) {
        if let Some(i) = &mut self.instrument {
            i.instance.note(time, key, velocity, on);
        }
    }

    /// Bearbeitet Stereo (getrennte Kanäle) an Ort und Stelle. Echtzeitsicher.
    pub fn process(&mut self, left: &mut [f32], right: &mut [f32]) {
        if let Some(i) = self.instrument.as_mut().filter(|i| !i.bypass) {
            i.instance.process(left, right);
        }
        for s in self.slots.iter_mut().filter(|s| !s.bypass) {
            s.instance.process(left, right);
        }
    }

    /// Vor der Rückgabe an den Hauptthread (im Audio-Thread aufrufen).
    pub fn stop_processing(&mut self) {
        for s in self.instrument.iter_mut().chain(&mut self.slots) {
            s.instance.stop_processing();
        }
    }
}
