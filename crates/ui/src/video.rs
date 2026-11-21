//! Videospur (Spec „Spurtypen“ → Video, Issue #174): ein Video als Event mit Thumbnails, sein
//! Beginn im Projekt (das Event verschieben oder im Inspector eingeben) und sein Ton an/aus.
//!
//! Dekodiert werden nur lizenzfreie Formate (`docs/video-dekodierung.md`, erste Stufe): AV1,
//! VP9, VP8 und MPEG-2 in WebM/Matroska, MP4 und MOV, mit den reinen Rust-Decodern aus
//! craft-video. H.264, HEVC und Herstellerformate liefert StudioCraft nicht selbst aus; für sie
//! erscheint ein Hinweis. Thumbnails entstehen im Hintergrund, je Keyframe höchstens eines.

// Im Browser werden noch keine Videos gelesen: die Dekodierung bleibt dort ungenutzt.
#![cfg_attr(target_arch = "wasm32", allow(dead_code))]

use std::collections::HashMap;
use std::sync::{Arc, Mutex};

use craft_media::{ByteSource, CodecId, CodecParams};
use craft_video::frame::{ColorFamily, Plane, VideoFrame};
use egui::{ColorImage, TextureHandle, Ui};
use studiocraft_project::{Position, TrackKind};

use crate::ProjectWindow;
use crate::i18n::tr;

/// Endungen von Videodateien.
pub(crate) const EXTENSIONS: [&str; 5] = ["webm", "mkv", "mp4", "mov", "m4v"];
/// Höchstens so viele Thumbnails je Video.
const MAX_THUMBS: usize = 400;
/// Abstand der Thumbnails mindestens (Sekunden).
const MIN_GAP: f64 = 1.0;
/// Höhe eines Thumbnails in Pixeln.
const THUMB_H: usize = 48;
/// Größtes Bild, das dekodiert wird (Breite × Höhe).
const MAX_PIXELS: u64 = 8192 * 4320;

/// Ein Thumbnail: Zeit im Video (Sekunden) und Bild.
pub(crate) struct Thumb {
    pub seconds: f64,
    pub image: ColorImage,
}

/// Was beim Öffnen über das Video bekannt ist.
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct Probe {
    pub seconds: f64,
    pub width: u32,
    pub height: u32,
}

/// Die Keyframes der Videospur einer Datei: Codec, (Sekunden, Probe-Index) und Dauer.
struct Keyframes {
    params: CodecParams,
    list: Vec<(f64, usize)>,
    probe: Probe,
}

enum Container {
    Mkv(Box<craft_media::matroska::MkvFile>),
    Mp4(craft_media::isobmff::Mp4File),
}

/// Codecs, die StudioCraft selbst dekodiert.
fn supported(codec: CodecId) -> bool {
    matches!(
        codec,
        CodecId::Av1 | CodecId::Vp9 | CodecId::Vp8 | CodecId::Mpeg2Video
    )
}

fn unsupported() -> String {
    tr("Videoformat nicht unterstützt (AV1, VP9, VP8 oder MPEG-2 in WebM, MKV, MP4 oder MOV umwandeln)")
        .to_owned()
}

/// Öffnet den Container und listet die Keyframes der ersten Videospur.
fn open<S: ByteSource>(src: &S) -> Result<(Container, usize, Keyframes), String> {
    let mut magic = [0u8; 4];
    src.read_at(0, &mut magic).map_err(|e| e.to_string())?;
    if magic == [0x1A, 0x45, 0xDF, 0xA3] {
        use craft_media::matroska::TrackKind as K;
        let mkv = craft_media::matroska::open(src).map_err(|e| e.to_string())?;
        let v = mkv
            .track_of_kind(K::Video)
            .ok_or_else(|| tr("keine Videospur in der Datei"))?;
        let t = &mkv.tracks[v];
        let (num, den) = t.timebase;
        let secs = |pts: i64| pts as f64 * num as f64 / den.max(1) as f64;
        let list = t
            .samples
            .iter()
            .enumerate()
            .filter(|(_, s)| s.keyframe)
            .map(|(i, s)| (secs(s.pts), i))
            .collect();
        let end = t
            .samples
            .iter()
            .map(|s| secs(s.pts) + secs(s.duration as i64))
            .fold(0.0, f64::max);
        let seconds = mkv.duration_ns().map_or(end, |ns| ns as f64 / 1e9).max(end);
        let video = t.video.as_ref();
        let probe = Probe {
            seconds,
            width: video.map_or(0, |v| v.pixel_width),
            height: video.map_or(0, |v| v.pixel_height),
        };
        let params = t.codec_params();
        Ok((
            Container::Mkv(Box::new(mkv)),
            v,
            Keyframes {
                params,
                list,
                probe,
            },
        ))
    } else {
        use craft_media::isobmff::TrackKind as K;
        let mp4 = craft_media::isobmff::open(src).map_err(|_| unsupported())?;
        let v = mp4
            .track_of_kind(K::Video)
            .ok_or_else(|| tr("keine Videospur in der Datei"))?;
        let t = &mp4.tracks[v];
        let scale = f64::from(t.timescale.max(1));
        let list = t
            .samples
            .iter()
            .enumerate()
            .filter(|(_, s)| s.is_sync)
            .filter_map(|(i, _)| Some((t.presentation_pts(i)? as f64 / scale, i)))
            .collect();
        let probe = Probe {
            seconds: t.duration as f64 / scale,
            width: t.width,
            height: t.height,
        };
        let params = t.codec_params();
        Ok((
            Container::Mp4(mp4),
            v,
            Keyframes {
                params,
                list,
                probe,
            },
        ))
    }
}

/// Dauer und Größe des Videos; Fehler, wenn StudioCraft das Format nicht dekodiert.
pub(crate) fn probe<S: ByteSource>(src: &S) -> Result<Probe, String> {
    let (_, _, k) = open(src)?;
    if !supported(k.params.codec) {
        return Err(unsupported());
    }
    if u64::from(k.probe.width) * u64::from(k.probe.height) > MAX_PIXELS {
        return Err(tr("Video zu groß").to_owned());
    }
    Ok(k.probe)
}

/// Dekodiert einen Keyframe für sich.
fn decode_key(params: &CodecParams, packet: &[u8]) -> Option<VideoFrame> {
    if params.codec == CodecId::Vp8 {
        let mut dec = craft_video::vp8::Decoder::new();
        let pic = dec.decode(packet, 0).ok()??;
        return VideoFrame::try_from(pic).ok();
    }
    let mut dec = craft_video::TrackDecoder::new(params).ok()?;
    let mut frames = dec.decode(packet, 0).ok()?;
    if frames.is_empty() {
        frames = dec.flush().ok()?;
    }
    frames.into_iter().next()
}

/// Ein Wert einer Ebene an (x, y), auf 0–1 gebracht.
fn sample(plane: &Plane, x: u32, y: u32) -> f32 {
    let x = x.min(plane.width().saturating_sub(1)) as usize;
    let y = y.min(plane.height().saturating_sub(1));
    if let Some(row) = plane.row_u8(y) {
        return row.get(x).map_or(0.0, |v| f32::from(*v) / 255.0);
    }
    let max = ((1u32 << plane.bit_depth().clamp(1, 16)) - 1) as f32;
    plane
        .row_u16(y)
        .and_then(|r| r.get(x))
        .map_or(0.0, |v| f32::from(*v) / max)
}

/// Verkleinert ein Bild auf [`THUMB_H`] Pixel Höhe (nächster Nachbar), nach RGB.
fn thumbnail(frame: &VideoFrame) -> Option<ColorImage> {
    let (w, h) = (frame.width(), frame.height());
    if w == 0 || h == 0 {
        return None;
    }
    let th = THUMB_H;
    let tw = ((w as f32 * th as f32 / h as f32).round() as usize).clamp(1, 4 * th);
    let planes = frame.planes();
    let sub = frame.subsampling();
    // BT.709 für HD, sonst BT.601; begrenzter Bereich, wenn nicht anders angegeben.
    let (kr, kb) = if h > 576 {
        (0.2126, 0.0722)
    } else {
        (0.299, 0.114)
    };
    let full = frame.color.full_range;
    let mut pixels = Vec::with_capacity(tw * th);
    for ty in 0..th {
        let y = (ty as u64 * u64::from(h) / th as u64) as u32;
        for tx in 0..tw {
            let x = (tx as u64 * u64::from(w) / tw as u64) as u32;
            let rgb = match frame.family() {
                ColorFamily::Rgb => {
                    [0, 1, 2].map(|i| planes.get(i).map_or(0.0, |p| sample(p, x, y)))
                }
                ColorFamily::Gray => [planes.first().map_or(0.0, |p| sample(p, x, y)); 3],
                ColorFamily::YCbCr => {
                    let (cx, cy) = (x >> sub.x.min(3), y >> sub.y.min(3));
                    let luma = planes.first().map_or(0.0, |p| sample(p, x, y));
                    let cb = planes.get(1).map_or(0.5, |p| sample(p, cx, cy));
                    let cr = planes.get(2).map_or(0.5, |p| sample(p, cx, cy));
                    let (luma, cb, cr) = if full {
                        (luma, cb - 0.5, cr - 0.5)
                    } else {
                        (
                            (luma - 16.0 / 255.0) * 255.0 / 219.0,
                            (cb - 128.0 / 255.0) * 255.0 / 224.0,
                            (cr - 128.0 / 255.0) * 255.0 / 224.0,
                        )
                    };
                    let kg = 1.0 - kr - kb;
                    let r = luma + 2.0 * (1.0 - kr) * cr;
                    let b = luma + 2.0 * (1.0 - kb) * cb;
                    let g = (luma - kr * r - kb * b) / kg;
                    [r, g, b]
                }
            };
            let [r, g, b] = rgb.map(|v| (v.clamp(0.0, 1.0) * 255.0).round() as u8);
            pixels.push(egui::Color32::from_rgb(r, g, b));
        }
    }
    Some(ColorImage {
        size: [tw, th],
        pixels,
        source_size: egui::vec2(tw as f32, th as f32),
    })
}

/// Thumbnails eines Videos: je Keyframe eines, im Abstand von mindestens [`MIN_GAP`] und
/// höchstens [`MAX_THUMBS`] Stück.
pub(crate) fn thumbnails<S: ByteSource>(src: &S) -> Result<Vec<Thumb>, String> {
    let (container, track, keys) = open(src)?;
    if !supported(keys.params.codec) {
        return Err(unsupported());
    }
    let gap = (keys.probe.seconds / MAX_THUMBS as f64).max(MIN_GAP);
    let mut out: Vec<Thumb> = Vec::new();
    for &(seconds, index) in &keys.list {
        if out.last().is_some_and(|t| seconds - t.seconds < gap) || out.len() >= MAX_THUMBS {
            continue;
        }
        let packet = match &container {
            Container::Mkv(m) => m.read_sample(src, track, index).map_err(|e| e.to_string()),
            Container::Mp4(m) => m.read_sample(src, track, index).map_err(|e| e.to_string()),
        };
        let Some(image) = packet
            .ok()
            .and_then(|p| decode_key(&keys.params, &p))
            .and_then(|f| thumbnail(&f))
        else {
            continue;
        };
        out.push(Thumb { seconds, image });
    }
    if out.is_empty() {
        return Err(tr("Video nicht lesbar").to_owned());
    }
    Ok(out)
}

/// Ergebnis des Hintergrund-Threads.
type Slot = Arc<Mutex<Option<Result<Vec<Thumb>, String>>>>;

/// Thumbnails eines Videos in der Oberfläche: im Hintergrund erzeugt, dann als Texturen.
#[derive(Default)]
enum Entry {
    #[default]
    Missing,
    Loading(Slot),
    Ready(Vec<(f64, TextureHandle)>),
    Failed(String),
}

/// Thumbnails je Videodatei.
#[derive(Default)]
pub(crate) struct Cache {
    entries: HashMap<String, Entry>,
}

impl Cache {
    /// Startet das Erzeugen für neue Videos und übernimmt fertige als Texturen.
    pub(crate) fn poll(&mut self, ctx: &egui::Context, paths: &[String]) {
        for path in paths {
            let entry = self.entries.entry(path.clone()).or_default();
            match entry {
                Entry::Missing => *entry = start(ctx, path),
                Entry::Loading(slot) => {
                    let done = slot.lock().ok().and_then(|mut s| s.take());
                    match done {
                        Some(Ok(thumbs)) => {
                            let textures = thumbs
                                .into_iter()
                                .enumerate()
                                .map(|(i, t)| {
                                    let name = format!("video:{path}:{i}");
                                    (
                                        t.seconds,
                                        ctx.load_texture(name, t.image, Default::default()),
                                    )
                                })
                                .collect();
                            *entry = Entry::Ready(textures);
                        }
                        Some(Err(e)) => *entry = Entry::Failed(e),
                        None => {}
                    }
                }
                Entry::Ready(_) | Entry::Failed(_) => {}
            }
        }
    }

    /// Fertige Thumbnails eines Videos.
    pub(crate) fn thumbs(&self, path: &str) -> &[(f64, TextureHandle)] {
        match self.entries.get(path) {
            Some(Entry::Ready(t)) => t,
            _ => &[],
        }
    }

    /// Fehler beim Lesen eines Videos.
    pub(crate) fn error(&self, path: &str) -> Option<&str> {
        match self.entries.get(path) {
            Some(Entry::Failed(e)) => Some(e),
            _ => None,
        }
    }

    /// Für Tests: fertige Thumbnails eintragen (werden beim nächsten Bild zu Texturen).
    #[cfg(test)]
    pub(crate) fn insert(&mut self, path: &str, thumbs: Vec<Thumb>) {
        let slot = Arc::new(Mutex::new(Some(Ok(thumbs))));
        self.entries.insert(path.to_owned(), Entry::Loading(slot));
    }

    pub(crate) fn loading(&self, path: &str) -> bool {
        matches!(self.entries.get(path), Some(Entry::Loading(_)))
    }
}

/// Erzeugt die Thumbnails in einem eigenen Thread.
#[cfg(not(target_arch = "wasm32"))]
fn start(ctx: &egui::Context, path: &str) -> Entry {
    let slot = Arc::new(Mutex::new(None));
    let out = slot.clone();
    let ctx = ctx.clone();
    let path = path.to_owned();
    let spawned = std::thread::Builder::new()
        .name("video-thumbs".into())
        .spawn(move || {
            let result = std::fs::File::open(&path)
                .map_err(|e| format!("{path}: {e}"))
                .and_then(|f| thumbnails(&f));
            if let Ok(mut s) = out.lock() {
                *s = Some(result);
            }
            ctx.request_repaint();
        });
    match spawned {
        Ok(_) => Entry::Loading(slot),
        Err(e) => Entry::Failed(e.to_string()),
    }
}

#[cfg(target_arch = "wasm32")]
fn start(_ctx: &egui::Context, _path: &str) -> Entry {
    Entry::Failed(tr("Videos gehen im Browser noch nicht").to_owned())
}

/// Zeichnet die Thumbnails eines Video-Events nebeneinander in `area`.
pub(crate) fn paint(
    w: &ProjectWindow,
    painter: &egui::Painter,
    track: &studiocraft_project::Track,
    ev: &studiocraft_project::Event,
    rect: egui::Rect,
    area: egui::Rect,
) {
    let Some(path) = track.video.as_deref() else {
        return;
    };
    let thumbs = w.videos.thumbs(path);
    if area.width() <= 0.0 || area.height() <= 2.0 {
        return;
    }
    let tempo = &w.project.tempo;
    let start = tempo.seconds(ev.start);
    let px_per_tick = rect.width() / ev.length.max(1) as f32;
    let Some((_, first)) = thumbs.first() else {
        let text = w.videos.error(path).map_or_else(
            || tr("Thumbnails werden erstellt …").to_owned(),
            str::to_owned,
        );
        painter.text(
            area.left_center() + egui::vec2(6.0, 0.0),
            egui::Align2::LEFT_CENTER,
            text,
            egui::FontId::proportional(10.0),
            crate::theme::tokens(painter.ctx()).event_text,
        );
        return;
    };
    let size = first.size_vec2();
    let h = area.height();
    let tile = (size.x / size.y.max(1.0) * h).max(4.0);
    let painter = painter.with_clip_rect(area);
    let mut x = area.left() - (area.left() - rect.left()).rem_euclid(tile);
    while x < area.right() {
        // Zeit im Video an der linken Kante der Kachel.
        let ticks = ev.start.0 as f64 + f64::from((x - rect.left()).max(0.0) / px_per_tick);
        let at = tempo.seconds(Position(ticks as u64)) - start;
        let i = thumbs.partition_point(|(s, _)| *s <= at).saturating_sub(1);
        if let Some((_, tex)) = thumbs.get(i) {
            let r = egui::Rect::from_min_size(egui::pos2(x, area.top()), egui::vec2(tile, h));
            painter.image(
                tex.id(),
                r,
                egui::Rect::from_min_max(egui::pos2(0.0, 0.0), egui::pos2(1.0, 1.0)),
                egui::Color32::WHITE,
            );
        }
        x += tile;
    }
}

impl ProjectWindow {
    /// Pfade aller Videos im Projekt (für den Thumbnail-Cache).
    pub(crate) fn video_paths(&self) -> Vec<String> {
        self.project
            .tracks
            .iter()
            .filter(|t| t.kind == TrackKind::Video)
            .filter_map(|t| t.video.clone())
            .collect()
    }

    /// Legt eine Videospur für `path` an: Dauer und Format prüfen, den Ton laden (falls das
    /// Video einen hat und er sich lesen lässt), ein Rückgängig-Schritt.
    #[cfg(not(target_arch = "wasm32"))]
    pub fn add_video_file(&mut self, path: &std::path::Path) -> Result<usize, String> {
        let err = |e: String| format!("{}: {e}", path.display());
        let file = std::fs::File::open(path).map_err(|e| err(e.to_string()))?;
        let info = probe(&file).map_err(err)?;
        let name = path.file_stem().map_or_else(
            || tr("Video").to_owned(),
            |s| s.to_string_lossy().into_owned(),
        );
        let before = self.project.clone();
        // Ton: als eigener Clip ohne Pfad, damit beim Speichern nur der Ton (als WAV) ins
        // Projekt wandert, nicht das ganze Video.
        let clip = studiocraft_engine::AudioClip::load(path)
            .ok()
            .map(|c| self.add_pool_clip(&name, None, c));
        let length = self.project.tempo.ticks_at_seconds(info.seconds.max(0.0));
        let length = if length.is_finite() {
            length.round().max(1.0) as u64
        } else {
            1
        };
        let t = self
            .project
            .add_video_track(name, path.display().to_string(), length, clip);
        self.selected_track = Some(t);
        self.checkpoint(before);
        Ok(t)
    }

    /// Dialog „Videospur hinzufügen“.
    #[cfg(not(target_arch = "wasm32"))]
    pub(crate) fn add_video_dialog(&mut self) {
        let Some(f) = crate::menu::dialog()
            .add_filter(tr("Videodateien"), &EXTENSIONS)
            .pick_file()
        else {
            return;
        };
        if let Err(e) = self.add_video_file(&f) {
            self.message = Some(e);
        }
    }

    /// Videospur im Inspector: Datei, Beginn im Projekt und Ton an/aus.
    pub(crate) fn video_ui(&mut self, ui: &mut Ui, track: usize) {
        let Some(t) = self.project.tracks.get(track) else {
            return;
        };
        let path = t.video.clone().unwrap_or_default();
        let Some(ev) = t.events.first().cloned() else {
            return;
        };
        let file = std::path::Path::new(&path)
            .file_name()
            .map_or_else(|| path.clone(), |n| n.to_string_lossy().into_owned());
        ui.label(file).on_hover_text(&path);
        if let Some(e) = self.videos.error(&path) {
            ui.colored_label(ui.visuals().warn_fg_color, e);
        } else if self.videos.loading(&path) {
            ui.weak(tr("Thumbnails werden erstellt …"));
        }
        // Beginn des Videos im Projekt (Offset).
        let mut start = None;
        ui.horizontal(|ui| {
            let l = ui.label(tr("Beginn"));
            let id = ui.make_persistent_id(("video-beginn", track));
            let shown = self.project.tempo.format_position(ev.start);
            let mut text = ui
                .data(|d| d.get_temp::<String>(id))
                .unwrap_or_else(|| shown.clone());
            let r = ui
                .add(egui::TextEdit::singleline(&mut text).desired_width(110.0))
                .labelled_by(l.id);
            if r.has_focus() {
                ui.data_mut(|d| d.insert_temp(id, text));
            } else {
                ui.data_mut(|d| d.remove_temp::<String>(id));
                if r.lost_focus() && text != shown {
                    start = self.project.tempo.parse_position(&text);
                }
            }
        });
        let mut sound = !ev.muted;
        let has_sound = ev.clip.is_some();
        let r = ui
            .add_enabled(has_sound, egui::Checkbox::new(&mut sound, tr("Ton")))
            .on_disabled_hover_text(tr("Das Video hat keinen lesbaren Ton"));
        if start.is_some() || r.changed() {
            let before = self.project.clone();
            if let Some(e) = self.project.event_mut(ev.id) {
                if let Some(p) = start {
                    e.start = p;
                }
                if has_sound {
                    e.muted = !sound;
                }
            }
            self.checkpoint(before);
        }
    }
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use craft_media::matroska::{MkvWriter, MuxOptions, TrackKind as K, TrackSpec};

    /// Ein WebM-Video (VP9, 64×36): `frames` Bilder bei 10 fps, Keyframe jede Sekunde; das
    /// Bild wird von Bild zu Bild heller.
    pub(crate) fn webm(frames: u32) -> Vec<u8> {
        let (w, h) = (64u32, 36u32);
        let mut enc = craft_video::vp9::Vp9Encoder::new(craft_video::vp9::EncoderConfig {
            width: w,
            height: h,
            keyframe_interval: 10,
            frame_rate: 10.0,
            ..Default::default()
        });
        let mut spec = TrackSpec::new(K::Video, "V_VP9");
        spec.video_size = Some((w, h));
        let opts = MuxOptions {
            doc_type: "webm".into(),
            ..Default::default()
        };
        let mut mux = MkvWriter::new(std::io::Cursor::new(Vec::new()), vec![spec], opts).unwrap();
        for i in 0..frames {
            let luma = (40 + i * 4).min(235) as u8;
            let y = vec![luma; (w * h) as usize];
            let c = vec![128u8; ((w / 2) * (h / 2)) as usize];
            let f = enc.encode_frame(&y, &c, &c, i % 10 == 0);
            mux.write_frame(
                0,
                i64::from(i) * 100_000_000,
                i % 10 == 0,
                &f.data,
                Some(100_000_000),
            )
            .unwrap();
        }
        mux.finish().unwrap().into_inner()
    }

    #[test]
    fn a_webm_video_gives_its_length_and_one_thumbnail_per_second() {
        let bytes = webm(30);
        let info = probe(&bytes).unwrap();
        assert!((info.seconds - 3.0).abs() < 0.11, "{}", info.seconds);
        assert_eq!((info.width, info.height), (64, 36));
        let thumbs = thumbnails(&bytes).unwrap();
        let times: Vec<f64> = thumbs.iter().map(|t| t.seconds).collect();
        assert_eq!(times, [0.0, 1.0, 2.0]);
        assert_eq!(thumbs[0].image.size, [85, THUMB_H]);
        // Gray picture, getting brighter.
        let px = |t: &Thumb| t.image.pixels[t.image.pixels.len() / 2];
        let (a, b) = (px(&thumbs[0]), px(&thumbs[2]));
        assert!(b.r() > a.r(), "{a:?} {b:?}");
        assert!(
            (i16::from(a.r()) - i16::from(a.b())).abs() < 8,
            "gray: {a:?}"
        );
    }

    #[test]
    fn unsupported_and_hostile_files_are_errors() {
        assert!(probe(&b"nicht ein Video".to_vec()).is_err());
        let good = webm(12);
        for cut in (0..good.len()).step_by(211) {
            let part = good[..cut].to_vec();
            let _ = probe(&part);
            let _ = thumbnails(&part);
        }
    }

    #[test]
    fn a_video_track_sets_its_start_and_sound() {
        let dir = std::env::temp_dir().join(format!("sc-video-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let f = dir.join("Film.webm");
        std::fs::write(&f, webm(20)).unwrap();
        let mut p = studiocraft_project::Project::default();
        p.tempo = studiocraft_project::TempoMap::fixed(120.0);
        let mut w = ProjectWindow::new(p, None, "kein Audio (Test)".into());
        let t = w.add_video_file(&f).unwrap();
        let track = &w.project.tracks[t];
        assert_eq!(track.kind, TrackKind::Video);
        assert_eq!(track.name, "Film");
        // Two seconds at 120 BPM: four quarters.
        assert_eq!(track.events[0].length, 4 * 960);
        assert!(track.events[0].muted, "no sound in this video");
        w.undo();
        assert!(w.project.tracks.is_empty());
        assert!(w.add_video_file(&dir.join("fehlt.webm")).is_err());
        let _ = std::fs::remove_dir_all(dir);
    }
}
