//! Rechte Zone (Spec „Fenster-Architektur“, docs/ui-parity.md „Rechte Zone“, Issue #27):
//! Reiter „Medien“ mit dem Pool des Projekts und „Plugins“ mit den gefundenen CLAP-Plugins.
//! Ein- und ausblenden mit Alt+R oder dem Zonen-Schalter rechts in der Werkzeugzeile.

use egui::Ui;
use studiocraft_plugin_host::PluginInfo;
use studiocraft_project::{ClipId, TrackKind};

use crate::ProjectWindow;

/// Welcher Reiter der rechten Zone offen ist.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub(crate) enum Tab {
    #[default]
    Media,
    Plugins,
}

/// Zustand der rechten Zone.
#[derive(Debug, Clone, Default)]
pub(crate) struct RightZone {
    pub tab: Tab,
    pub filter: String,
}

pub(crate) fn show(w: &mut ProjectWindow, ui: &mut Ui) {
    ui.horizontal(|ui| {
        ui.selectable_value(&mut w.right.tab, Tab::Media, "Medien");
        ui.selectable_value(&mut w.right.tab, Tab::Plugins, "Plugins");
    });
    ui.horizontal(|ui| {
        let l = ui.weak("Suche");
        ui.add(egui::TextEdit::singleline(&mut w.right.filter).desired_width(f32::INFINITY))
            .labelled_by(l.id);
    });
    ui.separator();
    match w.right.tab {
        Tab::Media => media(w, ui),
        Tab::Plugins => plugins(w, ui),
    }
}

fn matches(filter: &str, text: &str) -> bool {
    filter.is_empty() || text.to_lowercase().contains(&filter.to_lowercase())
}

fn media(w: &mut ProjectWindow, ui: &mut Ui) {
    if ui.button("Audio importieren …").clicked() {
        w.import_dialog();
    }
    let target = w.selected_track.filter(|&i| {
        w.project
            .tracks
            .get(i)
            .is_some_and(|t| t.kind == TrackKind::Audio)
    });
    if w.project.pool.is_empty() {
        ui.weak("Der Pool ist leer. Audiodateien importieren oder ins Projekt ziehen.");
        return;
    }
    ui.weak(match target {
        Some(_) => "Doppelklick: an den Cursor der gewählten Audio-Spur",
        None => "Doppelklick: auf eine neue Audio-Spur",
    });
    let mut insert: Option<ClipId> = None;
    egui::ScrollArea::vertical().show(ui, |ui| {
        for (i, clip) in w.project.pool.iter().enumerate() {
            if !matches(&w.right.filter, &clip.name) {
                continue;
            }
            let secs = clip.frames as f64 / f64::from(clip.sample_rate.max(1));
            let text = format!("{}  ·  {:.1} s", clip.name, secs);
            let r = ui.selectable_label(false, text);
            if r.double_clicked()
                && let Ok(id) = ClipId::try_from(i)
            {
                insert = Some(id);
            }
        }
    });
    if let Some(clip) = insert {
        place_clip(w, clip);
    }
}

/// Legt `clip` am Cursor auf die gewählte Audio-Spur, sonst auf eine neue Audio-Spur.
pub(crate) fn place_clip(w: &mut ProjectWindow, clip: ClipId) {
    let before = w.project.clone();
    let track = match w.selected_track.filter(|&i| {
        w.project
            .tracks
            .get(i)
            .is_some_and(|t| t.kind == TrackKind::Audio)
    }) {
        Some(t) => t,
        None => {
            let name = w
                .project
                .pool
                .get(clip as usize)
                .map_or_else(|| "Audio".to_owned(), |c| c.name.clone());
            let t = w.project.add_track(TrackKind::Audio, name);
            w.selected_track = Some(t);
            t
        }
    };
    if let Some(id) = w.project.add_audio_event(track, w.cursor, clip) {
        w.selection = vec![id];
    }
    w.checkpoint(before);
}

fn plugins(w: &mut ProjectWindow, ui: &mut Ui) {
    let filter = w.right.filter.clone();
    let list: Vec<PluginInfo> = w
        .plugin_list()
        .iter()
        .filter(|p| matches(&filter, &p.name) || matches(&filter, &p.vendor))
        .cloned()
        .collect();
    if list.is_empty() {
        ui.weak("Keine passenden CLAP-Plugins gefunden");
        return;
    }
    let target = w.selected_track.filter(|&i| {
        w.project
            .tracks
            .get(i)
            .is_some_and(|t| crate::mixer::has_channel(t.kind))
    });
    let mut action: Option<PluginInfo> = None;
    egui::ScrollArea::vertical().show(ui, |ui| {
        for (title, instruments) in [("Instrumente", true), ("Effekte", false)] {
            egui::CollapsingHeader::new(title)
                .default_open(true)
                .show(ui, |ui| {
                    for p in list.iter().filter(|p| p.is_instrument() == instruments) {
                        let hint = if instruments {
                            "Doppelklick: neue Instrument-Spur mit diesem Plugin"
                        } else if target.is_some() {
                            "Doppelklick: als Insert auf die gewählte Spur"
                        } else {
                            "Erst eine Spur mit Mixer-Kanal wählen"
                        };
                        let label = if p.vendor.is_empty() {
                            p.name.clone()
                        } else {
                            format!("{}  ·  {}", p.name, p.vendor)
                        };
                        if ui
                            .selectable_label(false, label)
                            .on_hover_text(hint)
                            .double_clicked()
                        {
                            action = Some(p.clone());
                        }
                    }
                });
        }
    });
    if let Some(p) = action {
        if p.is_instrument() {
            w.add_instrument_track(Some(p));
        } else if let Some(track) = target {
            w.add_insert(track, &p);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use studiocraft_project::{PoolClip, Project};

    fn window_with_clip() -> ProjectWindow {
        let mut project = Project::default();
        project.pool.push(PoolClip {
            name: "Loop".into(),
            path: None,
            sample_rate: 48_000,
            frames: 48_000,
        });
        ProjectWindow::new(project, None, String::new())
    }

    #[test]
    fn clip_goes_to_the_selected_audio_track_or_a_new_one() {
        let mut w = window_with_clip();
        w.selected_track = None;
        place_clip(&mut w, 0);
        assert_eq!(w.project.tracks.len(), 1);
        assert_eq!(w.project.tracks[0].events.len(), 1);
        w.cursor = 3840;
        place_clip(&mut w, 0);
        assert_eq!(w.project.tracks.len(), 1);
        let starts: Vec<u64> = w.project.tracks[0]
            .events
            .iter()
            .map(|e| e.start.0)
            .collect();
        assert_eq!(starts, vec![0, 3840]);
        w.undo();
        w.undo();
        assert!(w.project.tracks.is_empty());
    }

    #[test]
    fn filter_ignores_case() {
        assert!(matches("", "x"));
        assert!(matches("loo", "Drum Loop"));
        assert!(!matches("bass", "Drum Loop"));
    }
}
