//! Spuren einfrieren (Issue #54): Instrument und Inserts einer Spur offline in einen Clip
//! rendern und die Plugins entladen. Kanal (Fader, Panorama, EQ, Sends, Automation von
//! Lautstärke und Panorama) bleibt live. „Einfrieren aufheben“ lädt die Plugins wieder aus
//! ihrem gespeicherten Zustand; der Clip bleibt im Pool.

use crate::i18n::tr;
use studiocraft_engine::AudioClip;
use studiocraft_project::{Frozen, PoolClip, Project, TrackKind};

use crate::render::{RenderSpec, RenderTarget};
use crate::waveform::Peaks;
use crate::{LoadedClip, ProjectWindow};

impl ProjectWindow {
    /// Spur lässt sich einfrieren: Audio- oder Instrumentspur mit Events und Plugins.
    pub(crate) fn can_freeze(&self, track: usize) -> bool {
        self.project.tracks.get(track).is_some_and(|t| {
            matches!(t.kind, TrackKind::Audio | TrackKind::Instrument)
                && t.frozen.is_none()
                && !t.events.is_empty()
                && (t.instrument.is_some() || !t.inserts.is_empty())
        })
    }

    /// Friert `track` ein: rendert Instrument, Pre-Sektion und Inserts ohne Kanal und
    /// spielt danach nur noch den Clip. Rückgängig machbar.
    pub(crate) fn freeze_track(&mut self, track: usize) {
        if !self.can_freeze(track) {
            return;
        }
        let t = &self.project.tracks[track];
        let (Some(a), Some(b)) = (
            t.events.iter().map(|e| e.start.0).min(),
            t.events.iter().map(|e| e.end()).max(),
        ) else {
            return;
        };
        let name = crate::i18n::trf("{name} (eingefroren)", &[("name", &t.name)]);
        // Plugin-Latenz: der gerenderte Clip käme um so viel zu spät.
        let latency = self.track_latency(track) as usize;
        let mut audio = match self.render_offline(RenderSpec {
            target: RenderTarget::Track(track),
            range: (a, b),
            with_channel: false,
            inserts: true,
        }) {
            Ok(audio) => audio,
            Err(e) => {
                self.message = Some(e);
                return;
            }
        };
        audio.drain(..latency.saturating_mul(2).min(audio.len()));
        let clip = AudioClip {
            sample_rate: self.sample_rate,
            samples: audio.into(),
        };
        let before = self.project.clone();
        let id = self.project.add_clip(PoolClip {
            name,
            path: None,
            sample_rate: clip.sample_rate,
            frames: clip.frames(),
        });
        self.clips.insert(
            id,
            LoadedClip {
                peaks: Peaks::new(&clip.samples),
                samples: clip.samples,
            },
        );
        let events: Vec<_> = self.project.tracks[track]
            .events
            .iter()
            .map(|e| e.id)
            .collect();
        self.selection.retain(|e| !events.contains(e));
        self.project.tracks[track].frozen = Some(Frozen { clip: id, start: a });
        self.checkpoint(before);
    }

    /// Hebt das Einfrieren auf: Events und Plugins gelten wieder.
    pub(crate) fn unfreeze_track(&mut self, track: usize) {
        let Some(t) = self.project.tracks.get(track) else {
            return;
        };
        if t.frozen.is_none() {
            return;
        }
        let before = self.project.clone();
        self.project.tracks[track].frozen = None;
        self.checkpoint(before);
    }

    /// Sperre: Events, Inserts und Instrument eingefrorener Spuren bleiben, wie sie in
    /// `before` waren (aus [`ProjectWindow::checkpoint`] aufgerufen).
    pub(crate) fn keep_frozen(&mut self, before: &Project) {
        let mut locked = false;
        for t in &mut self.project.tracks {
            let Some(old) = before.tracks.iter().find(|o| o.id == t.id) else {
                continue;
            };
            if t.frozen.is_none() || t.frozen != old.frozen {
                continue;
            }
            if t.events != old.events {
                t.events.clone_from(&old.events);
                locked = true;
            }
            if t.inserts != old.inserts {
                t.inserts.clone_from(&old.inserts);
                locked = true;
            }
            if t.instrument != old.instrument {
                t.instrument.clone_from(&old.instrument);
                locked = true;
            }
        }
        if locked {
            self.message = Some(tr("Spur ist eingefroren – erst „Einfrieren aufheben“").into());
        }
    }

    /// Einträge im Menü Bearbeiten für die gewählte Spur.
    pub(crate) fn freeze_menu(&mut self, ui: &mut egui::Ui) {
        let track = self.selected_track;
        let frozen = track
            .and_then(|t| self.project.tracks.get(t))
            .is_some_and(|t| t.frozen.is_some());
        if frozen {
            if ui.button(tr("Einfrieren aufheben")).clicked() {
                ui.close();
                if let Some(t) = track {
                    self.unfreeze_track(t);
                }
            }
        } else {
            let can = track.is_some_and(|t| self.can_freeze(t));
            if ui
                .add_enabled(can, egui::Button::new(tr("Spur einfrieren")))
                .clicked()
            {
                ui.close();
                if let Some(t) = track {
                    self.freeze_track(t);
                }
            }
        }
    }
}
