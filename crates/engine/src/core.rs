//! Echtzeit-Kern: verarbeitet Befehle und rendert Audio-Blöcke.
//!
//! [`EngineCore`] lebt im Audio-Thread, [`EngineControl`] in der Oberfläche.
//! Dazwischen liegen nur eine lock-freie Befehls-Queue und atomare Statuswerte.

use std::sync::{
    Arc,
    atomic::{AtomicBool, AtomicU8, AtomicU64, Ordering},
};

use studiocraft_project::{Position, TempoMap};

use crate::{AudioClip, Transport, TransportState};

/// Befehle von der Oberfläche an den Audio-Thread.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Command {
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
    pub status: Arc<EngineStatus>,
    pub tempo: TempoMap,
    pub sample_rate: u32,
}

impl EngineControl {
    /// Sendet einen Befehl. Ist die Queue voll, geht der Befehl verloren
    /// und `false` wird zurückgegeben.
    pub fn send(&mut self, cmd: Command) -> bool {
        self.commands.push(cmd).is_ok()
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
    clip: AudioClip,
    transport: Transport,
    commands: rtrb::Consumer<Command>,
    status: Arc<EngineStatus>,
}

/// Erzeugt das zusammengehörige Paar aus Kern und Steuerung.
pub fn engine(clip: AudioClip, tempo: TempoMap) -> (EngineCore, EngineControl) {
    let (producer, consumer) = rtrb::RingBuffer::new(256);
    let status = Arc::new(EngineStatus::default());
    let sample_rate = clip.sample_rate;
    let core = EngineCore {
        clip,
        transport: Transport::default(),
        commands: consumer,
        status: status.clone(),
    };
    let control = EngineControl {
        commands: producer,
        status,
        tempo,
        sample_rate,
    };
    (core, control)
}

impl EngineCore {
    pub fn sample_rate(&self) -> u32 {
        self.clip.sample_rate
    }

    fn apply(&mut self, cmd: Command) {
        let t = &mut self.transport;
        match cmd {
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

        let mut frames_left = (out.len() / 2) as u64;
        let mut offset = 0usize;
        while frames_left > 0 {
            if !self.transport.is_running() {
                out[offset..].fill(0.0);
                break;
            }
            let run = self.transport.frames_until_wrap(frames_left);
            let n = run as usize * 2;
            let dst = &mut out[offset..offset + n];
            let src_start = (self.transport.cursor * 2) as usize;
            let src = self.clip.samples.get(src_start..).unwrap_or(&[]);
            let copy = n.min(src.len());
            dst[..copy].copy_from_slice(&src[..copy]);
            dst[copy..].fill(0.0);
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

    /// Clip, dessen linker Kanal den Frame-Index enthält.
    fn ramp(frames: usize) -> AudioClip {
        let samples: Vec<f32> = (0..frames).flat_map(|i| [i as f32, -(i as f32)]).collect();
        AudioClip {
            sample_rate: 48_000,
            samples: samples.into(),
        }
    }

    fn left(out: &[f32]) -> Vec<f32> {
        out.chunks(2).map(|c| c[0]).collect()
    }

    #[test]
    fn stopped_renders_silence() {
        let (mut core, _ctl) = engine(ramp(100), TempoMap::default());
        let mut out = [1.0f32; 16];
        core.process(&mut out);
        assert!(out.iter().all(|&s| s == 0.0));
    }

    #[test]
    fn plays_clip_from_cursor() {
        let (mut core, mut ctl) = engine(ramp(100), TempoMap::default());
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
        let (mut core, mut ctl) = engine(ramp(100), TempoMap::default());
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
        let (mut core, mut ctl) = engine(ramp(3), TempoMap::default());
        ctl.send(Command::Locate(1));
        ctl.send(Command::Play);
        let mut out = [9.0f32; 8];
        core.process(&mut out);
        assert_eq!(left(&out), [1.0, 2.0, 0.0, 0.0]);
    }

    #[test]
    fn bar_to_frames_at_120_bpm() {
        let (_core, ctl) = engine(ramp(1), TempoMap::default());
        let bar2 = ctl.tempo.position(2, 1);
        assert_eq!(ctl.frames(bar2), 96_000);
        assert_eq!(ctl.position(96_000), bar2);
    }
}
