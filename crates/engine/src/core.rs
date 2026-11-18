//! Echtzeit-Kern: verarbeitet Befehle und rendert Audio-Blöcke.
//!
//! [`EngineCore`] lebt im Audio-Thread, [`EngineControl`] in der Oberfläche.
//! Dazwischen liegen nur eine lock-freie Befehls-Queue und atomare Statuswerte.

use std::sync::{
    Arc,
    atomic::{AtomicBool, AtomicU8, AtomicU64, Ordering},
};

use studiocraft_project::{Position, TempoMap};

use crate::{Song, Transport, TransportState};

/// Befehle von der Oberfläche an den Audio-Thread.
#[derive(Debug, Clone)]
pub enum Command {
    /// Neue Abspielliste. Die alte geht über die Rückgabe-Queue an die
    /// Oberfläche zurück und wird dort freigegeben, nicht im Audio-Thread.
    SetSong(Arc<Song>),
    Play,
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
    pub status: Arc<EngineStatus>,
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
    status: Arc<EngineStatus>,
}

/// Erzeugt das zusammengehörige Paar aus Kern und Steuerung.
pub fn engine(sample_rate: u32, tempo: TempoMap) -> (EngineCore, EngineControl) {
    let (producer, consumer) = rtrb::RingBuffer::new(256);
    let (garbage_tx, garbage_rx) = rtrb::RingBuffer::new(64);
    let status = Arc::new(EngineStatus::default());
    let core = EngineCore {
        song: Arc::new(Song::default()),
        sample_rate,
        transport: Transport::default(),
        commands: consumer,
        garbage: garbage_tx,
        status: status.clone(),
    };
    let control = EngineControl {
        commands: producer,
        garbage: garbage_rx,
        status,
        tempo,
        sample_rate,
    };
    (core, control)
}

impl EngineCore {
    pub fn sample_rate(&self) -> u32 {
        self.sample_rate
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
            Command::Stop => t.stop(false),
            Command::ToggleCycle => t.cycle = !t.cycle,
            Command::SetLocators(l, r) => {
                t.left_locator = l.min(r);
                t.right_locator = l.max(r);
            }
            Command::Locate(f) => t.cursor = f,
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
        while frames_left > 0 && self.transport.is_running() {
            let run = self.transport.frames_until_wrap(frames_left);
            let n = run as usize * 2;
            self.song.mix(
                &mut out[offset..offset + n],
                self.transport.cursor,
                run as usize,
            );
            self.transport.advance(run);
            offset += n;
            frames_left -= run;
        }

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
    fn bar_to_frames_at_120_bpm() {
        let (_core, ctl) = ramp(1);
        let bar2 = ctl.tempo.position(2, 1);
        assert_eq!(ctl.frames(bar2), 96_000);
        assert_eq!(ctl.position(96_000), bar2);
    }
}
