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
    /// Alle Pakete der Spur in Dekodierreihenfolge: (Zeit in Sekunden, Keyframe).
    samples: Vec<(f64, bool)>,
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

/// Die Keyframes unter den Paketen: (Zeit, Index).
fn keys_of(samples: &[(f64, bool)]) -> Vec<(f64, usize)> {
    samples
        .iter()
        .enumerate()
        .filter(|(_, s)| s.1)
        .map(|(i, s)| (s.0, i))
        .collect()
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
        let samples: Vec<(f64, bool)> = t
            .samples
            .iter()
            .map(|s| (secs(s.pts), s.keyframe))
            .collect();
        let list = keys_of(&samples);
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
                samples,
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
        let samples: Vec<(f64, bool)> = t
            .samples
            .iter()
            .enumerate()
            .map(|(i, s)| {
                let pts = t.presentation_pts(i).unwrap_or(s.pts);
                (pts as f64 / scale, s.is_sync)
            })
            .collect();
        let list = keys_of(&samples);
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
                samples,
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

/// Ein Decoder für die Videospur, gleich welcher Codec.
enum Dec {
    Vp8(Box<craft_video::vp8::Decoder>),
    Track(Box<craft_video::TrackDecoder>),
}

impl Dec {
    fn new(params: &CodecParams) -> Option<Dec> {
        if params.codec == CodecId::Vp8 {
            return Some(Dec::Vp8(Box::new(craft_video::vp8::Decoder::new())));
        }
        craft_video::TrackDecoder::new(params)
            .ok()
            .map(|d| Dec::Track(Box::new(d)))
    }

    fn reset(&mut self) {
        match self {
            Dec::Vp8(d) => d.reset(),
            Dec::Track(d) => {
                let _ = d.reset();
            }
        }
    }

    /// Bilder, die dieses Paket fertig macht; ein Fehler gibt keine (das nächste Keyframe
    /// fängt neu an).
    fn decode(&mut self, packet: &[u8], pts: i64) -> Vec<VideoFrame> {
        match self {
            Dec::Vp8(d) => d
                .decode(packet, pts)
                .ok()
                .flatten()
                .and_then(|pic| VideoFrame::try_from(pic).ok())
                .map(|mut f| {
                    f.pts = Some(pts);
                    f
                })
                .into_iter()
                .collect(),
            Dec::Track(d) => d.decode(packet, pts).unwrap_or_default(),
        }
    }

    fn flush(&mut self) -> Vec<VideoFrame> {
        match self {
            Dec::Vp8(_) => Vec::new(),
            Dec::Track(d) => d.flush().unwrap_or_default(),
        }
    }
}

/// Dekodiert einen Keyframe für sich.
fn decode_key(params: &CodecParams, packet: &[u8]) -> Option<VideoFrame> {
    let mut dec = Dec::new(params)?;
    let mut frames = dec.decode(packet, 0);
    if frames.is_empty() {
        frames = dec.flush();
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
    Some(scaled(frame, tw, th))
}

/// Ein Bild, höchstens `max_w` × `max_h` groß (Seitenverhältnis bleibt, nicht vergrößert).
fn fitted(frame: &VideoFrame, max_w: usize, max_h: usize) -> Option<ColorImage> {
    let (w, h) = (frame.width() as f32, frame.height() as f32);
    if w < 1.0 || h < 1.0 {
        return None;
    }
    let k = (max_w as f32 / w).min(max_h as f32 / h).min(1.0);
    let tw = ((w * k).round() as usize).max(1);
    let th = ((h * k).round() as usize).max(1);
    Some(scaled(frame, tw, th))
}

/// Rechnet ein Bild auf `tw` × `th` Pixel um (nächster Nachbar), nach RGB.
fn scaled(frame: &VideoFrame, tw: usize, th: usize) -> ColorImage {
    let (w, h) = (frame.width(), frame.height());
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
    ColorImage {
        size: [tw, th],
        pixels,
        source_size: egui::vec2(tw as f32, th as f32),
    }
}

/// Höchstens so viele Pakete dekodiert der Player für ein Bild (danach gilt das letzte).
const MAX_DECODE: usize = 1200;
/// Größe des Bildes im Videofenster höchstens.
const VIEW_W: usize = 960;
const VIEW_H: usize = 540;

/// Spielt ein Video ab: liefert das Bild zu einer Zeit. Läuft die Zeit vorwärts, dekodiert er
/// von dort weiter; springt sie, beginnt er am Keyframe davor.
pub(crate) struct Player<S: ByteSource> {
    src: S,
    container: Container,
    track: usize,
    samples: Vec<(f64, bool)>,
    dec: Dec,
    /// Nächstes zu dekodierendes Paket.
    next: usize,
    /// Zuletzt fertiges Bild mit seiner Zeit.
    shown: Option<(f64, VideoFrame)>,
}

impl<S: ByteSource> Player<S> {
    pub(crate) fn open(src: S) -> Result<Self, String> {
        let (container, track, keys) = open(&src)?;
        if !supported(keys.params.codec) {
            return Err(unsupported());
        }
        let dec = Dec::new(&keys.params).ok_or_else(unsupported)?;
        Ok(Player {
            src,
            container,
            track,
            samples: keys.samples,
            dec,
            next: 0,
            shown: None,
        })
    }

    fn packet(&self, index: usize) -> Option<Vec<u8>> {
        match &self.container {
            Container::Mkv(m) => m.read_sample(&self.src, self.track, index).ok(),
            Container::Mp4(m) => m.read_sample(&self.src, self.track, index).ok(),
        }
    }

    /// Das Bild, das zur Zeit `t` (Sekunden im Video) zu sehen ist, mit seiner Zeit; `None`
    /// vor dem ersten Bild oder wenn nichts dekodierbar ist.
    pub(crate) fn frame_at(&mut self, t: f64) -> Option<(f64, &VideoFrame)> {
        const EPS: f64 = 1e-6;
        if !t.is_finite() {
            return None;
        }
        // Letztes Paket, das bis `t` zu sehen ist, und das Keyframe davor.
        let target = self.samples.iter().rposition(|s| s.0 <= t + EPS)?;
        let key = self.samples[..=target]
            .iter()
            .rposition(|s| s.1)
            .unwrap_or(0);
        let onward = self.shown.as_ref().is_some_and(|(s, _)| *s <= t + EPS)
            && self.next > key
            && self.next <= target + 1;
        if !onward {
            self.dec.reset();
            self.next = key;
            self.shown = None;
        }
        let want = self.samples[target].0;
        let mut budget = MAX_DECODE;
        while self.next < self.samples.len() && budget > 0 {
            if self.next > target
                && (self.shown.as_ref().is_some_and(|(s, _)| *s >= want - EPS)
                    || self.next > target + 4)
            {
                break;
            }
            let index = self.next;
            self.next += 1;
            budget -= 1;
            let Some(packet) = self.packet(index) else {
                continue;
            };
            for f in self.dec.decode(&packet, index as i64) {
                let at = f
                    .pts
                    .and_then(|p| usize::try_from(p).ok())
                    .and_then(|p| self.samples.get(p))
                    .map_or(self.samples[index].0, |s| s.0);
                let newer = self.shown.as_ref().is_none_or(|(s, _)| at >= *s);
                if at <= t + EPS && newer {
                    self.shown = Some((at, f));
                }
            }
        }
        self.shown.as_ref().map(|(s, f)| (*s, f))
    }
}

/// Was der Hintergrund-Thread zuletzt geliefert hat: Zeit und Bild (`None`: kein Bild an
/// dieser Stelle).
type ViewSlot = Arc<Mutex<Option<(f64, Option<ColorImage>)>>>;

/// Das Bild im Videofenster: ein Thread dekodiert zur angefragten Zeit, das Fenster zeigt das
/// zuletzt fertige Bild. Kommt er nicht nach, überspringt er Bilder (es gilt immer die neueste
/// Anfrage).
pub(crate) struct Viewer {
    path: String,
    requests: Option<std::sync::mpsc::Sender<f64>>,
    slot: ViewSlot,
    error: Arc<Mutex<Option<String>>>,
    texture: Option<TextureHandle>,
    /// Zeit des gezeigten Bildes; `None`: kein Bild an dieser Stelle.
    shown: Option<f64>,
    asked: Option<f64>,
}

impl Viewer {
    #[cfg(not(target_arch = "wasm32"))]
    fn start(ctx: &egui::Context, path: &str) -> Viewer {
        let (tx, rx) = std::sync::mpsc::channel::<f64>();
        let slot: ViewSlot = Arc::new(Mutex::new(None));
        let error = Arc::new(Mutex::new(None));
        let (out, err, ctx2, file) = (slot.clone(), error.clone(), ctx.clone(), path.to_owned());
        let spawned = std::thread::Builder::new()
            .name("video-view".into())
            .spawn(move || {
                let player = std::fs::File::open(&file)
                    .map_err(|e| format!("{file}: {e}"))
                    .and_then(Player::open);
                let mut player = match player {
                    Ok(p) => p,
                    Err(e) => {
                        if let Ok(mut s) = err.lock() {
                            *s = Some(e);
                        }
                        ctx2.request_repaint();
                        return;
                    }
                };
                let mut last: Option<f64> = None;
                // Endet, wenn das Fenster den Viewer verwirft (Sender weg).
                while let Ok(mut t) = rx.recv() {
                    while let Ok(newer) = rx.try_recv() {
                        t = newer;
                    }
                    let result = match player.frame_at(t) {
                        Some((at, _)) if Some(at) == last => continue,
                        Some((at, f)) => (at, fitted(f, VIEW_W, VIEW_H)),
                        None => (t, None),
                    };
                    last = result.1.as_ref().map(|_| result.0);
                    if let Ok(mut s) = out.lock() {
                        *s = Some(result);
                    }
                    ctx2.request_repaint();
                }
            });
        if let Err(e) = spawned
            && let Ok(mut s) = error.lock()
        {
            *s = Some(e.to_string());
        }
        Viewer {
            path: path.to_owned(),
            requests: Some(tx),
            slot,
            error,
            texture: None,
            shown: None,
            asked: None,
        }
    }

    #[cfg(target_arch = "wasm32")]
    fn start(_ctx: &egui::Context, path: &str) -> Viewer {
        Viewer {
            path: path.to_owned(),
            requests: None,
            slot: Arc::new(Mutex::new(None)),
            error: Arc::new(Mutex::new(Some(
                tr("Videos gehen im Browser noch nicht").to_owned(),
            ))),
            texture: None,
            shown: None,
            asked: None,
        }
    }

    /// Fragt das Bild zur Zeit `t` an (nur wenn sie sich geändert hat).
    fn request(&mut self, t: f64) {
        if self.asked.is_some_and(|a| (a - t).abs() < 1e-4) {
            return;
        }
        self.asked = Some(t);
        if let Some(tx) = &self.requests {
            let _ = tx.send(t);
        }
    }

    /// Übernimmt ein fertiges Bild als Textur.
    fn poll(&mut self, ctx: &egui::Context) {
        let done = self.slot.lock().ok().and_then(|mut s| s.take());
        let Some((at, image)) = done else {
            return;
        };
        match image {
            Some(image) => {
                match &mut self.texture {
                    Some(t) => t.set(image, Default::default()),
                    None => {
                        self.texture =
                            Some(ctx.load_texture("videofenster", image, Default::default()))
                    }
                }
                self.shown = Some(at);
            }
            None => self.shown = None,
        }
    }

    fn error(&self) -> Option<String> {
        self.error.lock().ok().and_then(|e| e.clone())
    }

    /// Für Tests: ein fertiges Bild eintragen.
    #[cfg(test)]
    pub(crate) fn with_image(path: &str, at: f64, image: ColorImage) -> Viewer {
        Viewer {
            path: path.to_owned(),
            requests: None,
            slot: Arc::new(Mutex::new(Some((at, Some(image))))),
            error: Arc::new(Mutex::new(None)),
            texture: None,
            shown: None,
            asked: None,
        }
    }
}

/// Zeit im Video als `m:ss.cc`.
fn video_time(seconds: f64) -> String {
    let cs = (seconds.max(0.0) * 100.0).round() as u64;
    format!("{}:{:02}.{:02}", cs / 6000, cs / 100 % 60, cs % 100)
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
    /// Die Videospur fürs Videofenster: die gewählte, sonst die erste; dazu ihr Pfad und der
    /// Beginn des Videos im Projekt (Sekunden).
    fn window_video(&self) -> Option<(String, f64)> {
        let tracks = &self.project.tracks;
        let pick = self
            .selected_track
            .filter(|&t| tracks.get(t).is_some_and(|t| t.kind == TrackKind::Video))
            .or_else(|| tracks.iter().position(|t| t.kind == TrackKind::Video))?;
        let t = tracks.get(pick)?;
        let start = t.events.first()?.start;
        Some((t.video.clone()?, self.project.tempo.seconds(start)))
    }

    /// Das Videofenster (Ansicht → Videofenster, Issue #219): das Bild an der Position des
    /// Cursors, beim Abspielen und beim Springen; dazu die Zeit im Video.
    pub(crate) fn video_window(&mut self, ctx: &egui::Context) {
        if !self.show_video {
            return;
        }
        let video = self.window_video();
        if let Some((path, _)) = &video
            && self.video_view.as_ref().is_none_or(|v| &v.path != path)
        {
            self.video_view = Some(Viewer::start(ctx, path));
        }
        let at = video
            .as_ref()
            .map(|(_, start)| self.project.tempo.seconds(Position(self.cursor)) - start);
        if let (Some(v), Some(t)) = (self.video_view.as_mut(), at) {
            if t >= 0.0 {
                v.request(t);
            }
            v.poll(ctx);
        }
        let mut open = true;
        egui::Window::new(tr("Video"))
            .open(&mut open)
            .default_size([480.0, 300.0])
            .default_pos(ctx.content_rect().right_top() + egui::vec2(-500.0, 90.0))
            .resizable(true)
            .show(ctx, |ui| {
                let (Some(v), Some(t)) = (self.video_view.as_ref(), at) else {
                    ui.weak(tr("Keine Videospur im Projekt"));
                    return;
                };
                if let Some(e) = v.error() {
                    ui.colored_label(ui.visuals().warn_fg_color, e);
                    return;
                }
                ui.label(egui::RichText::new(video_time(t)).monospace());
                match (&v.texture, v.shown) {
                    (Some(tex), Some(_)) if t >= 0.0 => {
                        let size = ui.available_size().max(egui::vec2(16.0, 9.0));
                        ui.add(
                            egui::Image::new(tex)
                                .fit_to_exact_size(size)
                                .maintain_aspect_ratio(true),
                        )
                        .on_hover_text(tr("Bild an der Cursor-Position"));
                    }
                    _ => {
                        ui.weak(tr("Kein Bild an dieser Stelle"));
                    }
                }
            });
        if !open {
            self.show_video = false;
            // Beendet den Thread.
            self.video_view = None;
        }
    }

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

    /// Das Bild zur Zeit `t` als Fensterbild (für den Render-Test).
    pub(crate) fn frame_image(p: &mut Player<Vec<u8>>, t: f64) -> ColorImage {
        fitted(p.frame_at(t).unwrap().1, VIEW_W, VIEW_H).unwrap()
    }

    /// Mittlerer Grauwert eines Bildes (Luma der ersten Ebene).
    fn luma(f: &VideoFrame) -> f32 {
        let p = &f.planes()[0];
        sample(p, p.width() / 2, p.height() / 2)
    }

    #[test]
    fn the_player_shows_the_frame_at_a_time_forward_and_after_jumps() {
        let mut p = Player::open(webm(30)).unwrap();
        assert!(p.frame_at(-1.0).is_none());
        let (at, f) = p.frame_at(0.05).unwrap();
        assert_eq!(at, 0.0);
        let first = luma(f);
        // Forward inside one GOP: decodes on from where it was.
        let (at, f) = p.frame_at(0.55).unwrap();
        assert!((at - 0.5).abs() < 1e-9, "{at}");
        let mid = luma(f);
        assert!(mid > first + 0.05, "{first} {mid}");
        let (at, f) = p.frame_at(2.95).unwrap();
        assert!((at - 2.9).abs() < 1e-9);
        let late = luma(f);
        assert!(late > mid);
        // Back to 0.5 s: starts again at the keyframe at 0 s and gives the same picture.
        let (at, f) = p.frame_at(0.5).unwrap();
        assert!((at - 0.5).abs() < 1e-9);
        assert!((luma(f) - mid).abs() < 0.02, "{} {mid}", luma(f));
        // After the end: the last picture.
        let (at, _) = p.frame_at(100.0).unwrap();
        assert!((at - 2.9).abs() < 1e-9);
        let img = fitted(p.frame_at(1.0).unwrap().1, VIEW_W, VIEW_H).unwrap();
        assert_eq!(img.size, [64, 36], "not enlarged");
        assert_eq!(video_time(61.234), "1:01.23");
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
