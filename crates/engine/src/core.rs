//! Echtzeit-Kern: verarbeitet Befehle und rendert Audio-Blöcke.
//!
//! [`EngineCore`] lebt im Audio-Thread, [`EngineControl`] in der Oberfläche.
//! Dazwischen liegen nur eine lock-freie Befehls-Queue und atomare Statuswerte.

use std::sync::{
    Arc,
    atomic::{AtomicBool, AtomicU8, AtomicU64, Ordering},
};

use studiocraft_project::{Position, TempoMap};

use crate::{InsertChain, MAX_METERED_TRACKS, Meters, Song, Transport, TransportState};

/// Größter Block, den der Kern in einem Stück mischt.
const SCRATCH_FRAMES: usize = 8192;

/// Befehle von der Oberfläche an den Audio-Thread.
#[derive(Debug)]
pub enum Command {
    /// Inserts einer Projektspur ersetzen (`None`: keine). Die alten gehen über
    /// die Rückgabe-Queue zurück und werden im Hauptthread zerstört.
    SetInserts(usize, Option<Box<InsertChain>>),
    /// Parameter eines Inserts setzen: Spur, Slot, Parameter-Id, Wert.
    SetParam(usize, usize, u32, f64),
    /// Insert umgehen oder wieder einschalten: Spur, Slot, umgangen.
    SetBypass(usize, usize, bool),
    /// Neue Abspielliste. Die alte geht über die Rückgabe-Queue an die
    /// Oberfläche zurück und wird dort freigegeben, nicht im Audio-Thread.
    SetSong(Arc<Song>),
    Play,
    /// Aufnahme starten (aus dem Stopp oder während der Wiedergabe).
    Record,
    Stop,
    ToggleCycle,
    /// Linker und rechter Locator in Frames.
    SetLocators(u64, u64),
    /// Cursor auf Frame setzen.
    Locate(u64),
}

/// Vom Audio-Thread veröffentlichter Zustand, lesbar von jedem Thread.
#[derive(Debug, Default)]
pub struct EngineStatus {
    cursor: AtomicU64,
    state: AtomicU8,
    cycle: AtomicBool,
    /// Projektposition (Frames), an der die letzte Aufnahme begann.
    rec_start: AtomicU64,
    /// Zeitpunkt (Uhr aus [`crate::clock`]), zu dem `rec_start` hörbar wurde; 0 = unbekannt.
    rec_played_at: AtomicU64,
    /// Gerenderte Blöcke, nützlich für Diagnose.
    blocks: AtomicU64,
}

impl EngineStatus {
    pub fn cursor(&self) -> u64 {
        self.cursor.load(Ordering::Relaxed)
    }
    pub fn state(&self) -> TransportState {
        TransportState::from_u8(self.state.load(Ordering::Relaxed))
    }
    pub fn rec_start(&self) -> u64 {
        self.rec_start.load(Ordering::Relaxed)
    }
    /// Wann `rec_start` aus dem Lautsprecher kam, falls die Ausgabe Zeitstempel liefert.
    pub fn rec_played_at(&self) -> Option<u64> {
        Some(self.rec_played_at.load(Ordering::Relaxed)).filter(|&n| n > 0)
    }
    pub fn cycle(&self) -> bool {
        self.cycle.load(Ordering::Relaxed)
    }
    pub fn blocks(&self) -> u64 {
        self.blocks.load(Ordering::Relaxed)
    }
}

/// Seite der Oberfläche: sendet Befehle, liest Status, rechnet Positionen um.
pub struct EngineControl {
    commands: rtrb::Producer<Command>,
    garbage: rtrb::Consumer<Arc<Song>>,
    plugin_garbage: rtrb::Consumer<Box<InsertChain>>,
    pub status: Arc<EngineStatus>,
    pub meters: Arc<Meters>,
    pub tempo: TempoMap,
    pub sample_rate: u32,
}

impl EngineControl {
    /// Sendet einen Befehl. Ist die Queue voll, geht der Befehl verloren
    /// und `false` wird zurückgegeben.
    pub fn send(&mut self, cmd: Command) -> bool {
        self.collect_garbage();
        self.commands.push(cmd).is_ok()
    }

    /// Gibt Abspiellisten frei, die der Audio-Thread nicht mehr braucht.
    pub fn collect_garbage(&mut self) {
        while self.garbage.pop().is_ok() {}
        // Plugins werden hier, im Hauptthread, deaktiviert und zerstört.
        while self.plugin_garbage.pop().is_ok() {}
    }

    pub fn frames(&self, pos: Position) -> u64 {
        (self.tempo.seconds(pos) * f64::from(self.sample_rate)).round() as u64
    }

    pub fn position(&self, frames: u64) -> Position {
        let seconds = frames as f64 / f64::from(self.sample_rate);
        let ticks = seconds * self.tempo.bpm / 60.0 * studiocraft_project::TICKS_PER_QUARTER as f64;
        Position(ticks as u64)
    }
}

/// Seite des Audio-Threads.
pub struct EngineCore {
    song: Arc<Song>,
    sample_rate: u32,
    transport: Transport,
    commands: rtrb::Consumer<Command>,
    garbage: rtrb::Producer<Arc<Song>>,
    plugin_garbage: rtrb::Producer<Box<InsertChain>>,
    /// Inserts je Projektspur, vorab angelegt (Austausch ohne Allokation).
    inserts: Vec<Option<Box<InsertChain>>>,
    /// Getrennte Kanäle für die Insert-Verarbeitung.
    planar: [Box<[f32]>; 2],
    /// Gehaltene Noten je Spur (Bit = Tonhöhe), für „alle Noten aus“.
    held: Vec<u128>,
    /// Beim nächsten Block alle gehaltenen Noten beenden (Stopp, Sprung, Cycle).
    panic: bool,
    status: Arc<EngineStatus>,
    meters: Arc<Meters>,
    /// Puffer für eine Spur, vorab angelegt (keine Allokation im Audio-Thread).
    scratch: Box<[f32]>,
    /// Wann der Anfang des aktuellen Blocks hörbar wird (Uhr aus [`crate::clock`]).
    block_played_at: u64,
}

/// Erzeugt das zusammengehörige Paar aus Kern und Steuerung.
pub fn engine(sample_rate: u32, tempo: TempoMap) -> (EngineCore, EngineControl) {
    let (producer, consumer) = rtrb::RingBuffer::new(256);
    let (garbage_tx, garbage_rx) = rtrb::RingBuffer::new(64);
    let (plugin_tx, plugin_rx) = rtrb::RingBuffer::new(64);
    let status = Arc::new(EngineStatus::default());
    let meters = Arc::new(Meters::default());
    let core = EngineCore {
        song: Arc::new(Song::default()),
        sample_rate,
        transport: Transport::default(),
        commands: consumer,
        garbage: garbage_tx,
        plugin_garbage: plugin_tx,
        inserts: (0..MAX_METERED_TRACKS).map(|_| None).collect(),
        held: vec![0; MAX_METERED_TRACKS],
        panic: false,
        planar: [
            vec![0.0; SCRATCH_FRAMES].into(),
            vec![0.0; SCRATCH_FRAMES].into(),
        ],
        status: status.clone(),
        meters: meters.clone(),
        scratch: vec![0.0; SCRATCH_FRAMES * 2].into_boxed_slice(),
        block_played_at: 0,
    };
    let control = EngineControl {
        commands: producer,
        garbage: garbage_rx,
        plugin_garbage: plugin_rx,
        status,
        meters,
        tempo,
        sample_rate,
    };
    (core, control)
}

impl EngineCore {
    pub fn sample_rate(&self) -> u32 {
        self.sample_rate
    }

    /// Vor `process`: wann der Anfang des nächsten Blocks hörbar wird.
    pub fn set_block_played_at(&mut self, nanos: u64) {
        self.block_played_at = nanos;
    }

    fn apply(&mut self, cmd: Command) {
        let t = &mut self.transport;
        match cmd {
            Command::SetSong(song) => {
                let old = std::mem::replace(&mut self.song, song);
                // Ist die Rückgabe-Queue voll, bleibt nur das Freigeben hier.
                // Bei 64 Plätzen und Abholung in jedem UI-Frame kommt das praktisch nicht vor.
                let _ = self.garbage.push(old);
            }
            Command::Play => t.play(),
            Command::Record => {
                self.status.rec_start.store(t.cursor, Ordering::Relaxed);
                self.status
                    .rec_played_at
                    .store(self.block_played_at, Ordering::Relaxed);
                t.record();
            }
            Command::Stop => {
                t.stop(false);
                self.panic = true;
            }
            Command::ToggleCycle => t.cycle = !t.cycle,
            Command::SetLocators(l, r) => {
                t.left_locator = l.min(r);
                t.right_locator = l.max(r);
            }
            Command::Locate(f) => {
                t.cursor = f;
                self.panic = true;
            }
            Command::SetInserts(track, chain) => {
                if let Some(slot) = self.inserts.get_mut(track) {
                    if let Some(mut old) = std::mem::replace(slot, chain) {
                        old.stop_processing();
                        // Volle Queue: dann bleibt nur das Freigeben hier (praktisch nie).
                        let _ = self.plugin_garbage.push(old);
                    }
                }
            }
            Command::SetParam(track, slot, id, value) => {
                if let Some(Some(chain)) = self.inserts.get_mut(track)
                    && let Some(ins) = chain.slot_mut(slot)
                {
                    ins.instance.set_param(id, value);
                }
            }
            Command::SetBypass(track, slot, bypass) => {
                if let Some(Some(chain)) = self.inserts.get_mut(track)
                    && let Some(ins) = chain.slot_mut(slot)
                {
                    ins.bypass = bypass;
                }
            }
        }
    }

    /// Rendert einen Block verschachteltes Stereo. Echtzeitsicher:
    /// keine Allokation, kein Lock.
    pub fn process(&mut self, out: &mut [f32]) {
        while let Ok(cmd) = self.commands.pop() {
            self.apply(cmd);
        }

        out.fill(0.0);
        let mut frames_left = (out.len() / 2) as u64;
        let mut offset = 0usize;
        // Instrumente und Inserts laufen auch im Stopp weiter (Ausklingen, Live-Spiel).
        while frames_left > 0 {
            let running = self.transport.is_running();
            let run = if running {
                self.transport.frames_until_wrap(frames_left)
            } else {
                frames_left
            }
            .min(SCRATCH_FRAMES as u64);
            let frames = run as usize;
            let n = frames * 2;
            let pos = self.transport.cursor;
            let panic = std::mem::take(&mut self.panic);
            let dst = &mut out[offset..offset + n];
            let tmp = &mut self.scratch[..n];
            for track in &self.song.tracks {
                let chain = self
                    .inserts
                    .get_mut(track.slot)
                    .and_then(|c| c.as_deref_mut())
                    .filter(|c| c.is_active());
                if !running && chain.is_none() {
                    continue;
                }
                tmp.fill(0.0);
                if running {
                    track.render(tmp, pos, frames);
                }
                if let Some(chain) = chain {
                    let held = &mut self.held[track.slot.min(MAX_METERED_TRACKS - 1)];
                    if panic {
                        for key in 0..128u8 {
                            if *held & (1 << key) != 0 {
                                chain.note(0, key, 0.0, false);
                            }
                        }
                        *held = 0;
                    }
                    if running && !track.notes.is_empty() {
                        let end = pos + run;
                        let last = track.notes.partition_point(|nt| nt.start < end);
                        for nt in &track.notes[..last] {
                            if nt.end < pos {
                                continue;
                            }
                            if nt.start >= pos {
                                chain.note(
                                    (nt.start - pos) as u32,
                                    nt.key,
                                    f32::from(nt.velocity) / 127.0,
                                    true,
                                );
                                *held |= 1 << nt.key;
                            }
                            if nt.end >= pos && nt.end < end {
                                chain.note((nt.end - pos) as u32, nt.key, 0.0, false);
                                *held &= !(1 << nt.key);
                            }
                        }
                    }
                    let [pl, pr] = &mut self.planar;
                    for (i, f) in tmp.chunks_exact(2).enumerate() {
                        pl[i] = f[0];
                        pr[i] = f[1];
                    }
                    chain.process(&mut pl[..frames], &mut pr[..frames]);
                    for (i, f) in tmp.chunks_exact_mut(2).enumerate() {
                        f[0] = pl[i];
                        f[1] = pr[i];
                    }
                }
                let (mut pl, mut pr) = (0.0f32, 0.0f32);
                for (o, t) in dst.chunks_exact_mut(2).zip(tmp.chunks_exact(2)) {
                    let (l, r) = (t[0] * track.gain_l, t[1] * track.gain_r);
                    pl = pl.max(l.abs());
                    pr = pr.max(r.abs());
                    o[0] += l;
                    o[1] += r;
                }
                self.meters.feed_track(track.slot, pl, pr);
            }
            if running {
                self.transport.advance(run);
                // Cycle-Sprung: gehaltene Noten beenden.
                if self.transport.cursor < pos + run {
                    self.panic = true;
                }
            }
            offset += n;
            frames_left -= run;
        }

        let master = self.song.master;
        let (mut pl, mut pr) = (0.0f32, 0.0f32);
        for o in out.chunks_exact_mut(2) {
            o[0] *= master;
            o[1] *= master;
            pl = pl.max(o[0].abs());
            pr = pr.max(o[1].abs());
        }
        self.meters.feed_master(pl, pr);

        let s = &self.status;
        s.cursor.store(self.transport.cursor, Ordering::Relaxed);
        s.state.store(self.transport.state as u8, Ordering::Relaxed);
        s.cycle.store(self.transport.cycle, Ordering::Relaxed);
        s.blocks.fetch_add(1, Ordering::Relaxed);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Kern mit einem Clip ab Frame 0, dessen linker Kanal den Frame-Index enthält.
    fn ramp(frames: usize) -> (EngineCore, EngineControl) {
        let samples: Vec<f32> = (0..frames).flat_map(|i| [i as f32, -(i as f32)]).collect();
        let (core, mut ctl) = engine(48_000, TempoMap::default());
        ctl.send(Command::SetSong(Arc::new(Song::single(samples.into()))));
        (core, ctl)
    }

    fn left(out: &[f32]) -> Vec<f32> {
        out.chunks(2).map(|c| c[0]).collect()
    }

    #[test]
    fn stopped_renders_silence() {
        let (mut core, _ctl) = ramp(100);
        let mut out = [1.0f32; 16];
        core.process(&mut out);
        assert!(out.iter().all(|&s| s == 0.0));
    }

    #[test]
    fn plays_clip_from_cursor() {
        let (mut core, mut ctl) = ramp(100);
        ctl.send(Command::Locate(10));
        ctl.send(Command::Play);
        let mut out = [0.0f32; 8];
        core.process(&mut out);
        assert_eq!(left(&out), [10.0, 11.0, 12.0, 13.0]);
        assert_eq!(out[1], -10.0);
        assert_eq!(ctl.status.cursor(), 14);
        assert_eq!(ctl.status.state(), TransportState::Playing);
    }

    #[test]
    fn cycle_wraps_sample_accurately_inside_block() {
        let (mut core, mut ctl) = ramp(100);
        ctl.send(Command::SetLocators(20, 23));
        ctl.send(Command::ToggleCycle);
        ctl.send(Command::Locate(21));
        ctl.send(Command::Play);
        let mut out = [0.0f32; 14];
        core.process(&mut out);
        assert_eq!(left(&out), [21.0, 22.0, 20.0, 21.0, 22.0, 20.0, 21.0]);
    }

    #[test]
    fn past_clip_end_is_silent() {
        let (mut core, mut ctl) = ramp(3);
        ctl.send(Command::Locate(1));
        ctl.send(Command::Play);
        let mut out = [9.0f32; 8];
        core.process(&mut out);
        assert_eq!(left(&out), [1.0, 2.0, 0.0, 0.0]);
    }

    #[test]
    fn replaced_song_is_returned_for_freeing() {
        let (mut core, mut ctl) = ramp(10);
        let mut out = [0.0f32; 4];
        core.process(&mut out);
        ctl.send(Command::SetSong(Arc::new(Song::default())));
        core.process(&mut out);
        // Die leere Start-Liste hat `send` schon freigegeben; die Rampen-Liste wartet.
        assert_eq!(ctl.garbage.slots(), 1);
        ctl.collect_garbage();
        assert_eq!(ctl.garbage.slots(), 0);
    }

    #[test]
    fn meters_report_track_and_master_peaks() {
        let (mut core, mut ctl) = ramp(100);
        let mut song = Song::single(
            (0..100)
                .flat_map(|_| [0.5f32, 0.25])
                .collect::<Vec<_>>()
                .into(),
        );
        song.tracks[0].gain_r = 2.0;
        song.master = 0.5;
        ctl.send(Command::SetSong(Arc::new(song)));
        ctl.send(Command::Play);
        core.process(&mut [0.0; 16]);
        assert_eq!(ctl.meters.take_track(0), [0.5, 0.5]);
        assert_eq!(ctl.meters.take_master(), [0.25, 0.25]);
        assert_eq!(
            ctl.meters.take_master(),
            [0.0, 0.0],
            "abgeholt ist abgeholt"
        );
    }

    #[test]
    fn record_remembers_start_position() {
        let (mut core, mut ctl) = ramp(10);
        ctl.send(Command::Locate(500));
        ctl.send(Command::Record);
        core.process(&mut [0.0; 8]);
        assert_eq!(ctl.status.rec_start(), 500);
        assert_eq!(ctl.status.state(), TransportState::Recording);
        assert_eq!(
            ctl.status.cursor(),
            504,
            "Aufnahme läuft wie Wiedergabe weiter"
        );
    }

    #[test]
    fn bar_to_frames_at_120_bpm() {
        let (_core, ctl) = ramp(1);
        let bar2 = ctl.tempo.position(2, 1);
        assert_eq!(ctl.frames(bar2), 96_000);
        assert_eq!(ctl.position(96_000), bar2);
    }
}
