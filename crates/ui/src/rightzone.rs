//! Rechte Zone (Spec „Fenster-Architektur“, docs/ui-parity.md „Rechte Zone“):
//! Reiter „Start“ mit großen Kacheln, die in die anderen Reiter springen,
//! „Medien“ mit dem Pool des Projekts, „Dateien“ als Dateibrowser mit Vorhören und
//! „Plugins“ mit den gefundenen CLAP-Plugins. Ein- und ausblenden mit Alt+R oder dem
//! Zonen-Schalter rechts in der Werkzeugzeile.
//!
//! Dateibrowser: Doppelklick öffnet einen Ordner bzw. legt eine Audiodatei an den Cursor der
//! gewählten Audio-Spur (sonst auf eine neue Spur); Dateien lassen sich auch in die
//! Event-Anzeige ziehen. Ein Klick hört die Datei vor (abschaltbar); geladen wird in einem
//! eigenen Thread, damit die Oberfläche nicht hängt.

use std::path::{Path, PathBuf};
use std::sync::{Arc, mpsc};

use egui::{Sense, Ui};
use studiocraft_engine::{AudioClip, Command};
use studiocraft_plugin_host::PluginInfo;
use studiocraft_project::{ClipId, TrackKind};

use crate::ProjectWindow;

/// Welcher Reiter der rechten Zone offen ist.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub(crate) enum Tab {
    /// Startseite mit Kacheln.
    #[default]
    Home,
    Media,
    Files,
    Plugins,
}

/// Welche Plugins der Reiter „Plugins“ zeigt.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub(crate) enum PluginKind {
    #[default]
    All,
    Instruments,
    Effects,
}

/// Eine Datei, die aus dem Browser in die Event-Anzeige gezogen wird.
#[derive(Debug, Clone)]
pub(crate) struct BrowserFile(pub PathBuf);

/// Ein Plugin, das aus der rechten Zone in die Spurliste oder Event-Anzeige gezogen wird.
#[derive(Debug, Clone)]
pub(crate) struct BrowserPlugin(pub PluginInfo);

/// Ein Eintrag im Dateibrowser.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Entry {
    pub path: PathBuf,
    pub name: String,
    pub dir: bool,
}

/// Ergebnis eines Ladevorgangs fürs Vorhören.
type Loaded = Result<Arc<[f32]>, String>;

/// Zustand der rechten Zone.
#[derive(Debug)]
pub(crate) struct RightZone {
    pub tab: Tab,
    /// Filter des Reiters „Plugins“ (von den Kacheln „Instrumente“ und „Effekte“ gesetzt).
    pub plugin_kind: PluginKind,
    pub filter: String,
    /// Dateibrowser: aktueller Ordner (beim ersten Öffnen gesetzt), Inhalt, Auswahl.
    pub dir: Option<PathBuf>,
    pub entries: Vec<Entry>,
    pub selected: Option<PathBuf>,
    /// Ein Klick auf eine Datei hört sie vor.
    pub auto_preview: bool,
    pub error: Option<String>,
    loading: Option<mpsc::Receiver<Loaded>>,
}

impl Default for RightZone {
    fn default() -> Self {
        Self {
            tab: Tab::default(),
            plugin_kind: PluginKind::default(),
            filter: String::new(),
            dir: None,
            entries: Vec::new(),
            selected: None,
            auto_preview: true,
            error: None,
            loading: None,
        }
    }
}

/// Ordner und Audiodateien in `dir`: Ordner zuerst, je nach Name sortiert, versteckte weg.
pub(crate) fn list_dir(dir: &Path) -> Result<Vec<Entry>, String> {
    let exts = AudioClip::extensions();
    let read = std::fs::read_dir(dir).map_err(|e| format!("{}: {e}", dir.display()))?;
    let mut out: Vec<Entry> = read
        .filter_map(Result::ok)
        .filter_map(|e| {
            let name = e.file_name().to_string_lossy().into_owned();
            if name.starts_with('.') {
                return None;
            }
            let path = e.path();
            let dir = path.is_dir();
            let audio = path
                .extension()
                .and_then(|x| x.to_str())
                .is_some_and(|x| exts.iter().any(|k| k.eq_ignore_ascii_case(x)));
            (dir || audio).then_some(Entry { path, name, dir })
        })
        .collect();
    out.sort_by_key(|e| (!e.dir, e.name.to_lowercase()));
    Ok(out)
}

/// Startordner: neben dem Projekt, sonst das Home-Verzeichnis, sonst das Arbeitsverzeichnis.
fn start_dir(w: &ProjectWindow) -> PathBuf {
    w.project_path()
        .and_then(Path::parent)
        .map(Path::to_path_buf)
        .or_else(|| {
            std::env::var_os("HOME")
                .or_else(|| std::env::var_os("USERPROFILE"))
                .map(PathBuf::from)
        })
        .or_else(|| std::env::current_dir().ok())
        .unwrap_or_else(|| PathBuf::from("."))
}

pub(crate) fn show(w: &mut ProjectWindow, ui: &mut Ui) {
    ui.horizontal(|ui| {
        ui.selectable_value(&mut w.right.tab, Tab::Home, "Start");
        ui.selectable_value(&mut w.right.tab, Tab::Media, "Medien");
        ui.selectable_value(&mut w.right.tab, Tab::Files, "Dateien");
        ui.selectable_value(&mut w.right.tab, Tab::Plugins, "Plugins");
    });
    if w.right.tab != Tab::Home {
        ui.horizontal(|ui| {
            let l = ui.weak("Suche");
            ui.add(egui::TextEdit::singleline(&mut w.right.filter).desired_width(f32::INFINITY))
                .labelled_by(l.id);
        });
    }
    ui.separator();
    match w.right.tab {
        Tab::Home => home(w, ui),
        Tab::Media => media(w, ui),
        Tab::Files => files(w, ui),
        Tab::Plugins => plugins(w, ui),
    }
    poll_preview(w, ui);
}

/// Startseite: große Kacheln, je zwei nebeneinander. Ein Klick öffnet den passenden Reiter;
/// „Instrumente“ und „Effekte“ zeigen die Plugins schon gefiltert.
fn home(w: &mut ProjectWindow, ui: &mut Ui) {
    use crate::icons::Icon;
    let tiles = [
        (
            Icon::Instruments,
            "Instrumente",
            "Instrument-Plugins: neue Instrument-Spur anlegen",
        ),
        (
            Icon::Effects,
            "Effekte",
            "Effekt-Plugins: als Insert auf eine Spur legen",
        ),
        (Icon::Media, "Medien", "Audio-Dateien des Projekts (Pool)"),
        (Icon::Folder, "Dateien", "Dateibrowser mit Vorhören"),
    ];
    let mut picked = None;
    let size = egui::vec2(84.0, 84.0);
    ui.add_space(6.0);
    for row in tiles.chunks(2) {
        ui.horizontal(|ui| {
            ui.spacing_mut().item_spacing.x = 10.0;
            for &(icon, label, hint) in row {
                if tile(ui, size, icon, label).on_hover_text(hint).clicked() {
                    picked = Some(label);
                }
            }
        });
        ui.add_space(8.0);
    }
    match picked {
        Some("Instrumente") => {
            w.right.tab = Tab::Plugins;
            w.right.plugin_kind = PluginKind::Instruments;
        }
        Some("Effekte") => {
            w.right.tab = Tab::Plugins;
            w.right.plugin_kind = PluginKind::Effects;
        }
        Some("Medien") => w.right.tab = Tab::Media,
        Some("Dateien") => w.right.tab = Tab::Files,
        _ => {}
    }
}

/// Eine Kachel: großes Symbol auf hinterlegter Fläche, darunter die Beschriftung.
fn tile(ui: &mut Ui, size: egui::Vec2, icon: crate::icons::Icon, label: &str) -> egui::Response {
    let t = crate::theme::tokens(ui.ctx());
    let (rect, resp) = ui.allocate_exact_size(size, Sense::click());
    resp.widget_info(|| egui::WidgetInfo::labeled(egui::Role::Button, true, label));
    if ui.is_rect_visible(rect) {
        let p = ui.painter();
        let bg = if resp.hovered() {
            t.button_hi
        } else {
            t.button
        };
        p.rect_filled(rect, 6.0, bg);
        let icon_rect = egui::Rect::from_center_size(
            rect.center() - egui::vec2(0.0, 9.0),
            egui::vec2(40.0, 40.0),
        );
        crate::icons::paint(p, icon_rect, icon, t.text);
        p.text(
            rect.center_bottom() - egui::vec2(0.0, 10.0),
            egui::Align2::CENTER_CENTER,
            label,
            egui::FontId::proportional(12.0),
            t.text,
        );
    }
    resp
}

/// Wechselt in `dir` und liest es ein.
pub(crate) fn open_dir(w: &mut ProjectWindow, dir: PathBuf) {
    match list_dir(&dir) {
        Ok(entries) => {
            w.right.entries = entries;
            w.right.error = None;
            w.right.dir = Some(dir);
        }
        Err(e) => w.right.error = Some(e),
    }
}

fn files(w: &mut ProjectWindow, ui: &mut Ui) {
    if w.right.dir.is_none() {
        let dir = start_dir(w);
        open_dir(w, dir);
    }
    let Some(dir) = w.right.dir.clone() else {
        if let Some(e) = &w.right.error {
            ui.colored_label(ui.visuals().warn_fg_color, e);
        }
        return;
    };
    ui.horizontal(|ui| {
        let up = dir.parent().map(Path::to_path_buf);
        if ui
            .add_enabled(up.is_some(), egui::Button::new(".."))
            .on_hover_text("Übergeordneter Ordner")
            .clicked()
            && let Some(up) = up
        {
            open_dir(w, up);
        }
        if ui
            .button("Neu laden")
            .on_hover_text("Ordner neu einlesen")
            .clicked()
        {
            open_dir(w, dir.clone());
        }
        ui.add(egui::Label::new(dir.display().to_string()).truncate())
            .on_hover_text(dir.display().to_string());
    });
    ui.horizontal(|ui| {
        let previewing = w.transport.as_ref().is_some_and(|t| t.status.previewing())
            || w.right.loading.is_some();
        use crate::icons::{Icon, icon_button};
        if previewing {
            if icon_button(ui, Icon::Stop, false, None, "Vorhören stoppen").clicked() {
                stop_preview(w);
            }
        } else if icon_button(ui, Icon::Start, false, None, "Vorhören").clicked()
            && let Some(p) = w.right.selected.clone()
        {
            start_preview(w, &p);
        }
        ui.checkbox(&mut w.right.auto_preview, "Automatisch");
    });
    if let Some(e) = &w.right.error {
        ui.colored_label(ui.visuals().warn_fg_color, e);
    }
    ui.separator();
    let mut enter: Option<PathBuf> = None;
    let mut select: Option<PathBuf> = None;
    let mut place: Option<PathBuf> = None;
    let filter = w.right.filter.clone();
    egui::ScrollArea::vertical().show(ui, |ui| {
        for e in w.right.entries.iter().filter(|e| matches(&filter, &e.name)) {
            let text = if e.dir {
                format!("{}/", e.name)
            } else {
                e.name.clone()
            };
            let selected = w.right.selected.as_ref() == Some(&e.path);
            let r = ui.selectable_label(selected, text);
            if e.dir {
                if r.double_clicked() {
                    enter = Some(e.path.clone());
                }
                continue;
            }
            // Ziehen in die Event-Anzeige.
            let r = r.interact(Sense::drag());
            r.dnd_set_drag_payload(BrowserFile(e.path.clone()));
            if r.double_clicked() {
                place = Some(e.path.clone());
            } else if r.clicked() {
                select = Some(e.path.clone());
            }
        }
    });
    if let Some(d) = enter {
        open_dir(w, d);
    }
    if let Some(p) = select {
        w.right.selected = Some(p.clone());
        if w.right.auto_preview {
            start_preview(w, &p);
        }
    }
    if let Some(p) = place {
        stop_preview(w);
        place_file(w, &p);
    }
}

/// Legt eine Audiodatei an den Cursor der gewählten Audio-Spur, sonst auf eine neue Spur.
pub(crate) fn place_file(w: &mut ProjectWindow, path: &Path) {
    let before = w.project.clone();
    let target = w
        .selected_track
        .filter(|&i| {
            w.project
                .tracks
                .get(i)
                .is_some_and(|t| t.kind == TrackKind::Audio)
        })
        .map_or((usize::MAX, w.cursor), |t| (t, w.cursor));
    match w.import_audio(path, Some(target)) {
        Ok(id) => w.selection = vec![id],
        Err(e) => w.message = Some(e),
    }
    w.checkpoint(before);
}

/// Lädt `path` in einem eigenen Thread und hört es danach vor.
fn start_preview(w: &mut ProjectWindow, path: &Path) {
    let Some(t) = &w.transport else {
        return;
    };
    let rate = t.sample_rate;
    let path = path.to_path_buf();
    let (tx, rx) = mpsc::channel();
    let spawned = std::thread::Builder::new()
        .name("vorhoeren".into())
        .spawn(move || {
            let r = AudioClip::load(&path)
                .map(|c| c.resampled(rate).samples)
                .map_err(|e| format!("{}: {e}", path.display()));
            let _ = tx.send(r);
        });
    match spawned {
        Ok(_) => w.right.loading = Some(rx),
        Err(e) => w.right.error = Some(format!("Vorhören: {e}")),
    }
}

fn stop_preview(w: &mut ProjectWindow) {
    w.right.loading = None;
    w.send(Command::Preview(None));
}

/// Fertig geladene Datei an die Engine geben.
fn poll_preview(w: &mut ProjectWindow, ui: &Ui) {
    let Some(rx) = &w.right.loading else {
        return;
    };
    match rx.try_recv() {
        Ok(Ok(samples)) => {
            w.right.loading = None;
            w.right.error = None;
            w.send(Command::Preview(Some(samples)));
        }
        Ok(Err(e)) => {
            w.right.loading = None;
            w.right.error = Some(e);
        }
        Err(mpsc::TryRecvError::Empty) => ui.ctx().request_repaint(),
        Err(mpsc::TryRecvError::Disconnected) => w.right.loading = None,
    }
    if w.transport.as_ref().is_some_and(|t| t.status.previewing()) {
        ui.ctx()
            .request_repaint_after(std::time::Duration::from_millis(100));
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
    ui.horizontal(|ui| {
        let k = &mut w.right.plugin_kind;
        ui.selectable_value(k, PluginKind::All, "Alle");
        ui.selectable_value(k, PluginKind::Instruments, "Instrumente");
        ui.selectable_value(k, PluginKind::Effects, "Effekte");
    });
    let kind = w.right.plugin_kind;
    let mut action: Option<PluginInfo> = None;
    egui::ScrollArea::vertical().show(ui, |ui| {
        for (title, instruments) in [("Instrumente", true), ("Effekte", false)] {
            let shown = match kind {
                PluginKind::All => true,
                PluginKind::Instruments => instruments,
                PluginKind::Effects => !instruments,
            };
            if !shown {
                continue;
            }
            egui::CollapsingHeader::new(title)
                .default_open(true)
                .show(ui, |ui| {
                    for p in list.iter().filter(|p| p.is_instrument() == instruments) {
                        let hint = if instruments {
                            "Doppelklick oder auf eine leere Fläche ziehen: neue Instrument-Spur"
                        } else if target.is_some() {
                            "Doppelklick: als Insert auf die gewählte Spur; oder auf eine Spur ziehen"
                        } else {
                            "Auf eine Spur ziehen oder erst eine Spur mit Mixer-Kanal wählen"
                        };
                        let label = if p.vendor.is_empty() {
                            p.name.clone()
                        } else {
                            format!("{}  ·  {}", p.name, p.vendor)
                        };
                        let r = ui
                            .selectable_label(false, label)
                            .on_hover_text(hint)
                            .interact(Sense::drag());
                        // Ziehen: Instrument auf eine leere Fläche oder Instrument-Spur,
                        // Effekt auf eine Spur mit Mixer-Kanal.
                        r.dnd_set_drag_payload(BrowserPlugin(p.clone()));
                        if r.double_clicked() {
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

    /// Leerer Testordner unter dem Systemordner für temporäre Dateien.
    fn test_dir(name: &str) -> PathBuf {
        let d =
            std::env::temp_dir().join(format!("studiocraft-browser-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(&d).expect("test dir");
        d
    }

    /// Kurze Mono-WAV-Datei (16 Bit, 48 kHz, 4800 Frames).
    fn write_wav(path: &Path) {
        let frames: u32 = 4800;
        let data = frames * 2;
        let mut b = Vec::new();
        b.extend_from_slice(b"RIFF");
        b.extend_from_slice(&(36 + data).to_le_bytes());
        b.extend_from_slice(b"WAVEfmt ");
        b.extend_from_slice(&16u32.to_le_bytes());
        b.extend_from_slice(&1u16.to_le_bytes());
        b.extend_from_slice(&1u16.to_le_bytes());
        b.extend_from_slice(&48_000u32.to_le_bytes());
        b.extend_from_slice(&96_000u32.to_le_bytes());
        b.extend_from_slice(&2u16.to_le_bytes());
        b.extend_from_slice(&16u16.to_le_bytes());
        b.extend_from_slice(b"data");
        b.extend_from_slice(&data.to_le_bytes());
        for i in 0..frames {
            let v = ((i as f32 * 0.05).sin() * 8000.0) as i16;
            b.extend_from_slice(&v.to_le_bytes());
        }
        std::fs::write(path, b).expect("wav");
    }

    #[test]
    fn browser_lists_folders_first_and_only_audio() {
        let d = test_dir("list");
        std::fs::create_dir(d.join("Takes")).expect("dir");
        write_wav(&d.join("b.wav"));
        write_wav(&d.join("A.WAV"));
        std::fs::write(d.join("notes.txt"), "x").expect("txt");
        write_wav(&d.join(".hidden.wav"));
        let names: Vec<String> = list_dir(&d)
            .expect("list")
            .into_iter()
            .map(|e| e.name)
            .collect();
        assert_eq!(names, ["Takes", "A.WAV", "b.wav"]);
        assert!(list_dir(&d.join("fehlt")).is_err());
        let _ = std::fs::remove_dir_all(&d);
    }

    #[test]
    fn placing_a_file_uses_the_selected_audio_track_and_cursor() {
        let d = test_dir("place");
        let f = d.join("Gesang.wav");
        write_wav(&f);
        let mut project = Project::default();
        let t = project.add_track(TrackKind::Audio, "Gesang");
        let mut w = ProjectWindow::new(project, None, String::new());
        w.selected_track = Some(t);
        w.cursor = 1920;
        place_file(&mut w, &f);
        let ev = &w.project.tracks[t].events;
        assert_eq!(ev.len(), 1);
        assert_eq!(ev[0].start.0, 1920);
        assert_eq!(w.selection, vec![ev[0].id]);
        w.undo();
        assert!(w.project.tracks[t].events.is_empty());
        let _ = std::fs::remove_dir_all(&d);
    }

    #[test]
    fn filter_ignores_case() {
        assert!(matches("", "x"));
        assert!(matches("loo", "Drum Loop"));
        assert!(!matches("bass", "Drum Loop"));
    }
}
