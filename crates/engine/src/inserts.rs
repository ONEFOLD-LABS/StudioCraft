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
        self.slots.iter().any(|s| !s.bypass)
    }

    /// Bearbeitet Stereo (getrennte Kanäle) an Ort und Stelle. Echtzeitsicher.
    pub fn process(&mut self, left: &mut [f32], right: &mut [f32]) {
        for s in self.slots.iter_mut().filter(|s| !s.bypass) {
            s.instance.process(left, right);
        }
    }

    /// Vor der Rückgabe an den Hauptthread (im Audio-Thread aufrufen).
    pub fn stop_processing(&mut self) {
        for s in &mut self.slots {
            s.instance.stop_processing();
        }
    }
}
