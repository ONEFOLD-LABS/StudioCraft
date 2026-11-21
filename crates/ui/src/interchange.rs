//! AAF- und OMF-Projekte importieren (Datei-Menü, Issue #154). Lesen übernimmt craft-aaf; hier
//! wird die erste Komposition auf Audiospuren abgebildet: je Tonspur der Datei eine Audiospur,
//! je Clip ein Audio-Event an seiner Zeit, mit Startpunkt in der Datei, konstanter Lautstärke
//! und Überblendungen als Fades. Audio kommt aus eingebettetem PCM oder aus den verknüpften
//! Dateien (absoluter Pfad, sonst neben der AAF/OMF-Datei gesucht).

use std::collections::HashMap;
use std::path::Path;

use craft_aaf::model::{CItem, CKind, Composition, Document, Gain, Source};
use craft_aaf::time::{TICKS_PER_SECOND, Tick};
use studiocraft_engine::AudioClip;
use studiocraft_project::{ClipId, EVENT_MAX_DB, EventId, TrackKind};

use crate::ProjectWindow;
use crate::i18n::{tr, trf};

/// Endungen von AAF- und OMF-Dateien.
pub(crate) const EXTENSIONS: [&str; 2] = ["aaf", "omf"];
/// Größte Datei, die gelesen wird (eingebettetes Audio kann groß sein).
#[cfg(not(target_arch = "wasm32"))]
pub(crate) const MAX_FILE_BYTES: u64 = 1 << 30;
/// Höchstens so viele Spuren und Clips werden übernommen.
const MAX_TRACKS: usize = 256;
const MAX_CLIPS: usize = 20_000;

/// Ein Clip der Datei, auf die Projektzeit gebracht (Sekunden).
#[derive(Debug, Clone, PartialEq)]
struct Placed {
    source: usize,
    start: f64,
    length: f64,
    /// Zeit in der Quelldatei, an der der Clip beginnt.
    from: f64,
    gain_db: f32,
    fade_in: f64,
    fade_out: f64,
    name: String,
}

/// Was aus der Datei übernommen wird: Spuren mit ihren Clips und Hinweise.
#[derive(Debug, Default)]
struct Plan {
    tracks: Vec<(String, Vec<Placed>)>,
    notes: Vec<String>,
}

fn seconds(t: Tick) -> f64 {
    t.0 as f64 / TICKS_PER_SECOND as f64
}

fn gain_db(g: Option<&Gain>, notes: &mut Vec<String>) -> f32 {
    let linear = match g {
        None => return 0.0,
        Some(Gain::Constant(v)) => *v,
        Some(Gain::Varying { points, .. }) => {
            let note = tr("Lautstärkekurven werden als fester Wert übernommen").to_owned();
            if !notes.contains(&note) {
                notes.push(note);
            }
            points.first().map_or(1.0, |p| p.1)
        }
    };
    if !linear.is_finite() || linear <= 0.0 {
        return -60.0;
    }
    (20.0 * linear.log10()).clamp(-60.0, f64::from(EVENT_MAX_DB)) as f32
}

/// Bildet die Tonspuren einer Komposition auf Spuren mit Clips ab.
fn plan(comp: &Composition, doc: &Document) -> Plan {
    let mut plan = Plan::default();
    let mut clips = 0usize;
    let mut nested = false;
    for (n, track) in comp
        .tracks
        .iter()
        .filter(|t| t.kind == CKind::Sound)
        .take(MAX_TRACKS)
        .enumerate()
    {
        let name = if track.name.trim().is_empty() {
            trf("Audio {n}", &[("n", &(n + 1))])
        } else {
            track.name.clone()
        };
        let mut placed: Vec<Placed> = Vec::new();
        let mut pos = Tick(0);
        // Fade-In des nächsten Clips nach einer Überblendung.
        let mut fade_next = 0.0;
        for item in &track.items {
            match item {
                CItem::Filler(len) => {
                    pos = Tick(pos.0.saturating_add(len.0.max(0)));
                    fade_next = 0.0;
                }
                CItem::Transition(t) => {
                    // Die Überblendung überlappt den Clip davor und den danach.
                    let len = t.len.0.max(0);
                    pos = Tick(pos.0.saturating_sub(len).max(0));
                    if let Some(prev) = placed.last_mut() {
                        prev.fade_out = seconds(Tick(len)).min(prev.length);
                    }
                    fade_next = seconds(Tick(len));
                }
                CItem::Clip(c) => {
                    let len = c.len.0.max(0);
                    let source = doc.sources.get(c.source);
                    let audible = source.is_some_and(|s| s.kind == CKind::Sound);
                    if source.is_some_and(|s| s.nested.is_some()) {
                        nested = true;
                    } else if audible && len > 0 && clips < MAX_CLIPS {
                        let offset = source.map_or(Tick(0), |s| s.offset);
                        let length = seconds(Tick(len));
                        placed.push(Placed {
                            source: c.source,
                            start: seconds(pos),
                            length,
                            from: seconds(Tick(c.start.0.saturating_sub(offset.0))).max(0.0),
                            gain_db: gain_db(c.gain.as_ref(), &mut plan.notes),
                            fade_in: fade_next.min(length),
                            fade_out: 0.0,
                            name: c.name.clone(),
                        });
                        clips += 1;
                    }
                    pos = Tick(pos.0.saturating_add(len));
                    fade_next = 0.0;
                }
            }
        }
        plan.tracks.push((name, placed));
    }
    if nested {
        plan.notes
            .push(tr("Verschachtelte Sequenzen werden nicht übernommen").to_owned());
    }
    if clips >= MAX_CLIPS {
        plan.notes.push(trf(
            "Nur die ersten {n} Clips übernommen",
            &[("n", &MAX_CLIPS)],
        ));
    }
    plan
}

/// Liest AAF oder OMF; dazu die Zahl der Hinweise des Lesers.
fn read(bytes: &[u8]) -> Result<(Document, usize), String> {
    let mut report = craft_aaf::Report::default();
    let doc = if craft_aaf::aaf::sniff(bytes) {
        craft_aaf::aaf::read(bytes, &mut report)
    } else if craft_aaf::omf::sniff(bytes) {
        craft_aaf::omf::read(bytes, &mut report)
    } else {
        return Err(tr("keine AAF- oder OMF-Datei").to_owned());
    };
    let doc = doc.map_err(|e| match e {
        craft_aaf::Error::Empty => tr("keine Zeitleiste in der Datei").to_owned(),
        e => trf("Datei nicht lesbar ({e})", &[("e", &e)]),
    })?;
    Ok((doc, report.warnings().count()))
}

/// Pfad einer verknüpften Datei: `file://`-URL oder Pfad, Prozentzeichen dekodiert.
#[cfg(not(target_arch = "wasm32"))]
fn local_path(reference: &str) -> String {
    let rest = reference
        .strip_prefix("file://localhost")
        .or_else(|| reference.strip_prefix("file://"))
        .or_else(|| reference.strip_prefix("file:"))
        .unwrap_or(reference);
    // Windows: file:///C:/… → C:/…
    let rest = match rest.as_bytes() {
        [b'/', d, b':', ..] if d.is_ascii_alphabetic() => &rest[1..],
        _ => rest,
    };
    let mut out = Vec::with_capacity(rest.len());
    let b = rest.as_bytes();
    let mut i = 0;
    while i < b.len() {
        let hex = b
            .get(i + 1..i + 3)
            .and_then(|h| std::str::from_utf8(h).ok())
            .and_then(|h| u8::from_str_radix(h, 16).ok());
        match (b[i], hex) {
            (b'%', Some(v)) => {
                out.push(v);
                i += 3;
            }
            (c, _) => {
                out.push(c);
                i += 1;
            }
        }
    }
    String::from_utf8_lossy(&out).into_owned()
}

/// Die Datei hinter `reference`: wie angegeben, sonst gleichnamig neben der AAF/OMF-Datei.
#[cfg(not(target_arch = "wasm32"))]
fn find_media(reference: &str, dir: Option<&Path>) -> Option<std::path::PathBuf> {
    let path = std::path::PathBuf::from(local_path(reference));
    if path.is_absolute() && path.is_file() {
        return Some(path);
    }
    let dir = dir?;
    let relative = dir.join(&path);
    if relative.is_file() {
        return Some(relative);
    }
    // Nur der Dateiname, auch bei Pfaden eines anderen Systems (Backslashes).
    let name = path
        .to_string_lossy()
        .rsplit(['/', '\\'])
        .next()?
        .to_owned();
    let beside = dir.join(name);
    beside.is_file().then_some(beside)
}

impl ProjectWindow {
    /// Lädt das Audio einer Quelle; `None` mit Hinweis, wenn es fehlt.
    fn interchange_clip(
        &mut self,
        source: &Source,
        dir: Option<&Path>,
        notes: &mut Vec<String>,
    ) -> Option<ClipId> {
        let channel = source
            .channel
            .filter(|_| source.file_channels > 1)
            .map(|c| c as usize);
        let name = if source.name.trim().is_empty() {
            tr("Audio").to_owned()
        } else {
            source.name.clone()
        };
        if let Some(pcm) = &source.embedded {
            let channels = u16::try_from(source.file_channels.max(1)).unwrap_or(u16::MAX);
            return match AudioClip::from_pcm(
                pcm,
                channels,
                source.bits,
                source.sample_rate,
                channel,
            ) {
                Ok(clip) => Some(self.add_pool_clip(&name, None, clip)),
                Err(e) => {
                    notes.push(trf("{name}: {fehler}", &[("name", &name), ("fehler", &e)]));
                    None
                }
            };
        }
        #[cfg(not(target_arch = "wasm32"))]
        if let Some(path) = source.path.as_deref().and_then(|r| find_media(r, dir)) {
            let ext = path.extension().and_then(|e| e.to_str());
            let loaded = std::fs::read(&path)
                .map_err(|e| e.to_string())
                .and_then(|b| {
                    AudioClip::from_bytes_channel(&b, ext, channel).map_err(|e| e.to_string())
                });
            return match loaded {
                // Ein einzelner Kanal einer Mehrkanaldatei wird beim Speichern als eigene Datei
                // abgelegt; sonst bleibt der Verweis auf die Datei.
                Ok(clip) => {
                    let keep = channel.is_none().then(|| path.display().to_string());
                    Some(self.add_pool_clip(&name, keep, clip))
                }
                Err(e) => {
                    notes.push(trf(
                        "{name}: {fehler}",
                        &[("name", &path.display()), ("fehler", &e)],
                    ));
                    None
                }
            };
        }
        let _ = dir;
        notes.push(trf("Audiodatei fehlt: {name}", &[("name", &name)]));
        None
    }

    /// Importiert ein AAF- oder OMF-Projekt aus Bytes: neue Audiospuren mit den Clips an ihren
    /// Zeiten (vom Projektanfang an). `dir` ist der Ordner der Datei, in dem verknüpftes Audio
    /// gesucht wird. Zurück kommen Hinweise auf nicht Übernommenes.
    pub fn import_interchange_bytes(
        &mut self,
        name: &str,
        bytes: &[u8],
        dir: Option<&Path>,
    ) -> Result<Vec<String>, String> {
        let err = |e: String| trf("{name}: {fehler}", &[("name", &name), ("fehler", &e)]);
        let (doc, warnings) = read(bytes).map_err(err)?;
        let comp = doc
            .compositions
            .first()
            .ok_or_else(|| err(tr("keine Zeitleiste in der Datei").to_owned()))?;
        let mut plan = plan(comp, &doc);
        if plan.tracks.iter().all(|(_, c)| c.is_empty()) {
            return Err(err(tr("keine Audio-Clips in der Datei").to_owned()));
        }
        let mut loaded: HashMap<usize, Option<ClipId>> = HashMap::new();
        let mut added_tracks = Vec::new();
        let mut added: Vec<EventId> = Vec::new();
        for (track_name, clips) in std::mem::take(&mut plan.tracks) {
            let track = self.project.add_track(TrackKind::Audio, &track_name);
            added_tracks.push(track);
            for c in clips {
                let Some(source) = doc.sources.get(c.source) else {
                    continue;
                };
                let clip = match loaded.get(&c.source) {
                    Some(id) => *id,
                    None => {
                        let id = self.interchange_clip(source, dir, &mut plan.notes);
                        loaded.insert(c.source, id);
                        id
                    }
                };
                let Some(clip) = clip else {
                    continue;
                };
                if let Some(ev) = self.place_interchange_clip(track, clip, &c) {
                    added.push(ev);
                }
            }
        }
        if warnings > 0 {
            plan.notes.push(trf(
                "{n} Teile der Datei wurden nicht übernommen",
                &[("n", &warnings)],
            ));
        }
        self.selected_track = added_tracks.first().copied();
        self.selection = added;
        self.dirty = true;
        Ok(plan.notes)
    }

    /// Legt das Event für einen Clip an: Zeiten in Ticks des Projekts, Startpunkt in Frames.
    fn place_interchange_clip(
        &mut self,
        track: usize,
        clip: ClipId,
        c: &Placed,
    ) -> Option<EventId> {
        let pool = self.project.pool.get(clip as usize)?;
        let (rate, frames) = (pool.sample_rate, pool.frames);
        let tempo = &self.project.tempo;
        let at = |s: f64| -> u64 {
            let t = tempo.ticks_at_seconds(s.max(0.0));
            if t.is_finite() {
                t.round().max(0.0) as u64
            } else {
                0
            }
        };
        let start = at(c.start);
        let offset =
            ((c.from * f64::from(rate)).round().max(0.0) as u64).min(frames.saturating_sub(1));
        // Nicht über das Ende der Quelle hinaus.
        let rest = (frames - offset) as f64 / f64::from(rate.max(1));
        let length_s = c.length.min(rest);
        let length = at(c.start + length_s).saturating_sub(start).max(1);
        let fade_in = at(c.start + c.fade_in.min(length_s))
            .saturating_sub(start)
            .min(length);
        let fade_out = at(c.start + length_s)
            .saturating_sub(at(c.start + length_s - c.fade_out.min(length_s)))
            .min(length - fade_in);
        let id = self.project.add_audio_event(track, start, clip)?;
        let ev = self.project.event_mut(id)?;
        ev.length = length;
        ev.offset = offset;
        ev.fade_in = fade_in;
        ev.fade_out = fade_out;
        ev.gain_db = c.gain_db;
        if !c.name.trim().is_empty() {
            ev.name = c.name.clone();
        }
        Some(id)
    }

    /// Dialog „AAF/OMF importieren“.
    #[cfg(not(target_arch = "wasm32"))]
    pub(crate) fn import_interchange_dialog(&mut self) {
        let Some(f) = crate::menu::dialog()
            .add_filter(tr("AAF- und OMF-Dateien"), &EXTENSIONS)
            .pick_file()
        else {
            return;
        };
        let name = f
            .file_stem()
            .map_or_else(|| "AAF".into(), |s| s.to_string_lossy().into_owned());
        let before = self.project.clone();
        let read = match std::fs::metadata(&f) {
            Ok(m) if m.len() > MAX_FILE_BYTES => Err(tr("Datei zu groß").to_owned()),
            _ => std::fs::read(&f).map_err(|e| e.to_string()),
        };
        let result = read
            .map_err(|e| trf("{name}: {fehler}", &[("name", &name), ("fehler", &e)]))
            .and_then(|bytes| self.import_interchange_bytes(&name, &bytes, f.parent()));
        self.message = match result {
            Ok(notes) if notes.is_empty() => None,
            Ok(notes) => Some(notes.join("; ")),
            Err(e) => Some(e),
        };
        self.checkpoint(before);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use craft_aaf::model::{CClip, CTrack, CTransition};
    use craft_aaf::time::FrameRate;
    use studiocraft_project::{Project, TempoMap};

    fn secs(s: f64) -> Tick {
        Tick((s * TICKS_PER_SECOND as f64) as i64)
    }

    /// Eine Sekunde 16-Bit-PCM, mono, 48 kHz, ansteigend.
    fn source(name: &str) -> Source {
        let pcm: Vec<u8> = (0..48_000u32)
            .flat_map(|i| ((i % 1000) as i16 * 20).to_le_bytes())
            .collect();
        Source {
            key: name.into(),
            name: name.into(),
            kind: CKind::Sound,
            path: None,
            channel: None,
            channels: 1,
            file_channels: 1,
            width: 0,
            height: 0,
            frame_rate: FrameRate::new(25, 1),
            sample_rate: 48_000,
            bits: 16,
            length: secs(1.0),
            offset: Tick(0),
            start_tc: None,
            tc_rate: FrameRate::new(25, 1),
            embedded: Some(pcm),
            markers: Vec::new(),
            nested: None,
        }
    }

    fn clip(source: usize, len: f64, start: f64, gain: Option<Gain>) -> CItem {
        CItem::Clip(CClip {
            len: secs(len),
            source,
            start: secs(start),
            gain,
            name: String::new(),
        })
    }

    fn document() -> Document {
        Document {
            compositions: vec![Composition {
                name: "Schnitt".into(),
                aaf_mob_id: None,
                rate: FrameRate::new(25, 1),
                sample_rate: 48_000,
                width: 0,
                height: 0,
                start_tc: 0,
                drop: false,
                tracks: vec![
                    CTrack {
                        kind: CKind::Sound,
                        name: "Dialog".into(),
                        number: 1,
                        channels: 1,
                        items: vec![
                            CItem::Filler(secs(1.0)),
                            clip(0, 0.5, 0.25, Some(Gain::Constant(0.5))),
                            CItem::Transition(CTransition {
                                len: secs(0.1),
                                cut: secs(0.05),
                                effect: "MonoAudioDissolve".into(),
                            }),
                            clip(1, 0.4, 0.0, None),
                        ],
                    },
                    CTrack {
                        kind: CKind::Picture,
                        name: "Bild".into(),
                        number: 1,
                        channels: 0,
                        items: vec![CItem::Filler(secs(3.0))],
                    },
                    CTrack {
                        kind: CKind::Sound,
                        name: String::new(),
                        number: 2,
                        channels: 1,
                        items: vec![clip(0, 2.0, 0.0, None)],
                    },
                ],
                markers: Vec::new(),
            }],
            sources: vec![source("Take 1"), source("Take 2")],
            nested: Vec::new(),
        }
    }

    fn options() -> craft_aaf::aaf::WriteOptions<'static> {
        craft_aaf::aaf::WriteOptions {
            identity: craft_aaf::Identity {
                company: "Test",
                product: "Test",
                version: "1",
                platform: "Test",
                product_id: [0; 16],
            },
            small_sectors: false,
        }
    }

    fn aaf_bytes() -> Vec<u8> {
        let mut report = craft_aaf::Report::default();
        craft_aaf::aaf::write(&document(), &options(), &mut report).unwrap()
    }

    #[test]
    fn plan_places_clips_with_gain_and_crossfades() {
        let doc = document();
        let plan = plan(&doc.compositions[0], &doc);
        assert_eq!(plan.tracks.len(), 2, "the picture track is left out");
        let (name, clips) = &plan.tracks[0];
        assert_eq!(name, "Dialog");
        assert_eq!(plan.tracks[1].0, "Audio 2");
        let a = &clips[0];
        assert!((a.start - 1.0).abs() < 1e-9 && (a.length - 0.5).abs() < 1e-9);
        assert!((a.from - 0.25).abs() < 1e-9);
        assert!((a.gain_db + 6.02).abs() < 0.01, "{}", a.gain_db);
        assert!((a.fade_out - 0.1).abs() < 1e-9);
        let b = &clips[1];
        assert!(
            (b.start - 1.4).abs() < 1e-9,
            "overlaps by the transition: {}",
            b.start
        );
        assert!((b.fade_in - 0.1).abs() < 1e-9);
    }

    #[test]
    fn an_aaf_file_imports_as_audio_tracks() {
        let mut p = Project::default();
        p.tempo = TempoMap::fixed(120.0);
        let mut w = ProjectWindow::new(p, None, "kein Audio (Test)".into());
        let notes = w
            .import_interchange_bytes("Schnitt", &aaf_bytes(), None)
            .unwrap();
        assert!(notes.is_empty(), "{notes:?}");
        let tracks = &w.project.tracks;
        assert_eq!(tracks.len(), 2);
        assert_eq!(tracks[0].name, "Dialog");
        assert!(tracks.iter().all(|t| t.kind == TrackKind::Audio));
        let a = &tracks[0].events[0];
        // 120 BPM: one second is two quarters (960 ticks each).
        assert_eq!(a.start.0, 1920);
        assert_eq!(a.length, 960);
        assert_eq!(
            a.offset,
            (0.25 * w.project.pool[0].sample_rate as f64) as u64
        );
        assert!((a.gain_db + 6.02).abs() < 0.01);
        assert_eq!(a.fade_out, 192);
        let b = &tracks[0].events[1];
        assert_eq!(b.fade_in, 192);
        // The second track's clip is longer than its one-second source: cut at the end.
        let c = &tracks[1].events[0];
        assert_eq!(c.length, 1920);
        assert_eq!(w.project.pool.len(), 2, "each source is loaded once");
        assert_eq!(w.selection.len(), 3);
    }

    #[test]
    fn an_omf_file_imports_too() {
        let mut report = craft_aaf::Report::default();
        let bytes = craft_aaf::omf::write(&document(), &options().identity, &mut report).unwrap();
        let mut w = ProjectWindow::new(Project::default(), None, "kein Audio (Test)".into());
        w.import_interchange_bytes("Schnitt", &bytes, None).unwrap();
        let events: usize = w.project.tracks.iter().map(|t| t.events.len()).sum();
        assert_eq!(events, 3);
        assert_eq!(w.project.tracks[0].name, "Dialog");
    }

    /// AAF and OMF files changed at random import or are refused, never panic; what imports
    /// leaves a project that needs no repair.
    #[test]
    fn fuzzed_aaf_and_omf_files() {
        use crate::fuzzkit::{Case, Rng, cases, mutate_bytes};
        let mut report = craft_aaf::Report::default();
        let omf = craft_aaf::omf::write(&document(), &options().identity, &mut report)
            .unwrap_or_default();
        let seeds = [aaf_bytes(), omf];
        let mut w = ProjectWindow::new(Project::default(), None, "Fuzz".into());
        for seed in &seeds {
            assert!(w.import_interchange_bytes("seed", seed, None).is_ok());
        }
        for seed in cases(0xaaf, 300) {
            let _case = Case("aaf-omf", seed);
            let mut rng = Rng(seed | 1);
            let mut bytes = seeds[rng.below(seeds.len())].clone();
            for _ in 0..1 + rng.below(4) {
                mutate_bytes(&mut rng, &mut bytes);
            }
            w.project = Project::default();
            if w.import_interchange_bytes("x", &bytes, None).is_ok() {
                let mut p = w.project.clone();
                assert_eq!(p.sanitize(), 0, "the import leaves valid values");
            }
        }
    }

    #[test]
    fn missing_media_and_hostile_files_are_reported() {
        let mut doc = document();
        for s in &mut doc.sources {
            s.embedded = None;
            s.path = Some("file:///nirgends/Take%201.wav".into());
        }
        let mut report = craft_aaf::Report::default();
        let bytes = craft_aaf::aaf::write(&doc, &options(), &mut report).unwrap();
        let mut w = ProjectWindow::new(Project::default(), None, "kein Audio (Test)".into());
        let notes = w.import_interchange_bytes("Schnitt", &bytes, None).unwrap();
        assert!(
            notes.iter().any(|n| n.contains("Audiodatei fehlt")),
            "{notes:?}"
        );
        assert!(w.project.tracks.iter().all(|t| t.events.is_empty()));

        assert!(w.import_interchange_bytes("x", b"kein AAF", None).is_err());
        let good = aaf_bytes();
        for cut in (0..good.len()).step_by(997) {
            let _ = w.import_interchange_bytes("x", &good[..cut], None);
        }
        assert_eq!(
            local_path("file:///C:/Ton/Take%201.wav"),
            "C:/Ton/Take 1.wav"
        );
        assert_eq!(local_path("file://localhost/home/a%2"), "/home/a%2");
    }

    #[test]
    fn linked_media_is_found_beside_the_file() {
        let dir = std::env::temp_dir().join(format!("sc-aaf-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let src = source("Take");
        let clip =
            AudioClip::from_pcm(src.embedded.as_deref().unwrap(), 1, 16, 48_000, None).unwrap();
        studiocraft_engine::export::write_wav_f32(&clip.samples, 48_000, &dir.join("Take 1.wav"))
            .unwrap();
        let mut doc = document();
        for s in &mut doc.sources {
            s.embedded = None;
            s.path = Some("file:///ganz/woanders/Take%201.wav".into());
        }
        let mut report = craft_aaf::Report::default();
        let bytes = craft_aaf::aaf::write(&doc, &options(), &mut report).unwrap();
        let mut w = ProjectWindow::new(Project::default(), None, "kein Audio (Test)".into());
        let notes = w
            .import_interchange_bytes("Schnitt", &bytes, Some(&dir))
            .unwrap();
        assert!(notes.is_empty(), "{notes:?}");
        assert_eq!(w.project.tracks[0].events.len(), 2);
        let path = w.project.pool[0].path.clone().unwrap();
        assert!(path.ends_with("Take 1.wav"), "{path}");
        let _ = std::fs::remove_dir_all(dir);
    }
}
