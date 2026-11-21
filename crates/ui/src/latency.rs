//! Latenzausgleich in der Oberfläche (Issue #51): Plugin-Latenz je Spur, Verzögerungen an
//! die Engine, Begrenzen bei der Aufnahme und die Anzeige.

use std::collections::HashSet;

use studiocraft_engine::Command;
use studiocraft_engine::delay::{Compensation, DelayBank};
use studiocraft_engine::song::Song;
use studiocraft_project::Track;

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

    /// Latenz des Instruments einer Spur in Frames, 0 ohne Plugin.
    fn instrument_latency(&self, track: usize) -> u32 {
        self.plugins
            .sent
            .get(track)
            .and_then(|s| s.instrument_handle.as_ref())
            .map_or(0, |h| h.latency())
    }

    /// Plugin-Latenz je Spur in Frames: Instrument und eingeschaltete Inserts; wegen der
    /// Begrenzung umgangene zählen nicht.
    pub(crate) fn track_latencies(&self) -> Vec<u32> {
        latencies(
            &self.project.tracks,
            &self.latency.constrained,
            |i, k| self.insert_latency(i, k),
            |i| self.instrument_latency(i),
        )
    }

    /// Latenz einer Spur in Frames (für die Anzeige).
    pub(crate) fn track_latency(&self, track: usize) -> u32 {
        self.track_latencies().get(track).copied().unwrap_or(0)
    }

    /// Begrenzen bei der Aufnahme: Inserts mit hoher Latenz auf aufnahmebereiten oder
    /// mitgehörten Spuren in der Engine umgehen (das Projekt bleibt unverändert); sonst den
    /// Zustand aus dem Projekt wiederherstellen.
    pub(crate) fn sync_constrain(&mut self) {
        let want = if self.latency.constrain {
            constrained(&self.project.tracks, |i, k| self.insert_latency(i, k))
        } else {
            HashSet::new()
        };
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
        let latencies = self.track_latencies();
        self.sync_delays_with(song, &latencies);
    }

    /// Wie [`Self::sync_delays`] mit gegebener Latenz je Spur.
    fn sync_delays_with(&mut self, song: &Song, latencies: &[u32]) {
        let comp = Compensation::compute(song, latencies, self.project.tracks.len());
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

/// Plugin-Latenz je Spur: eingeschaltete Inserts, die nicht wegen der Begrenzung umgangen
/// sind, plus das Instrument, wenn es nicht umgangen ist. `insert(spur, slot)` und
/// `instrument(spur)` liefern die gemeldete Latenz der Plugins.
fn latencies(
    tracks: &[Track],
    constrained: &HashSet<(usize, usize)>,
    insert: impl Fn(usize, usize) -> u32,
    instrument: impl Fn(usize) -> u32,
) -> Vec<u32> {
    tracks
        .iter()
        .enumerate()
        .map(|(i, t)| {
            let inserts = t
                .inserts
                .iter()
                .enumerate()
                .filter(|&(k, slot)| !slot.bypass && !constrained.contains(&(i, k)))
                .fold(0u32, |sum, (k, _)| sum.saturating_add(insert(i, k)));
            let instr = match &t.instrument {
                Some(slot) if !slot.bypass => instrument(i),
                _ => 0,
            };
            inserts.saturating_add(instr)
        })
        .collect()
}

/// Inserts, die das Begrenzen umgeht: eingeschaltete mit mehr als [`CONSTRAIN_FRAMES`]
/// Latenz auf aufnahmebereiten oder mitgehörten Spuren.
fn constrained(tracks: &[Track], insert: impl Fn(usize, usize) -> u32) -> HashSet<(usize, usize)> {
    let mut want = HashSet::new();
    for (i, t) in tracks.iter().enumerate() {
        if !(t.record || t.monitor) {
            continue;
        }
        for (k, slot) in t.inserts.iter().enumerate() {
            if !slot.bypass && insert(i, k) > CONSTRAIN_FRAMES {
                want.insert((i, k));
            }
        }
    }
    want
}

#[cfg(test)]
mod tests {
    use super::*;
    use studiocraft_project::{InsertSlot, Project, SendSlot, TrackKind};

    fn slot(bypass: bool) -> InsertSlot {
        InsertSlot {
            plugin_id: "test".into(),
            name: "Test".into(),
            bypass,
            params: Vec::new(),
            state: None,
            isolated: false,
        }
    }

    /// Latenz je Insert in diesem Test: (Spur + 1) × 100 × (Slot + 1).
    fn fake(i: usize, k: usize) -> u32 {
        ((i + 1) * 100 * (k + 1)) as u32
    }

    #[test]
    fn track_latency_counts_active_inserts_and_the_instrument() {
        let mut p = Project::default();
        let a = p.add_track(TrackKind::Audio, "A");
        let b = p.add_track(TrackKind::Instrument, "B");
        let c = p.add_track(TrackKind::Instrument, "C");
        p.tracks[a].inserts = vec![slot(false), slot(true), slot(false)];
        p.tracks[b].inserts = vec![slot(false)];
        p.tracks[b].instrument = Some(slot(false));
        p.tracks[c].instrument = Some(slot(true));
        let none = HashSet::new();
        let lat = latencies(&p.tracks, &none, fake, |i| 7 * (i as u32 + 1));
        // A: Slot 0 (100) + Slot 2 (300), Slot 1 umgangen; B: Insert 200 + Instrument 14;
        // C: Instrument umgangen.
        assert_eq!(lat, [400, 214, 0]);
        // Wegen der Begrenzung umgangene Inserts zählen nicht.
        let held = HashSet::from([(a, 2)]);
        assert_eq!(latencies(&p.tracks, &held, fake, |_| 0)[a], 100);
        // Riesige gemeldete Latenzen laufen nicht über.
        assert_eq!(
            latencies(&p.tracks, &none, |_, _| u32::MAX, |_| u32::MAX)[b],
            u32::MAX
        );
    }

    #[test]
    fn constraining_takes_only_long_inserts_on_recording_tracks() {
        let mut p = Project::default();
        for name in ["Aufnahme", "Mithören", "Wiedergabe"] {
            let t = p.add_track(TrackKind::Audio, name);
            p.tracks[t].inserts = vec![slot(false), slot(false), slot(true)];
        }
        p.tracks[0].record = true;
        p.tracks[1].monitor = true;
        // Slot 0: 256 Frames (genau die Grenze, bleibt), Slot 1: 512, Slot 2: umgangen.
        let lat = |_: usize, k: usize| [CONSTRAIN_FRAMES, 512, 10_000][k];
        assert_eq!(constrained(&p.tracks, lat), HashSet::from([(0, 1), (1, 1)]));
    }

    /// Die Verzögerungen erreichen die Engine, auch über Gruppe und Send; unverändert wird
    /// nichts neu geschickt, ohne Latenz ist der Ausgleich aus.
    #[test]
    fn delays_reach_the_engine_through_groups_and_sends() {
        let (mut core, ctl) =
            studiocraft_engine::engine(48_000, studiocraft_project::TempoMap::default());
        let mut w = ProjectWindow::new(Project::default(), Some(ctl), String::new());
        let group = w.project.add_track(TrackKind::Group, "Gruppe");
        let fx = w.project.add_track(TrackKind::Fx, "Hall");
        let a = w.project.add_track(TrackKind::Audio, "In der Gruppe");
        let b = w.project.add_track(TrackKind::Audio, "Mit Send");
        w.project.tracks[a].parent = Some(w.project.tracks[group].id);
        let fx_id = w.project.tracks[fx].id;
        w.project.tracks[b].sends.push(SendSlot {
            target: fx_id,
            level_db: -6.0,
            pre_fader: false,
            on: true,
        });
        for t in [a, b] {
            let clip = studiocraft_engine::AudioClip {
                sample_rate: 48_000,
                samples: vec![0.1f32; 9600].into(),
            };
            w.import_clip("Take", None, clip, Some((t, 0))).unwrap();
        }
        w.rebuild_song();
        let samples: std::collections::HashMap<_, _> = w
            .clips
            .iter()
            .map(|(&id, c)| (id, c.samples.clone()))
            .collect();
        let song = Song::build(&w.project, &samples, 48_000);
        let mut buf = vec![0.0f32; 512];

        let mut lat = vec![0; w.project.tracks.len()];
        lat[a] = 480;
        w.sync_delays_with(&song, &lat);
        core.process(&mut buf);
        assert_eq!(w.latency.sent.total, 480);
        assert_eq!(w.transport.as_ref().unwrap().status.latency(), 480);
        // Die Spur ohne Latenz wartet auf die mit Latenz: insgesamt kommt sie gleich spät an.
        let d = &w.latency.sent.tracks[b];
        assert_eq!(d.common + d.direct, 480, "{d:?}");
        let before = w.latency.sent.clone();
        w.sync_delays_with(&song, &lat);
        assert_eq!(w.latency.sent, before);

        w.sync_delays_with(&song, &vec![0; lat.len()]);
        core.process(&mut buf);
        assert_eq!(w.latency.sent.total, 0);
        assert_eq!(w.transport.as_ref().unwrap().status.latency(), 0);
    }

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
