//! Offline rendern (Issue #54): eine Spur samt Instrument und Inserts in Audio verwandeln,
//! schneller als Echtzeit. Grundlage für „An Ort und Stelle rendern“ und Einfrieren.
//!
//! Gerendert wird mit einem eigenen Engine-Kern und frischen Plugin-Instanzen, die Zustand
//! und Parameter aus dem Projekt laden. Die Instanzen entstehen im Hauptthread, gerechnet wird
//! in einem eigenen Thread (er ist für die Plugins der Audio-Thread), danach gehen Kern und
//! Kette zurück und werden im Hauptthread freigegeben (CLAP-Thread-Regeln).

use crate::i18n::tr;
use std::collections::HashMap;
use std::sync::Arc;

use studiocraft_engine::{Command, Insert, InsertChain, Song, engine};
use studiocraft_project::{AutoParam, TrackKind};

use crate::ProjectWindow;
use crate::inserts::INSTRUMENT;

/// Nachklang hinter dem Bereich (Hall, Release) in Sekunden.
pub(crate) const TAIL_SECONDS: f64 = 2.0;
/// Blockgröße beim Offline-Rendern.
const BLOCK: usize = 2048;

/// Was gerendert wird.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum RenderTarget {
    /// Eine Spur; bei Gruppen und FX-Kanälen samt allem, was hineinläuft.
    Track(usize),
    /// Der Hauptmix (Summe).
    Main,
}

/// Ein Render-Auftrag.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct RenderSpec {
    pub target: RenderTarget,
    /// Bereich in Ticks.
    pub range: (u64, u64),
    /// Mit Fader, Panorama und EQ des Ziels (beim Hauptmix: mit Summenfader).
    pub with_channel: bool,
    /// Mit den Inserts des Ziels (Instrumente spielen immer).
    pub inserts: bool,
}

impl RenderSpec {
    /// Eine Spur, wie „An Ort und Stelle rendern“ sie braucht.
    pub(crate) fn track(track: usize, range: (u64, u64), with_channel: bool) -> Self {
        Self {
            target: RenderTarget::Track(track),
            range,
            with_channel,
            inserts: true,
        }
    }
}

impl ProjectWindow {
    /// Spuren, die (direkt, über Gruppen oder Sends) in `track` laufen, samt `track`.
    fn feeding(&self, track: usize) -> Vec<usize> {
        let mut set = vec![track];
        loop {
            let ids: Vec<u32> = set.iter().map(|&i| self.project.tracks[i].id).collect();
            let more: Vec<usize> = (0..self.project.tracks.len())
                .filter(|i| !set.contains(i))
                .filter(|&i| {
                    let t = &self.project.tracks[i];
                    t.output.is_some_and(|o| ids.contains(&o))
                        || t.sends.iter().any(|s| ids.contains(&s.target))
                })
                .collect();
            if more.is_empty() {
                return set;
            }
            set.extend(more);
        }
    }

    /// Rendert `spec` als verschachteltes Stereo bei der Projekt-Abtastrate, mit Nachklang.
    pub(crate) fn render_offline(&mut self, spec: RenderSpec) -> Result<Vec<f32>, String> {
        let (a, b) = spec.range;
        if b <= a {
            return Err(crate::i18n::tr("Nichts zu rendern").into());
        }
        // Aktuelle Plugin-Zustände ins Projekt, damit die frischen Instanzen sie laden.
        self.capture_plugin_states();
        let mut project = self.project.clone();
        // Spuren, die klingen; beim Hauptmix alle, sonst das Ziel und seine Quellen.
        let keep: Vec<usize> = match spec.target {
            RenderTarget::Main => (0..project.tracks.len()).collect(),
            RenderTarget::Track(t) if t < project.tracks.len() => self.feeding(t),
            RenderTarget::Track(_) => return Err(crate::i18n::tr("Nichts zu rendern").into()),
        };
        let ids: Vec<u32> = keep.iter().map(|&i| project.tracks[i].id).collect();
        for (i, t) in project.tracks.iter_mut().enumerate() {
            if !keep.contains(&i) {
                t.mute = true;
                continue;
            }
            if spec.target != RenderTarget::Main {
                t.solo = false;
                t.mute = false;
                // Nur Wege, die im Gerenderten bleiben.
                t.sends.retain(|s| ids.contains(&s.target));
            }
        }
        if let RenderTarget::Track(target) = spec.target {
            let t = &mut project.tracks[target];
            t.output = None;
            t.out_bus = None;
            t.sends.clear();
            if !spec.with_channel {
                t.volume_db = 0.0;
                t.pan = 0.0;
                t.eq.on = false;
                t.automation.retain(|l| {
                    !matches!(
                        l.param,
                        AutoParam::Volume | AutoParam::Pan | AutoParam::Mute
                    )
                });
            }
            if !spec.inserts {
                for slot in &mut t.inserts {
                    slot.bypass = true;
                }
            }
            project.master_db = 0.0;
        } else if !spec.with_channel {
            project.master_db = 0.0;
        }
        let samples: HashMap<u32, Arc<[f32]>> = self
            .clips
            .iter()
            .map(|(&id, c)| (id, c.samples.clone()))
            .collect();
        let rate = self.sample_rate;
        let mut song = Song::build(&project, &samples, rate);
        let bus_gains = song.output_gains.clone();
        for t in &mut song.tracks {
            t.monitor = None;
            // Weitere Ausgangsbusse landen im Gerenderten auf der Summe, mit dem Pegel (und
            // Mute) ihres Kanalzugs.
            if let studiocraft_engine::song::Output::Hw(k) = t.output {
                t.output = studiocraft_engine::song::Output::Master;
                t.out_gain = Some(bus_gains.get(k).copied().unwrap_or(1.0));
            }
        }
        // Frische Plugin-Ketten für alle klingenden Spuren.
        let mut chains = Vec::new();
        let mut missing = Vec::new();
        for &i in &keep {
            let pt = project.tracks[i].clone();
            // Eingefrorene Spuren spielen ihren Clip, ohne Plugins.
            if pt.mute || pt.frozen.is_some() {
                continue;
            }
            let mut chain = InsertChain::default();
            if let Some(slot) = pt.instrument.as_ref().filter(|s| !s.bypass) {
                match self.instantiate(slot) {
                    Ok(instance) => {
                        chain.instrument = Some(Insert {
                            instance,
                            bypass: false,
                            model: INSTRUMENT,
                        })
                    }
                    Err(e) => missing.push(e),
                }
            }
            for (k, slot) in pt.inserts.iter().enumerate().filter(|(_, s)| !s.bypass) {
                match self.instantiate(slot) {
                    Ok(instance) => chain.slots.push(Insert {
                        instance,
                        bypass: false,
                        model: k,
                    }),
                    Err(e) => missing.push(e),
                }
            }
            if chain.instrument.is_some() || !chain.slots.is_empty() {
                chains.push((i, chain));
            }
        }
        if !missing.is_empty() {
            self.message = Some(crate::i18n::trf(
                "Rendern ohne: {liste}",
                &[("liste", &missing.join(", "))],
            ));
        }
        let tempo = project.tempo.clone();
        let start = tempo.frames(a, rate);
        let end = tempo.frames(b, rate) + (TAIL_SECONDS * f64::from(rate)) as u64;
        let (mut core, mut ctl) = engine(rate, tempo);
        ctl.send(Command::SetSong(Arc::new(song)));
        let slots: Vec<usize> = chains.iter().map(|(i, _)| *i).collect();
        for (i, chain) in chains {
            ctl.send(Command::SetInserts(i, Some(Box::new(chain))));
        }
        ctl.send(Command::Locate(start));
        ctl.send(Command::Play);
        // Rechnen im eigenen Thread (für die Plugins der Audio-Thread).
        let frames = usize::try_from(end - start).map_err(|e| e.to_string())?;
        let work = move || {
            let mut out = vec![0.0f32; frames * 2];
            for block in out.chunks_mut(BLOCK * 2) {
                core.process(block);
            }
            (core, out)
        };
        #[cfg(not(target_arch = "wasm32"))]
        let (mut core, out) = std::thread::Builder::new()
            .name("studiocraft-render".into())
            .spawn(work)
            .map_err(|e| format!("Render-Thread: {e}"))?
            .join()
            .map_err(|_| crate::i18n::tr("Rendern abgebrochen").to_owned())?;
        // Im Browser gibt es keine Threads (und keine Plugins): direkt hier rechnen.
        #[cfg(target_arch = "wasm32")]
        let (mut core, out) = work();
        // Ketten zurück in den Hauptthread und hier freigeben.
        for i in slots {
            ctl.send(Command::SetInserts(i, None));
        }
        core.process(&mut [0.0; 2]);
        ctl.collect_garbage();
        drop(core);
        Ok(out)
    }

    /// „An Ort und Stelle rendern“: das Ergebnis als neuer Clip auf einer neuen Audio-Spur
    /// unter der Quelle, ab dem Anfang des Bereichs. Ein Undo-Schritt.
    pub(crate) fn render_in_place(&mut self, spec: RenderSpec) {
        let audio = match self.render_offline(spec) {
            Ok(a) => a,
            Err(e) => {
                self.message = Some(e);
                return;
            }
        };
        if audio.iter().all(|s| *s == 0.0) {
            self.message = Some(tr("Rendern: nur Stille").into());
            return;
        }
        let RenderTarget::Track(source) = spec.target else {
            return;
        };
        let before = self.project.clone();
        let src = &self.project.tracks[source];
        let name = crate::i18n::trf("{name} (gerendert)", &[("name", &src.name)]);
        let color = src.color;
        let t = self.project.add_track(TrackKind::Audio, &name);
        // Unter die Quelle legen.
        let new = self.project.tracks.remove(t);
        let at = (source + 1).min(self.project.tracks.len());
        self.project.tracks.insert(at, new);
        self.project.tracks[at].color = color;
        let clip = studiocraft_engine::AudioClip {
            sample_rate: self.sample_rate,
            samples: audio.into(),
        };
        match self.import_clip(&name, None, clip, Some((at, spec.range.0))) {
            Ok(id) => {
                self.selection = vec![id];
                self.selected_track = Some(at);
                self.checkpoint(before);
            }
            Err(e) => {
                self.project = before;
                self.message = Some(e);
            }
        }
    }

    /// Untermenü Bearbeiten → „An Ort und Stelle rendern“ für die gewählte Spur.
    pub(crate) fn render_menu(&mut self, ui: &mut egui::Ui) {
        let target = self.selected_track.filter(|&t| {
            self.project
                .tracks
                .get(t)
                .is_some_and(|t| matches!(t.kind, TrackKind::Audio | TrackKind::Instrument))
        });
        let range = target.and_then(|t| self.render_range(t));
        ui.add_enabled_ui(range.is_some(), |ui| {
            ui.menu_button(tr("An Ort und Stelle rendern"), |ui| {
                let (Some(track), Some(range)) = (target, range) else {
                    return;
                };
                for (label, with_channel) in [
                    (tr("Nur Instrument und Inserts"), false),
                    (tr("Mit Kanaleinstellungen"), true),
                ] {
                    if ui.button(label).clicked() {
                        ui.close();
                        self.render_in_place(RenderSpec::track(track, range, with_channel));
                    }
                }
            });
        });
    }

    /// Bereich zum Rendern: Bereichsauswahl, sonst die gewählten Events dieser Spur, sonst
    /// die ganze Spur (Anfang bis Ende ihres letzten Events).
    pub(crate) fn render_range(&self, track: usize) -> Option<(u64, u64)> {
        if let Some((r0, r1, a, b)) = self.range
            && (r0..=r1).contains(&track)
            && b > a
        {
            return Some((a, b));
        }
        let t = self.project.tracks.get(track)?;
        let sel: Vec<&studiocraft_project::Event> = t
            .events
            .iter()
            .filter(|e| self.selection.contains(&e.id))
            .collect();
        let events: Vec<&studiocraft_project::Event> = if sel.is_empty() {
            t.events.iter().collect()
        } else {
            sel
        };
        let a = events.iter().map(|e| e.start.0).min()?;
        let b = events.iter().map(|e| e.end()).max()?;
        Some((a, b))
    }
}
