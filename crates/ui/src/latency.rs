//! Latenzausgleich in der Oberfläche (Issue #51): Plugin-Latenz je Spur, Verzögerungen an
//! die Engine, Begrenzen bei der Aufnahme und die Anzeige.

use std::collections::HashSet;

use studiocraft_engine::Command;
use studiocraft_engine::delay::{Compensation, DelayBank};
use studiocraft_engine::song::Song;

use crate::ProjectWindow;

/// Begrenzter Latenzausgleich: Inserts mit mehr Latenz (Frames) werden auf aufnahmebereiten
/// und mitgehörten Spuren umgangen.
pub(crate) const CONSTRAIN_FRAMES: u32 = 256;

/// Zustand des Latenzausgleichs im Fenster.
#[derive(Default)]
pub(crate) struct LatencyState {
    /// Zuletzt an die Engine gegebene Verzögerungen.
    pub sent: Compensation,
    /// „Latenzausgleich begrenzen“ (Aufnahme).
    pub constrain: bool,
    /// Inserts, die wegen der Begrenzung in der Engine umgangen sind: (Spur, Modell-Slot).
    pub constrained: HashSet<(usize, usize)>,
}

impl ProjectWindow {
    /// Latenz eines Inserts (Modell-Slot) in Frames, 0 ohne Plugin.
    fn insert_latency(&self, track: usize, slot: usize) -> u32 {
        self.plugins
            .sent
            .get(track)
            .and_then(|s| s.handles.get(slot))
            .and_then(Option::as_ref)
            .map_or(0, |h| h.latency())
    }

    /// Plugin-Latenz je Spur in Frames: Instrument und eingeschaltete Inserts; wegen der
    /// Begrenzung umgangene zählen nicht.
    pub(crate) fn track_latencies(&self) -> Vec<u32> {
        (0..self.project.tracks.len())
            .map(|i| {
                let t = &self.project.tracks[i];
                let inserts: u32 = (0..t.inserts.len())
                    .filter(|&k| {
                        !t.inserts[k].bypass && !self.latency.constrained.contains(&(i, k))
                    })
                    .map(|k| self.insert_latency(i, k))
                    .sum();
                let instrument = match (&t.instrument, self.plugins.sent.get(i)) {
                    (Some(slot), Some(sent)) if !slot.bypass => {
                        sent.instrument_handle.as_ref().map_or(0, |h| h.latency())
                    }
                    _ => 0,
                };
                inserts.saturating_add(instrument)
            })
            .collect()
    }

    /// Latenz einer Spur in Frames (für die Anzeige).
    pub(crate) fn track_latency(&self, track: usize) -> u32 {
        self.track_latencies().get(track).copied().unwrap_or(0)
    }

    /// Begrenzen bei der Aufnahme: Inserts mit hoher Latenz auf aufnahmebereiten oder
    /// mitgehörten Spuren in der Engine umgehen (das Projekt bleibt unverändert); sonst den
    /// Zustand aus dem Projekt wiederherstellen.
    pub(crate) fn sync_constrain(&mut self) {
        let mut want = HashSet::new();
        if self.latency.constrain {
            for (i, t) in self.project.tracks.iter().enumerate() {
                if !(t.record || t.monitor) {
                    continue;
                }
                for (k, slot) in t.inserts.iter().enumerate() {
                    if !slot.bypass && self.insert_latency(i, k) > CONSTRAIN_FRAMES {
                        want.insert((i, k));
                    }
                }
            }
        }
        if want == self.latency.constrained {
            return;
        }
        let mut cmds = Vec::new();
        for &(i, k) in want.symmetric_difference(&self.latency.constrained) {
            let Some(idx) = self
                .plugins
                .sent
                .get(i)
                .and_then(|s| s.map.get(k).copied().flatten())
            else {
                continue;
            };
            let model = self.project.tracks.get(i).and_then(|t| t.inserts.get(k));
            let bypass = want.contains(&(i, k)) || model.is_some_and(|s| s.bypass);
            cmds.push(Command::SetBypass(i, idx, bypass));
        }
        for c in cmds {
            self.send(c);
        }
        self.latency.constrained = want;
    }

    /// Verzögerungen für `song` berechnen und an die Engine geben, wenn sie sich geändert
    /// haben (die Leitungen werden hier, im Hauptthread, angelegt).
    pub(crate) fn sync_delays(&mut self, song: &Song) {
        let comp = Compensation::compute(song, &self.track_latencies(), self.project.tracks.len());
        if comp == self.latency.sent {
            return;
        }
        let bank = Box::new(DelayBank::new(&comp));
        let total = comp.total;
        self.latency.sent = comp;
        self.send(Command::SetDelays(bank, total));
    }

    /// Je Frame: Hat ein Plugin eine neue Latenz gemeldet, alles neu berechnen.
    pub(crate) fn poll_latency(&mut self) {
        let mut changed = false;
        for sent in &self.plugins.sent {
            for h in sent
                .handles
                .iter()
                .flatten()
                .chain(sent.instrument_handle.iter())
            {
                changed |= h.take_latency_changed();
            }
        }
        if changed {
            self.dirty = true;
        }
    }

    /// Latenz in Frames als Text mit Millisekunden.
    pub(crate) fn latency_text(&self, frames: u32) -> String {
        let ms = f64::from(frames) * 1000.0 / f64::from(self.sample_rate.max(1));
        format!("{frames} Samples ({ms:.1} ms)")
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use studiocraft_project::{InsertSlot, Project, TrackKind};

    /// Mit einem installierten Plugin mit Latenz (ZaMaximX2 meldet 480 Samples): Die Spur
    /// zeigt ihre Latenz, die Engine bekommt den Ausgleich, Begrenzen nimmt das Plugin auf
    /// einer aufnahmebereiten Spur aus dem Ausgleich. Ohne das Plugin übersprungen.
    #[test]
    fn a_plugin_with_latency_drives_the_compensation() {
        let (mut core, ctl) =
            studiocraft_engine::engine(48_000, studiocraft_project::TempoMap::default());
        let mut w = ProjectWindow::new(Project::default(), Some(ctl), String::new());
        let Some(info) = w
            .plugin_list()
            .iter()
            .find(|p| p.name == "ZaMaximX2")
            .cloned()
        else {
            eprintln!("ZaMaximX2 nicht installiert: übersprungen");
            return;
        };
        let a = w.project.add_track(TrackKind::Audio, "Mit Plugin");
        let b = w.project.add_track(TrackKind::Audio, "Ohne");
        // Beide Spuren mit Inhalt (Spuren ohne Events sind nicht im Song).
        for t in [a, b] {
            let clip = studiocraft_engine::AudioClip {
                sample_rate: 48_000,
                samples: vec![0.1f32; 9600].into(),
            };
            w.import_clip("Take", None, clip, Some((t, 0))).unwrap();
        }
        w.project.tracks[a].inserts.push(InsertSlot {
            plugin_id: info.id.clone(),
            name: info.name.clone(),
            bypass: false,
            params: Vec::new(),
            state: None,
            isolated: false,
        });
        w.sync_inserts();
        w.rebuild_song();
        let mut buf = vec![0.0f32; 512];
        core.process(&mut buf);
        let lat = w.track_latency(a);
        assert!(lat > 0, "plugin reports latency");
        assert_eq!(w.latency.sent.total, lat);
        assert_eq!(
            w.latency.sent.tracks[b].common, lat,
            "the other track waits"
        );
        assert_eq!(w.transport.as_ref().unwrap().status.latency(), lat);
        // Constrain while recording: the plugin is bypassed and leaves the compensation.
        w.project.tracks[a].record = true;
        w.latency.constrain = lat > CONSTRAIN_FRAMES;
        w.rebuild_song();
        core.process(&mut buf);
        if lat > CONSTRAIN_FRAMES {
            assert!(w.latency.constrained.contains(&(a, 0)));
            assert_eq!(w.latency.sent.total, 0);
            assert!(
                !w.project.tracks[a].inserts[0].bypass,
                "the project is unchanged"
            );
        }
        // Off again: back to full compensation.
        w.latency.constrain = false;
        w.rebuild_song();
        core.process(&mut buf);
        assert!(w.latency.constrained.is_empty());
        assert_eq!(w.latency.sent.total, lat);
    }
}
