//! Echtzeit-Kern: verarbeitet Befehle und rendert Audio-Blöcke.
//!
//! [`EngineCore`] lebt im Audio-Thread, [`EngineControl`] in der Oberfläche.
//! Dazwischen liegen nur eine lock-freie Befehls-Queue und atomare Statuswerte.

use std::sync::{
    Arc,
    atomic::{AtomicBool, AtomicU8, AtomicU32, AtomicU64, Ordering},
};

use studiocraft_project::{Position, TempoMap};

use crate::midi::{MidiMsg, MidiSender, RecordedMidi};
use crate::monitor::{MonitorInput, mix_into};
use crate::song::Output;
use crate::{InsertChain, MAX_METERED_TRACKS, Meters, Song, Transport, TransportState};

/// Größter Block, den der Kern in einem Stück mischt.
pub(crate) const SCRATCH_FRAMES: usize = 8192;

/// Befehle von der Oberfläche an den Audio-Thread.
#[derive(Debug)]
pub enum Command {
    /// Inserts einer Projektspur ersetzen (`None`: keine). Die alten gehen über
    /// die Rückgabe-Queue zurück und werden im Hauptthread zerstört.
    SetInserts(usize, Option<Box<InsertChain>>),
    /// Parameter eines Inserts setzen: Spur, Slot, Parameter-Id, Wert.
    SetParam(usize, usize, u32, f64),
    /// Note live spielen (Anspielen im Editor, MIDI-Keyboard): Spur, Taste,
    /// Anschlag 0–127, an/aus. Klingt am Anfang des nächsten Blocks.
    LiveNote(usize, u8, u8, bool),
    /// Metronom ein/aus.
    SetMetronome(bool),
    /// Ziel für das MIDI-Keyboard (Projektspur), `None`: niemand.
    SetMidiTarget(Option<usize>),
    /// Insert umgehen oder wieder einschalten: Spur, Slot, umgangen.
    SetBypass(usize, usize, bool),
    /// Verzögerungen für den Latenzausgleich (im Hauptthread angelegt); die alten gehen über
    /// die Rückgabe-Queue zurück. Dazu die Latenz am Ausgang in Frames.
    SetDelays(Box<crate::delay::DelayBank>, u32),
    /// Neue Abspielliste. Die alte geht über die Rückgabe-Queue an die
    /// Oberfläche zurück und wird dort freigegeben, nicht im Audio-Thread.
    SetSong(Arc<Song>),
    Play,
    /// Aufnahme starten (aus dem Stopp oder während der Wiedergabe).
    Record,
    /// Aufnahme nach einem Vorzähler: `frames` lang klickt es (Zählzeit alle `beat` Frames,
    /// Takt-Eins alle `beats_per_bar`), der Cursor steht; danach beginnt die Aufnahme
    /// sample-genau. Stopp bricht den Vorzähler ab.
    RecordAfterCount {
        frames: u64,
        beat: u64,
        beats_per_bar: u32,
    },
    Stop,
    ToggleCycle,
    /// Linker und rechter Locator in Frames.
    SetLocators(u64, u64),
    /// Cursor auf Frame setzen.
    Locate(u64),
    /// Eingang zum Mithören (Monitor). Ein vorheriger geht über die Rückgabe-Queue zurück.
    SetMonitorInput(Box<MonitorInput>),
    /// Vorhören (Browser): verschachteltes Stereo in der Abtastrate der Engine, unabhängig vom
    /// Transport, hinter dem Ausgangsfader. `None` beendet es. Die alten Samples gehen über die
    /// Rückgabe-Queue zurück.
    Preview(Option<Arc<[f32]>>),
}

/// Vom Audio-Thread veröffentlichter Zustand, lesbar von jedem Thread.
#[derive(Debug, Default)]
pub struct EngineStatus {
    cursor: AtomicU64,
    state: AtomicU8,
    cycle: AtomicBool,
    /// Vorhören läuft.
    previewing: AtomicBool,
    /// Audio-Last in Promille: Rechenzeit eines Blocks im Verhältnis zu seiner Dauer,
    /// schnell steigend, langsam fallend.
    load: AtomicU32,
    /// Projektposition (Frames), an der die letzte Aufnahme begann.
    rec_start: AtomicU64,
    /// Zeitpunkt (Uhr aus [`crate::clock`]), zu dem `rec_start` hörbar wurde; 0 = unbekannt.
    rec_played_at: AtomicU64,
    /// Gerenderte Blöcke, nützlich für Diagnose.
    blocks: AtomicU64,
    /// Restliche Frames des Vorzählers (0 = keiner).
    count_in: AtomicU64,
    /// Die letzte Aufnahme hat wirklich begonnen (nach einem Vorzähler erst an seinem Ende).
    rec_started: AtomicBool,
    /// Latenz am Ausgang durch Plugins (Frames, mit Latenzausgleich).
    latency: AtomicU32,
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
    /// Audio-Last: Anteil der Blockdauer, den das Rechnen braucht (1.0 = 100 %).
    pub fn load(&self) -> f32 {
        self.load.load(Ordering::Relaxed) as f32 / 1000.0
    }

    /// Ob gerade etwas vorgehört wird.
    pub fn previewing(&self) -> bool {
        self.previewing.load(Ordering::Relaxed)
    }
    pub fn cycle(&self) -> bool {
        self.cycle.load(Ordering::Relaxed)
    }
    pub fn blocks(&self) -> u64 {
        self.blocks.load(Ordering::Relaxed)
    }
    /// Restliche Frames des Vorzählers.
    pub fn count_in(&self) -> u64 {
        self.count_in.load(Ordering::Relaxed)
    }
    /// Ob die letzte Aufnahme begonnen hat (ein abgebrochener Vorzähler: nein).
    pub fn rec_started(&self) -> bool {
        self.rec_started.load(Ordering::Relaxed)
    }
    /// Latenz am Ausgang durch Plugins in Frames.
    pub fn latency(&self) -> u32 {
        self.latency.load(Ordering::Relaxed)
    }
}

/// Seite der Oberfläche: sendet Befehle, liest Status, rechnet Positionen um.
pub struct EngineControl {
    commands: rtrb::Producer<Command>,
    garbage: rtrb::Consumer<Arc<Song>>,
    plugin_garbage: rtrb::Consumer<Box<InsertChain>>,
    monitor_garbage: rtrb::Consumer<Box<MonitorInput>>,
    preview_garbage: rtrb::Consumer<Arc<[f32]>>,
    delay_garbage: rtrb::Consumer<Box<crate::delay::DelayBank>>,
    /// Schreibzugang für MIDI-Eingänge (Keyboard).
    pub midi: MidiSender,
    /// Während der Aufnahme gespielte Noten (vom Audio-Thread).
    pub recorded_midi: rtrb::Consumer<RecordedMidi>,
    /// Geplante Nachrichten an externe MIDI-Geräte; die Oberfläche übergibt sie dem
    /// Ausgabe-Thread ([`crate::midi_out::MidiOutThread::spawn`]).
    pub midi_out: Option<rtrb::Consumer<crate::midi_out::OutMsg>>,
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

    /// Wie [`Self::send`], gibt den Befehl aber zurück, wenn die Queue voll ist
    /// (statt ihn zu verwerfen).
    pub fn try_send(&mut self, cmd: Command) -> Result<(), Command> {
        self.collect_garbage();
        self.commands
            .push(cmd)
            .map_err(|rtrb::PushError::Full(c)| c)
    }

    /// Gibt Abspiellisten frei, die der Audio-Thread nicht mehr braucht.
    pub fn collect_garbage(&mut self) {
        while self.garbage.pop().is_ok() {}
        // Plugins werden hier, im Hauptthread, deaktiviert und zerstört.
        while self.plugin_garbage.pop().is_ok() {}
        while self.monitor_garbage.pop().is_ok() {}
        while self.preview_garbage.pop().is_ok() {}
        while self.delay_garbage.pop().is_ok() {}
    }

    pub fn frames(&self, pos: Position) -> u64 {
        (self.tempo.seconds(pos) * f64::from(self.sample_rate)).round() as u64
    }

    pub fn position(&self, frames: u64) -> Position {
        let seconds = frames as f64 / f64::from(self.sample_rate);
        let ticks = self.tempo.ticks_at_seconds(seconds);
        Position(if ticks.is_finite() && ticks > 0.0 {
            ticks as u64
        } else {
            0
        })
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
    monitor_garbage: rtrb::Producer<Box<MonitorInput>>,
    /// Eingang zum Mithören, sobald die Oberfläche ihn geöffnet hat.
    monitor: Option<Box<MonitorInput>>,
    /// Vorhören: Samples und Leseposition in Samples.
    preview: Option<(Arc<[f32]>, usize)>,
    preview_garbage: rtrb::Producer<Arc<[f32]>>,
    delay_garbage: rtrb::Producer<Box<crate::delay::DelayBank>>,
    /// Latenzausgleich je Projektspur.
    delays: Box<crate::delay::DelayBank>,
    /// Puffer für verzögerte Sends.
    send_tmp: Box<[f32]>,
    /// Weitere Ausgangsbusse (Audio-Anschlüsse), je ein Stereo-Puffer; Platz `k - 1` gehört
    /// zu `Output::Hw(k)`.
    hw_out: Box<[Box<[f32]>]>,
    /// Frames des zuletzt gerenderten Blocks (gültiger Teil von `hw_out`).
    last_frames: usize,
    midi_in: rtrb::Consumer<MidiMsg>,
    midi_rec: rtrb::Producer<RecordedMidi>,
    /// Nachrichten an externe MIDI-Geräte, mit Fälligkeit (midi_out.rs).
    midi_out: rtrb::Producer<crate::midi_out::OutMsg>,
    midi_target: Option<usize>,
    /// Puffer der Gruppen und FX-Kanäle, vorab angelegt.
    bus_bufs: Vec<Box<[f32]>>,
    /// Metronom: Frames pro Zählzeit, Zählzeiten pro Takt, ein/aus, Klänge
    /// (0 = Takt-Eins, 1 = übrige Zählzeiten) und der gerade klingende Klick.
    metronome: bool,
    /// Klicks: hell (Eins, Zählzeit), dunkel (Eins, Zählzeit).
    clicks: [Box<[f32]>; 4],
    click: Option<(usize, usize)>,
    /// Vorzähler: restliche Frames, Zählzeit in Frames, Zählzeiten je Takt, gezählte Frames.
    count_left: u64,
    count_beat: u64,
    count_bpb: u32,
    count_pos: u64,
    /// Haltepedal gedrückt; während der Aufnahme zurückgehaltene Note-Offs.
    sustain: bool,
    sustained: u128,
    /// Inserts je Projektspur, vorab angelegt (Austausch ohne Allokation).
    inserts: Vec<Option<Box<InsertChain>>>,
    /// Getrennte Kanäle für die Insert-Verarbeitung.
    planar: [Box<[f32]>; 2],
    /// Faktoren links/rechts pro Frame bei automatisierter Lautstärke, Pan oder Mute.
    auto_gain: [Box<[f32]>; 2],
    /// Faktor pro Frame eines automatisierten Sends.
    send_gain: Box<[f32]>,
    /// Filterzustände des Kanal-EQs je Projektspur, vorab angelegt.
    eq_state: Vec<crate::eq::EqState>,
    /// Zustand der Pre-Filter je Spur (Hochpass, Tiefpass; links, rechts).
    pre_state: Vec<[[crate::eq::BandState; 2]; 2]>,
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
    /// Frei laufender Frame-Zähler (zählt immer, auch im Stopp).
    clock: u64,
    /// Geglättete Audio-Last (siehe [`EngineStatus::load`]).
    load_avg: f32,
}

/// Kurzer Klick: Sinus mit schnell abfallender Hülle, 30 ms.
fn click_sound(rate: u32, hz: f32) -> Box<[f32]> {
    let n = (rate as usize * 30) / 1000;
    (0..n)
        .map(|i| {
            let t = i as f32 / rate as f32;
            (t * hz * std::f32::consts::TAU).sin() * (-t * 180.0).exp()
        })
        .collect()
}

/// Erzeugt das zusammengehörige Paar aus Kern und Steuerung.
pub fn engine(sample_rate: u32, tempo: TempoMap) -> (EngineCore, EngineControl) {
    let (producer, consumer) = rtrb::RingBuffer::new(256);
    let (garbage_tx, garbage_rx) = rtrb::RingBuffer::new(64);
    let (plugin_tx, plugin_rx) = rtrb::RingBuffer::new(64);
    let (monitor_tx, monitor_rx) = rtrb::RingBuffer::new(4);
    let (preview_tx, preview_rx) = rtrb::RingBuffer::new(16);
    let (delay_tx, delay_rx) = rtrb::RingBuffer::new(16);
    let (midi_tx, midi_rx) = rtrb::RingBuffer::new(1024);
    let (rec_tx, rec_rx) = rtrb::RingBuffer::new(8192);
    let (out_tx, out_rx) = rtrb::RingBuffer::new(crate::midi_out::OUT_RING);
    let status = Arc::new(EngineStatus::default());
    let meters = Arc::new(Meters::default());
    let core = EngineCore {
        song: Arc::new(Song::default()),
        sample_rate,
        transport: Transport::default(),
        commands: consumer,
        garbage: garbage_tx,
        plugin_garbage: plugin_tx,
        monitor_garbage: monitor_tx,
        monitor: None,
        preview: None,
        preview_garbage: preview_tx,
        delay_garbage: delay_tx,
        delays: Box::default(),
        send_tmp: vec![0.0; SCRATCH_FRAMES * 2].into_boxed_slice(),
        hw_out: (1..studiocraft_project::MAX_IO_BUSES)
            .map(|_| vec![0.0; SCRATCH_FRAMES * 2].into_boxed_slice())
            .collect(),
        last_frames: 0,
        midi_in: midi_rx,
        midi_rec: rec_tx,
        midi_out: out_tx,
        midi_target: None,
        bus_bufs: (0..crate::song::MAX_BUSES)
            .map(|_| vec![0.0; SCRATCH_FRAMES * 2].into_boxed_slice())
            .collect(),
        metronome: false,
        clicks: {
            let (b1, b2) = studiocraft_project::ClickSound::Bright.pitches();
            let (d1, d2) = studiocraft_project::ClickSound::Dark.pitches();
            [
                click_sound(sample_rate, b1),
                click_sound(sample_rate, b2),
                click_sound(sample_rate, d1),
                click_sound(sample_rate, d2),
            ]
        },
        click: None,
        count_left: 0,
        count_beat: 0,
        count_bpb: 4,
        count_pos: 0,
        sustain: false,
        sustained: 0,
        inserts: (0..MAX_METERED_TRACKS).map(|_| None).collect(),
        held: vec![0; MAX_METERED_TRACKS],
        panic: false,
        planar: [
            vec![0.0; SCRATCH_FRAMES].into(),
            vec![0.0; SCRATCH_FRAMES].into(),
        ],
        auto_gain: [
            vec![0.0; SCRATCH_FRAMES].into(),
            vec![0.0; SCRATCH_FRAMES].into(),
        ],
        send_gain: vec![0.0; SCRATCH_FRAMES].into(),
        eq_state: vec![Default::default(); MAX_METERED_TRACKS],
        pre_state: vec![Default::default(); MAX_METERED_TRACKS],
        status: status.clone(),
        meters: meters.clone(),
        scratch: vec![0.0; SCRATCH_FRAMES * 2].into_boxed_slice(),
        block_played_at: 0,
        clock: 0,
        load_avg: 0.0,
    };
    let control = EngineControl {
        commands: producer,
        garbage: garbage_rx,
        plugin_garbage: plugin_rx,
        monitor_garbage: monitor_rx,
        preview_garbage: preview_rx,
        delay_garbage: delay_rx,
        midi: std::sync::Arc::new(std::sync::Mutex::new(midi_tx)),
        recorded_midi: rec_rx,
        midi_out: Some(out_rx),
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
                self.status.rec_started.store(true, Ordering::Relaxed);
                t.record();
            }
            Command::RecordAfterCount {
                frames,
                beat,
                beats_per_bar,
            } => {
                if frames == 0 || t.is_running() {
                    // Kein Vorzähler oder schon in Wiedergabe: sofort.
                    self.status.rec_start.store(t.cursor, Ordering::Relaxed);
                    self.status
                        .rec_played_at
                        .store(self.block_played_at, Ordering::Relaxed);
                    self.status.rec_started.store(true, Ordering::Relaxed);
                    t.record();
                } else {
                    self.status.rec_started.store(false, Ordering::Relaxed);
                    self.status.rec_played_at.store(0, Ordering::Relaxed);
                    self.count_left = frames;
                    self.count_beat = beat.max(1);
                    self.count_bpb = beats_per_bar.max(1);
                    self.count_pos = 0;
                    self.click = None;
                }
            }
            Command::Stop => {
                self.count_left = 0;
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
            Command::LiveNote(track, key, velocity, on) => {
                let key = key.min(127);
                if let Some(Some(chain)) = self.inserts.get_mut(track) {
                    chain.note(0, key, f32::from(velocity) / 127.0, on);
                    let held = &mut self.held[track.min(MAX_METERED_TRACKS - 1)];
                    if on {
                        *held |= 1 << key;
                    } else {
                        *held &= !(1 << key);
                    }
                }
            }
            Command::SetMetronome(on) => {
                self.metronome = on;
                if !on {
                    self.click = None;
                }
            }
            Command::SetMidiTarget(t) => {
                // Gehaltene Noten des alten Ziels beenden.
                if self.midi_target != t {
                    self.panic = true;
                }
                self.midi_target = t;
            }
            Command::SetMonitorInput(input) => {
                if let Some(old) = self.monitor.replace(input) {
                    // Volle Queue: dann bleibt nur das Freigeben hier (praktisch nie).
                    let _ = self.monitor_garbage.push(old);
                }
            }
            Command::SetDelays(bank, total) => {
                let old = std::mem::replace(&mut self.delays, bank);
                // Volle Queue: dann bleibt nur das Freigeben hier (praktisch nie).
                let _ = self.delay_garbage.push(old);
                self.status.latency.store(total, Ordering::Relaxed);
            }
            Command::Preview(samples) => {
                let next = samples.map(|s| (s, 0));
                if let Some((old, _)) = std::mem::replace(&mut self.preview, next) {
                    // Volle Queue: dann bleibt nur das Freigeben hier (praktisch nie).
                    let _ = self.preview_garbage.push(old);
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

    /// Vorzähler-Klicks für die nächsten `frames` Frames (addiert); unabhängig vom Metronom.
    fn render_count_in(&mut self, dst: &mut [f32], frames: usize) {
        let settings = self.song.click;
        let level = studiocraft_project::db_to_gain(settings.level_db);
        let base = match settings.sound {
            studiocraft_project::ClickSound::Bright => 0,
            studiocraft_project::ClickSound::Dark => 2,
        };
        let beat = self.count_beat.max(1);
        let pos = self.count_pos;
        let end = pos + frames as u64;
        let mut k = pos.div_ceil(beat);
        let mut i = 0usize;
        loop {
            let beat_at = k.saturating_mul(beat);
            let until = if beat_at >= end {
                frames
            } else {
                (beat_at - pos) as usize
            };
            if let Some((which, mut cp)) = self.click {
                let sound = &self.clicks[which];
                for j in i..until {
                    let Some(&v) = sound.get(cp) else { break };
                    dst[j * 2] += v * level;
                    dst[j * 2 + 1] += v * level;
                    cp += 1;
                }
                self.click = (cp < sound.len()).then_some((which, cp));
            }
            if until >= frames {
                break;
            }
            let downbeat = k % u64::from(self.count_bpb) == 0;
            self.click = Some((base + usize::from(!(downbeat && settings.accent)), 0));
            k += 1;
            i = until;
        }
        self.count_pos = end;
    }

    /// Metronom-Klicks auf die Zählzeiten in `pos..pos + frames` legen (addiert). Die
    /// Zählzeiten folgen der Tempo-Map des Songs (auch Tempowechsel); keine Allokation.
    fn render_click(&mut self, dst: &mut [f32], pos: u64, frames: usize) {
        if !self.metronome {
            self.click = None;
            return;
        }
        let settings = self.song.click;
        let level = studiocraft_project::db_to_gain(settings.level_db);
        let base = match settings.sound {
            studiocraft_project::ClickSound::Bright => 0,
            studiocraft_project::ClickSound::Dark => 2,
        };
        let rate = self.sample_rate.max(1);
        let tempo = &self.song.tempo;
        // Erste Zählzeit ab `pos` (in Ticks), dann jede weitere bis zum Blockende; Taktart
        // und Takt-Eins folgen den Taktwechseln.
        let tick = tempo.ticks_at_seconds(pos as f64 / f64::from(rate)).ceil();
        let tick = if tick.is_finite() && tick > 0.0 {
            tick as u64
        } else {
            0
        };
        let (mut next, mut downbeat) = tempo.next_beat(tick);
        if tempo.frames(next, rate) < pos {
            (next, downbeat) = tempo.next_beat(next + 1);
        }
        let mut i = 0usize;
        let end = pos + frames as u64;
        loop {
            let beat_at = tempo.frames(next, rate);
            let until = if beat_at >= end {
                frames
            } else {
                (beat_at.max(pos) - pos) as usize
            };
            if let Some((which, mut cp)) = self.click {
                let sound = &self.clicks[which];
                for j in i..until {
                    let Some(&v) = sound.get(cp) else { break };
                    dst[j * 2] += v * level;
                    dst[j * 2 + 1] += v * level;
                    cp += 1;
                }
                self.click = (cp < sound.len()).then_some((which, cp));
            }
            if until >= frames {
                break;
            }
            let which = base + usize::from(!(downbeat && settings.accent));
            self.click = Some((which, 0));
            (next, downbeat) = tempo.next_beat(next + 1);
            i = until;
        }
    }

    /// Ausgangsbusse des aktuellen Songs: (erster Hardware-Kanal, Stereo); Platz 0 ist der
    /// Hauptmix (der Block aus [`Self::process`]).
    pub fn output_layout(&self) -> &[(u16, bool)] {
        &self.song.outputs
    }

    /// Stereo-Block des Ausgangsbusses `k` (ab 1) aus dem letzten [`Self::process`]; leer, wenn
    /// es ihn nicht gibt oder der Block größer als der Puffer war.
    pub fn hw_block(&self, k: usize) -> &[f32] {
        self.hw_out
            .get(k.wrapping_sub(1))
            .map_or(&[][..], |b| &b[..self.last_frames * 2])
    }

    /// Rendert einen Block verschachteltes Stereo. Echtzeitsicher:
    /// keine Allokation, kein Lock.
    pub fn process(&mut self, out: &mut [f32]) {
        let started = crate::clock::now_nanos();
        self.render(out);
        // Last: Rechenzeit gegen Blockdauer; Spitzen sofort, Abfall langsam.
        let busy = crate::clock::now_nanos().saturating_sub(started) as f64;
        let budget = (out.len() / 2) as f64 / f64::from(self.sample_rate.max(1)) * 1e9;
        if budget > 0.0 {
            let ratio = (busy / budget) as f32;
            self.load_avg = if ratio > self.load_avg {
                ratio
            } else {
                self.load_avg * 0.95 + ratio * 0.05
            };
            let permille = (self.load_avg * 1000.0).clamp(0.0, 100_000.0) as u32;
            self.status.load.store(permille, Ordering::Relaxed);
        }
    }

    fn render(&mut self, out: &mut [f32]) {
        while let Ok(cmd) = self.commands.pop() {
            self.apply(cmd);
        }

        // Keyboard: live an das Ziel, während der Aufnahme mit Position mitschreiben.
        while let Ok(m) = self.midi_in.pop() {
            let Some(track) = self.midi_target else {
                continue;
            };
            let recording = self.transport.state == TransportState::Recording;
            let running = self.transport.is_running();
            let pos = self.transport.cursor;
            let clock = self.clock;
            let chain = self.inserts.get_mut(track).and_then(|c| c.as_deref_mut());
            match m {
                MidiMsg::Note { key, velocity, on } => {
                    if let Some(chain) = chain {
                        chain.note(0, key, f32::from(velocity) / 127.0, on);
                        let held = &mut self.held[track.min(MAX_METERED_TRACKS - 1)];
                        if on {
                            *held |= 1 << key;
                        } else {
                            *held &= !(1 << key);
                        }
                    }
                    // Jede Note melden (Aufnahme oder nachträgliche Aufnahme). Pedal gedrückt: das
                    // Loslassen erst beim Pedal-Loslassen.
                    if !on && self.sustain {
                        self.sustained |= 1 << key;
                    } else {
                        if on {
                            self.sustained &= !(1 << key);
                        }
                        let _ = self.midi_rec.push(RecordedMidi {
                            pos,
                            key,
                            velocity,
                            on,
                            recording,
                            running,
                            clock,
                            ctrl: None,
                        });
                    }
                }
                MidiMsg::Other(data) => {
                    if let Some(chain) = chain {
                        chain.midi(0, data);
                    }
                    // Controller mitschreiben; das Haltepedal steckt schon in den Notenlängen.
                    let sustain = data[0] & 0xf0 == 0xb0 && data[1] == crate::midi::SUSTAIN;
                    if recording
                        && !sustain
                        && studiocraft_project::CtrlKind::from_midi(data).is_some()
                    {
                        let _ = self.midi_rec.push(RecordedMidi {
                            pos,
                            key: 0,
                            velocity: 0,
                            on: false,
                            recording,
                            running,
                            clock,
                            ctrl: Some(data),
                        });
                    }
                    if data[0] & 0xf0 == 0xb0 && data[1] == crate::midi::SUSTAIN {
                        let down = data[2] >= 64;
                        if self.sustain && !down {
                            for key in 0..128u8 {
                                if self.sustained & (1 << key) != 0 {
                                    let _ = self.midi_rec.push(RecordedMidi {
                                        pos,
                                        key,
                                        velocity: 0,
                                        on: false,
                                        recording,
                                        running,
                                        clock,
                                        ctrl: None,
                                    });
                                }
                            }
                        }
                        if !down {
                            self.sustained = 0;
                        }
                        self.sustain = down;
                    }
                }
            }
        }

        out.fill(0.0);
        let frames_total = out.len() / 2;
        self.last_frames = frames_total.min(SCRATCH_FRAMES);
        for b in self.hw_out.iter_mut() {
            b[..self.last_frames * 2].fill(0.0);
        }
        let mut frames_left = frames_total as u64;
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
            // Vorzähler: Abschnitt endet genau, wo die Aufnahme beginnt.
            let run = if self.count_left > 0 {
                run.min(self.count_left)
            } else {
                run
            };
            let frames = run as usize;
            let n = frames * 2;
            let pos = self.transport.cursor;
            let panic = std::mem::take(&mut self.panic);
            let dst = &mut out[offset..offset + n];
            let tmp = &mut self.scratch[..n];
            for b in self.bus_bufs.iter_mut().take(self.song.buses) {
                b[..n].fill(0.0);
            }
            // Eingang zum Mithören: immer lesen, damit die Verzögerung klein bleibt.
            let in_ch = self.monitor.as_ref().map_or(0, |m| m.channels());
            let input: &[f32] = match self.monitor.as_deref_mut() {
                Some(m) => m.read(frames),
                None => &[],
            };
            for track in &self.song.tracks {
                // MIDI-Spuren zu externen Geräten: Nachrichten mit Fälligkeit planen; Audio
                // haben sie keins. Vor dem Überspringen, damit Stopp ihre Noten beendet.
                if let Some((port, ch)) = track.midi_out {
                    let held = &mut self.held[track.slot.min(MAX_METERED_TRACKS - 1)];
                    let rate = u64::from(self.sample_rate.max(1));
                    // Ohne Zeitstempel vom Ausgang (manche Backends): jetzt, also sofort senden.
                    let block = if self.block_played_at > 0 {
                        self.block_played_at
                    } else {
                        crate::clock::now_nanos()
                    };
                    // Latenzausgleich: so spät senden, wie der Rest am Ausgang ankommt.
                    let first = (offset / 2) as u64
                        + self
                            .delays
                            .tracks
                            .get(track.slot)
                            .map_or(0, |l| u64::from(l.midi));
                    let due = |k: u64| block + (first + k) * 1_000_000_000 / rate;
                    let out = &mut self.midi_out;
                    let mut send = |at: u64, data: [u8; 3]| {
                        let _ = out.push(crate::midi_out::OutMsg {
                            due: due(at),
                            port,
                            data,
                        });
                    };
                    if panic {
                        for key in 0..128u8 {
                            if *held & (1 << key) != 0 {
                                send(0, [0x80 | ch, key, 0]);
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
                                send(nt.start - pos, [0x90 | ch, nt.key, nt.velocity.max(1)]);
                                *held |= 1 << nt.key;
                            }
                            if nt.end >= pos && nt.end < end {
                                send(nt.end - pos, [0x80 | ch, nt.key, 0]);
                                *held &= !(1 << nt.key);
                            }
                        }
                    }
                    // Controller mit dem Kanal der Spur.
                    if running && !track.ctrls.is_empty() {
                        let end = pos + run;
                        let first = track.ctrls.partition_point(|c| c.at < pos);
                        let last = track.ctrls.partition_point(|c| c.at < end);
                        for c in &track.ctrls[first..last] {
                            let mut data = c.data;
                            data[0] = (data[0] & 0xf0) | ch;
                            send(c.at - pos, data);
                        }
                    }
                    continue;
                }
                let mut chain = self
                    .inserts
                    .get_mut(track.slot)
                    .and_then(|c| c.as_deref_mut())
                    .filter(|c| c.is_active());
                // Busse laufen immer (sie sammeln, was andere Spuren schicken).
                if !running && chain.is_none() && track.bus.is_none() && track.monitor.is_none() {
                    continue;
                }
                tmp.fill(0.0);
                if let Some(b) = track.bus {
                    tmp.copy_from_slice(&self.bus_bufs[b][..n]);
                } else if running {
                    track.render(tmp, pos, frames);
                }
                if let Some((first, stereo)) = track.monitor {
                    mix_into(tmp, input, in_ch, first, stereo);
                }
                if let Some(chain) = chain.as_deref_mut() {
                    let held = &mut self.held[track.slot.min(MAX_METERED_TRACKS - 1)];
                    if panic {
                        for key in 0..128u8 {
                            if *held & (1 << key) != 0 {
                                chain.note(0, key, 0.0, false);
                            }
                        }
                        *held = 0;
                        // Haltepedal lösen und Pitchbend zurück in die Mitte.
                        chain.midi(0, [0xb0, crate::midi::SUSTAIN, 0]);
                        chain.midi(0, [0xe0, 0, 64]);
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
                    // Controller-Werte (Pitchbend, CC, Aftertouch) zeitgenau an das Instrument.
                    if running && !track.ctrls.is_empty() {
                        let end = pos + run;
                        let first = track.ctrls.partition_point(|c| c.at < pos);
                        let last = track.ctrls.partition_point(|c| c.at < end);
                        for c in &track.ctrls[first..last] {
                            chain.midi((c.at - pos) as u32, c.data);
                        }
                    }
                    // Automation: Plugin-Parameter als zeitgestempelte Ereignisse.
                    if running && let Some(auto) = &track.auto {
                        auto.param_events(pos, frames, |slot, time, id, value| {
                            if let Some(ins) = chain.by_model(slot) {
                                ins.instance.set_param_at(time, id, value);
                            }
                        });
                    }
                    let [pl, pr] = &mut self.planar;
                    for (i, f) in tmp.chunks_exact(2).enumerate() {
                        pl[i] = f[0];
                        pr[i] = f[1];
                    }
                    // Erst nur das Instrument; die Insert-Slots folgen hinter der Pre-Sektion.
                    chain.process_instrument(&mut pl[..frames], &mut pr[..frames]);
                    for (i, f) in tmp.chunks_exact_mut(2).enumerate() {
                        f[0] = pl[i];
                        f[1] = pr[i];
                    }
                }
                // Eingangsgain und Phase, dann Hoch- und Tiefpass (Spec „Signalfluss“).
                if let Some(g) = track.input_gain {
                    for v in tmp.iter_mut() {
                        *v *= g;
                    }
                }
                if !track.pre_filters.is_empty()
                    && let Some(state) = self.pre_state.get_mut(track.slot)
                {
                    for &(k, f) in &track.pre_filters {
                        let Some([l, r]) = state.get_mut(k) else {
                            continue;
                        };
                        for fr in tmp.chunks_exact_mut(2) {
                            fr[0] = f.tick(l, fr[0]);
                            fr[1] = f.tick(r, fr[1]);
                        }
                    }
                }
                // Inserts vor dem Fader (Slots 1–6 als Standard).
                let post_from = track.post_from.unwrap_or(usize::MAX);
                if let Some(chain) = chain.as_deref_mut().filter(|c| c.has_pre(post_from)) {
                    let [pl, pr] = &mut self.planar;
                    for (i, f) in tmp.chunks_exact(2).enumerate() {
                        pl[i] = f[0];
                        pr[i] = f[1];
                    }
                    chain.process_slots(false, post_from, &mut pl[..frames], &mut pr[..frames]);
                    for (i, f) in tmp.chunks_exact_mut(2).enumerate() {
                        f[0] = pl[i];
                        f[1] = pr[i];
                    }
                }
                // Kanal-EQ hinter den Inserts, vor Sends und Fader.
                if !track.eq.is_empty()
                    && let Some(state) = self.eq_state.get_mut(track.slot)
                {
                    crate::eq::process(&track.eq, state, tmp);
                }
                // Latenzausgleich: gemeinsamer Teil für alle Wege der Spur.
                let lines = self.delays.tracks.get_mut(track.slot);
                let mut lines = lines;
                if let Some(d) = lines.as_deref_mut().and_then(|l| l.common.as_mut()) {
                    d.process(tmp);
                }
                // Sends vor dem Fader.
                for (si, send) in track.sends.iter().enumerate().filter(|(_, s)| s.pre_fader) {
                    let sg = &mut self.send_gain[..frames];
                    let automated = running
                        && track
                            .auto
                            .as_deref()
                            .is_some_and(|a| a.fill_send_gain(send.model, pos, sg));
                    let st = &mut self.send_tmp[..n];
                    if automated {
                        for (i, (o, t)) in
                            st.chunks_exact_mut(2).zip(tmp.chunks_exact(2)).enumerate()
                        {
                            o[0] = t[0] * sg[i];
                            o[1] = t[1] * sg[i];
                        }
                    } else {
                        for (o, t) in st.iter_mut().zip(tmp.iter()) {
                            *o = t * send.gain;
                        }
                    }
                    // Latenzausgleich: zusätzlich für diesen Send.
                    if let Some(d) = lines
                        .as_deref_mut()
                        .and_then(|l| l.sends.get_mut(si))
                        .and_then(Option::as_mut)
                    {
                        d.process(st);
                    }
                    let bus = &mut self.bus_bufs[send.bus][..n];
                    for (o, t) in bus.iter_mut().zip(st.iter()) {
                        *o += t;
                    }
                }
                // Fader und Panorama an Ort und Stelle; Pegel messen.
                let (mut pl, mut pr) = (0.0f32, 0.0f32);
                let automated = track.auto.as_deref().filter(|a| running && a.has_gain());
                if let (Some(auto), Some(_)) = (automated, track.pan_mix) {
                    // Stereo kombiniert mit Automation: Matrix je Frame aus Pegel und Pan.
                    for (i, t) in tmp.chunks_exact_mut(2).enumerate() {
                        let (g, pan) = auto.level_at(pos + i as u64);
                        let m = studiocraft_project::pan_matrix(
                            studiocraft_project::PanMode::Combined,
                            pan,
                        );
                        let (l, r) = (t[0], t[1]);
                        t[0] = g * (l * m[0] + r * m[2]);
                        t[1] = g * (l * m[1] + r * m[3]);
                        pl = pl.max(t[0].abs());
                        pr = pr.max(t[1].abs());
                    }
                } else if let Some(m) = track.pan_mix.filter(|_| automated.is_none()) {
                    // Stereo kombiniert: feste Matrix (samt Lautstärke).
                    for t in tmp.chunks_exact_mut(2) {
                        let (l, r) = (t[0], t[1]);
                        t[0] = l * m[0] + r * m[2];
                        t[1] = l * m[1] + r * m[3];
                        pl = pl.max(t[0].abs());
                        pr = pr.max(t[1].abs());
                    }
                } else if let Some(auto) = automated {
                    // Automation: Faktor pro Frame (sample-genau).
                    let [gl, gr] = &mut self.auto_gain;
                    auto.fill_gains(pos, &mut gl[..frames], &mut gr[..frames]);
                    for (i, t) in tmp.chunks_exact_mut(2).enumerate() {
                        t[0] *= gl[i];
                        t[1] *= gr[i];
                        pl = pl.max(t[0].abs());
                        pr = pr.max(t[1].abs());
                    }
                } else {
                    for t in tmp.chunks_exact_mut(2) {
                        t[0] *= track.gain_l;
                        t[1] *= track.gain_r;
                        pl = pl.max(t[0].abs());
                        pr = pr.max(t[1].abs());
                    }
                }
                // Inserts hinter dem Fader (Slots 7–8 als Standard); Pegel danach messen.
                if let Some(chain) = chain.filter(|c| c.has_post(post_from)) {
                    let [ql, qr] = &mut self.planar;
                    for (i, f) in tmp.chunks_exact(2).enumerate() {
                        ql[i] = f[0];
                        qr[i] = f[1];
                    }
                    chain.process_slots(true, post_from, &mut ql[..frames], &mut qr[..frames]);
                    (pl, pr) = (0.0, 0.0);
                    for (i, f) in tmp.chunks_exact_mut(2).enumerate() {
                        f[0] = ql[i];
                        f[1] = qr[i];
                        pl = pl.max(f[0].abs());
                        pr = pr.max(f[1].abs());
                    }
                }
                self.meters.feed_track(track.slot, pl, pr);
                // Sends nach dem Fader.
                for (si, send) in track.sends.iter().enumerate().filter(|(_, s)| !s.pre_fader) {
                    let sg = &mut self.send_gain[..frames];
                    let automated = running
                        && track
                            .auto
                            .as_deref()
                            .is_some_and(|a| a.fill_send_gain(send.model, pos, sg));
                    let st = &mut self.send_tmp[..n];
                    if automated {
                        for (i, (o, t)) in
                            st.chunks_exact_mut(2).zip(tmp.chunks_exact(2)).enumerate()
                        {
                            o[0] = t[0] * sg[i];
                            o[1] = t[1] * sg[i];
                        }
                    } else {
                        for (o, t) in st.iter_mut().zip(tmp.iter()) {
                            *o = t * send.gain;
                        }
                    }
                    // Latenzausgleich: zusätzlich für diesen Send.
                    if let Some(d) = lines
                        .as_deref_mut()
                        .and_then(|l| l.sends.get_mut(si))
                        .and_then(Option::as_mut)
                    {
                        d.process(st);
                    }
                    let bus = &mut self.bus_bufs[send.bus][..n];
                    for (o, t) in bus.iter_mut().zip(st.iter()) {
                        *o += t;
                    }
                }
                // Weitere Ausgänge: dasselbe Signal, je mit eigenem Latenzausgleich.
                let g = track.out_gain.unwrap_or(1.0);
                for (ei, out) in track.extra.iter().enumerate() {
                    let st = &mut self.send_tmp[..n];
                    st.copy_from_slice(tmp);
                    if let Some(d) = lines
                        .as_deref_mut()
                        .and_then(|l| l.extras.get_mut(ei))
                        .and_then(Option::as_mut)
                    {
                        d.process(st);
                    }
                    let target: Option<&mut [f32]> = match *out {
                        Output::Hw(k) => self
                            .hw_out
                            .get_mut(k.wrapping_sub(1))
                            .and_then(|b| b.get_mut(offset..offset + n)),
                        Output::Master => Some(&mut dst[..]),
                        Output::Bus(b) => self.bus_bufs.get_mut(b).map(|v| &mut v[..n]),
                        Output::Nowhere => None,
                    };
                    if let Some(target) = target {
                        for (o, t) in target.iter_mut().zip(st.iter()) {
                            *o += t * g;
                        }
                    }
                }
                // Latenzausgleich: zusätzlich für den Ausgang.
                if let Some(d) = lines.and_then(|l| l.direct.as_mut()) {
                    d.process(tmp);
                }
                // Ausgang.
                let target: Option<&mut [f32]> = match track.output {
                    Output::Hw(k) => self
                        .hw_out
                        .get_mut(k.wrapping_sub(1))
                        .and_then(|b| b.get_mut(offset..offset + n)),
                    Output::Master => Some(&mut dst[..]),
                    Output::Bus(b) => Some(&mut self.bus_bufs[b][..n]),
                    Output::Nowhere => None,
                };
                if let Some(target) = target {
                    let g = track.out_gain.unwrap_or(1.0);
                    for (o, t) in target.iter_mut().zip(tmp.iter()) {
                        *o += t * g;
                    }
                }
            }
            if self.count_left > 0 {
                self.render_count_in(dst, frames);
                self.count_left -= run;
                if self.count_left == 0 {
                    // Ende des Vorzählers: Aufnahme beginnt hier, sample-genau.
                    let at = (offset / 2) as u64 + run;
                    let played = if self.block_played_at > 0 {
                        self.block_played_at.saturating_add(
                            at.saturating_mul(1_000_000_000) / u64::from(self.sample_rate.max(1)),
                        )
                    } else {
                        0
                    };
                    let t = &mut self.transport;
                    self.status.rec_start.store(t.cursor, Ordering::Relaxed);
                    self.status.rec_played_at.store(played, Ordering::Relaxed);
                    self.status.rec_started.store(true, Ordering::Relaxed);
                    t.record();
                }
            }
            if running {
                self.render_click(dst, pos, frames);
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

        // Weitere Ausgangsbusse: Kanalzug-Pegel (stumm = 0) und Pegelanzeige.
        let valid = self.last_frames * 2;
        for (k, bus) in self.hw_out.iter_mut().enumerate() {
            let gain = self.song.output_gains.get(k + 1).copied().unwrap_or(1.0);
            let (mut bl, mut br) = (0.0f32, 0.0f32);
            for o in bus[..valid].chunks_exact_mut(2) {
                o[0] *= gain;
                o[1] *= gain;
                bl = bl.max(o[0].abs());
                br = br.max(o[1].abs());
            }
            self.meters.feed_output(k + 1, bl, br);
        }

        // Vorhören hinter dem Ausgangsfader, unabhängig vom Transport.
        if let Some((samples, at)) = &mut self.preview {
            let n = out.len().min(samples.len().saturating_sub(*at));
            for (o, s) in out[..n].iter_mut().zip(&samples[*at..*at + n]) {
                *o += s;
            }
            *at += n;
            if *at >= samples.len()
                && let Some((old, _)) = self.preview.take()
            {
                let _ = self.preview_garbage.push(old);
            }
        }
        self.status
            .previewing
            .store(self.preview.is_some(), Ordering::Relaxed);

        let s = &self.status;
        s.cursor.store(self.transport.cursor, Ordering::Relaxed);
        s.state.store(self.transport.state as u8, Ordering::Relaxed);
        s.cycle.store(self.transport.cycle, Ordering::Relaxed);
        s.count_in.store(self.count_left, Ordering::Relaxed);
        s.blocks.fetch_add(1, Ordering::Relaxed);
        self.clock = self.clock.wrapping_add((out.len() / 2) as u64);
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

    #[test]
    fn midi_tracks_schedule_messages_for_external_devices() {
        use std::collections::HashMap;
        use studiocraft_project::{CtrlKind, CtrlPoint, Note, Project, TrackKind};
        let mut p = Project::default();
        let t = p.add_track(TrackKind::Midi, "Synth");
        p.tracks[t].midi_port = Some("Gerät".into());
        p.tracks[t].midi_channel = 3;
        // 48 frames per quarter would be too coarse; use a short part at tick 0.
        let id = p.create_event(t, 0, 4 * 960).unwrap();
        let ev = p.event_mut(id).unwrap();
        ev.notes = vec![Note {
            pitch: 60,
            velocity: 90,
            start: 0,
            length: 960,
        }];
        ev.controllers = vec![CtrlPoint {
            kind: CtrlKind::Cc(1),
            start: 480,
            value: 70,
        }];
        // A track without a port sends nothing.
        let silent = p.add_track(TrackKind::Midi, "Ohne");
        let id2 = p.create_event(silent, 0, 960).unwrap();
        p.event_mut(id2).unwrap().notes = vec![Note {
            pitch: 40,
            velocity: 90,
            start: 0,
            length: 10,
        }];
        let song = Song::build(&p, &HashMap::new(), 48_000);
        assert_eq!(song.midi_ports, ["Gerät"]);
        let track = song.tracks.iter().find(|tr| tr.slot == t).unwrap();
        assert_eq!(track.midi_out, Some((0, 2)));
        assert!(!song.tracks.iter().any(|tr| tr.slot == silent));

        let (mut core, mut ctl) = engine(48_000, TempoMap::default());
        let mut rx = ctl.midi_out.take().unwrap();
        ctl.send(Command::SetSong(Arc::new(song)));
        ctl.send(Command::Play);
        // One quarter at 120 BPM = 24 000 frames; play a bit more than that.
        // Wie der Audio-Ausgang: jeder Block wird eine Blockdauer später hörbar.
        let mut out = vec![0.0; 2 * 1024];
        let block_ns = 1024 * 1_000_000_000 / 48_000;
        for k in 1..=30u64 {
            core.set_block_played_at(k * block_ns);
            core.process(&mut out);
        }
        let mut got = Vec::new();
        while let Ok(m) = rx.pop() {
            got.push(m);
        }
        let data: Vec<[u8; 3]> = got.iter().map(|m| m.data).collect();
        assert_eq!(data, [[0x92, 60, 90], [0xb2, 1, 70], [0x82, 60, 0]]);
        assert!(
            got.windows(2).all(|w| w[0].due <= w[1].due),
            "in time order"
        );
        assert!(got.iter().all(|m| m.port == 0));
        // Stop while a note is held: a note-off goes out.
        ctl.send(Command::Locate(0));
        ctl.send(Command::Play);
        core.process(&mut out);
        ctl.send(Command::Stop);
        core.process(&mut out);
        let mut tail = Vec::new();
        while let Ok(m) = rx.pop() {
            tail.push(m.data);
        }
        assert_eq!(tail.last(), Some(&[0x82, 60, 0]));
    }

    #[test]
    fn combined_pan_moves_both_channels_balance_only_dims() {
        use studiocraft_project::{PanMode, pan_matrix};
        // Left channel only, four frames.
        let samples: Arc<[f32]> = vec![1.0, 0.0, 1.0, 0.0, 1.0, 0.0, 1.0, 0.0].into();
        let run = |mix: Option<[f32; 4]>, gl: f32, gr: f32| {
            let (mut core, mut ctl) = engine(48_000, TempoMap::default());
            let track = crate::PlayTrack {
                gain_l: gl,
                gain_r: gr,
                pan_mix: mix,
                events: vec![crate::song::PlayEvent::new(0, 4, 0, samples.clone())],
                ..Default::default()
            };
            ctl.send(Command::SetSong(Arc::new(Song {
                tracks: vec![track],
                master: 1.0,
                buses: 0,
                tempo: TempoMap::default(),
                midi_ports: Vec::new(),
                click: Default::default(),
                outputs: Vec::new(),
                output_gains: Vec::new(),
            })));
            ctl.send(Command::Play);
            let mut out = [0.0f32; 8];
            core.process(&mut out);
            (out[0], out[1])
        };
        // Balance hard right: the left-only signal disappears.
        let b = pan_matrix(PanMode::Balance, 1.0);
        assert_eq!(run(None, b[0], b[3]), (0.0, 0.0));
        // Combined hard right: it comes out fully on the right.
        let (l, r) = run(Some(pan_matrix(PanMode::Combined, 1.0)), 1.0, 1.0);
        assert!(l.abs() < 1e-6 && (r - 1.0).abs() < 1e-6, "{l} {r}");
        // Combined centre leaves the signal as it is.
        let (l, r) = run(Some(pan_matrix(PanMode::Combined, 0.0)), 1.0, 1.0);
        assert!((l - 1.0).abs() < 1e-6 && r.abs() < 1e-6, "{l} {r}");
    }

    #[test]
    fn input_gain_phase_and_pre_filters_act_before_the_fader() {
        let ones: Arc<[f32]> = vec![1.0; 2 * 4096].into();
        let run = |input_gain: Option<f32>, filters: Vec<(usize, crate::eq::Biquad)>| {
            let (mut core, mut ctl) = engine(48_000, TempoMap::default());
            let track = crate::PlayTrack {
                gain_l: 0.5,
                gain_r: 0.5,
                input_gain,
                pre_filters: filters,
                events: vec![crate::song::PlayEvent::new(0, 4096, 0, ones.clone())],
                ..Default::default()
            };
            ctl.send(Command::SetSong(Arc::new(Song {
                tracks: vec![track],
                master: 1.0,
                ..Default::default()
            })));
            ctl.send(Command::Play);
            let mut out = vec![0.0f32; 2 * 4096];
            core.process(&mut out);
            out
        };
        // Gain −6 dB with inverted phase, then the fader halves it again.
        let g = -studiocraft_project::db_to_gain(-6.0);
        let out = run(Some(g), Vec::new());
        assert!(
            (out[0] - g * 0.5).abs() < 1e-6 && out[0] < 0.0,
            "{}",
            out[0]
        );
        // A high-pass removes the DC signal after a while.
        let hp = crate::eq::Biquad::design(
            &studiocraft_project::EqBand {
                on: true,
                kind: studiocraft_project::BandKind::LowCut,
                freq: 1000.0,
                gain: 0.0,
                q: std::f32::consts::FRAC_1_SQRT_2,
            },
            48_000,
        );
        let out = run(None, vec![(0, hp)]);
        let tail = out[2 * 4000].abs();
        assert!(tail < 1e-3, "high-pass leaves DC: {tail}");
    }

    fn left(out: &[f32]) -> Vec<f32> {
        out.chunks(2).map(|c| c[0]).collect()
    }

    #[test]
    fn automated_send_level_feeds_the_fx_channel() {
        use std::collections::HashMap;
        use studiocraft_project::{AutoParam, MIN_DB, PoolClip, Project, SendSlot, TrackKind};
        let mut p = Project::default();
        let a = p.add_track(TrackKind::Audio, "A");
        let fx = p.add_track(TrackKind::Fx, "Hall");
        let clip = p.add_clip(PoolClip {
            name: "c".into(),
            path: None,
            sample_rate: 48_000,
            frames: 1000,
        });
        p.add_audio_event(a, 0, clip).expect("event");
        // Fader closed: only the pre-fader send reaches the output, through the FX channel.
        p.tracks[a].volume_db = MIN_DB;
        let target = p.tracks[fx].id;
        p.tracks[a].sends.push(SendSlot {
            target,
            level_db: 0.0,
            pre_fader: true,
            on: true,
        });
        p.tracks[a].read = true;
        p.tracks[a]
            .automation_lane_mut(AutoParam::Send { index: 0 })
            .set_point(0, -6.0);
        let mut clips = HashMap::new();
        clips.insert(clip, Arc::<[f32]>::from(vec![1.0f32; 2000]));
        let song = Song::build(&p, &clips, 48_000);
        let (mut core, mut ctl) = engine(48_000, TempoMap::default());
        ctl.send(Command::SetSong(Arc::new(song)));
        ctl.send(Command::Play);
        let mut out = [0.0f32; 64];
        core.process(&mut out);
        let g = studiocraft_project::db_to_gain(-6.0);
        assert!((out[10] - g).abs() < 1e-5, "{}", out[10]);
        assert!((out[11] - g).abs() < 1e-5, "{}", out[11]);
    }

    #[test]
    fn automation_shapes_the_track_while_playing_only() {
        use studiocraft_project::{AutoParam, Project, TrackKind};
        let mut p = Project::default();
        let t = p.add_track(TrackKind::Audio, "A");
        p.tracks[t].read = true;
        p.tracks[t].volume_db = 0.0;
        // -6 dB from the start; mute from tick 1 (= frame 25 at 120 bpm, 48 kHz) on.
        p.tracks[t]
            .automation_lane_mut(AutoParam::Volume)
            .set_point(0, -6.0);
        let mute = p.tracks[t].automation_lane_mut(AutoParam::Mute);
        mute.set_point(0, 0.0);
        mute.set_point(1, 0.0);
        mute.set_point(2, 1.0);
        let auto = crate::automation::TrackAuto::build(&p.tracks[t], &p.tempo, 48_000);
        let mut song = Song::single(vec![1.0f32; 200].into());
        song.tracks[0].auto = auto;
        let (mut core, mut ctl) = engine(48_000, TempoMap::default());
        ctl.send(Command::SetSong(Arc::new(song)));
        ctl.send(Command::Play);
        let mut out = [0.0f32; 160];
        core.process(&mut out);
        let g = studiocraft_project::db_to_gain(-6.0);
        assert!((out[0] - g).abs() < 1e-6, "{}", out[0]);
        assert!((out[2 * 24] - g).abs() < 1e-6);
        // The mute curve crosses 0.5 at frame 37.5: silent from frame 38.
        assert!(out[2 * 37] > 0.0);
        assert_eq!(out[2 * 38], 0.0);
        assert_eq!(out[2 * 79 + 1], 0.0);
    }

    #[test]
    fn monitored_track_plays_the_input_through_its_fader_while_stopped() {
        let (mut core, mut ctl) = engine(48_000, TempoMap::default());
        let (mut tx, input) = crate::monitor::monitor_ring(2, 48_000, SCRATCH_FRAMES);
        ctl.send(Command::SetMonitorInput(Box::new(input)));
        let track = crate::PlayTrack {
            gain_l: 0.5,
            gain_r: 0.5,
            monitor: Some((1, false)),
            ..Default::default()
        };
        let song = Song {
            tracks: vec![track],
            master: 1.0,
            buses: 0,
            tempo: TempoMap::default(),
            midi_ports: Vec::new(),
            click: Default::default(),
            outputs: Vec::new(),
            output_gains: Vec::new(),
        };
        ctl.send(Command::SetSong(Arc::new(song)));
        crate::monitor::push_frames(&mut tx, 2, [0.1, 0.8, 0.2, 0.6].into_iter());
        let mut out = [0.0f32; 6];
        core.process(&mut out);
        assert_eq!(out, [0.4, 0.4, 0.3, 0.3, 0.0, 0.0]);
        // Without monitor the same input stays silent.
        let song = Song {
            tracks: vec![crate::PlayTrack::default()],
            master: 1.0,
            buses: 0,
            tempo: TempoMap::default(),
            midi_ports: Vec::new(),
            click: Default::default(),
            outputs: Vec::new(),
            output_gains: Vec::new(),
        };
        ctl.send(Command::SetSong(Arc::new(song)));
        crate::monitor::push_frames(&mut tx, 2, [0.1, 0.8].into_iter());
        core.process(&mut out);
        assert!(out.iter().all(|&s| s == 0.0));
    }

    #[test]
    fn preview_plays_once_while_stopped_and_can_be_stopped() {
        let (mut core, mut ctl) = engine(48_000, TempoMap::default());
        ctl.send(Command::Preview(Some(vec![0.5f32; 6].into())));
        let mut out = [0.0f32; 4];
        core.process(&mut out);
        assert_eq!(out, [0.5; 4]);
        assert!(ctl.status.previewing());
        core.process(&mut out);
        assert_eq!(out, [0.5, 0.5, 0.0, 0.0], "ends after the last sample");
        assert!(!ctl.status.previewing());
        ctl.send(Command::Preview(Some(vec![0.25f32; 100].into())));
        core.process(&mut out);
        ctl.send(Command::Preview(None));
        core.process(&mut out);
        assert_eq!(out, [0.0; 4]);
        // Old buffers come back to the main thread.
        ctl.collect_garbage();
    }

    #[test]
    fn load_is_published_as_a_share_of_the_block() {
        let (mut core, ctl) = engine(48_000, TempoMap::default());
        let mut out = [0.0f32; 512];
        core.process(&mut out);
        let l = ctl.status.load();
        assert!(l.is_finite() && l >= 0.0, "{l}");
        // An empty block has no duration: the value stays as it was.
        core.process(&mut []);
        assert_eq!(ctl.status.load(), l);
    }

    #[test]
    fn crossfaded_events_blend_in_the_overlap() {
        use std::collections::HashMap;
        use studiocraft_project::{PoolClip, Project, TrackKind};
        let mut p = Project::default();
        let t = p.add_track(TrackKind::Audio, "A");
        let mut clips = HashMap::new();
        let mut add = |p: &mut Project, level: f32| {
            let c = p.add_clip(PoolClip {
                name: "c".into(),
                path: None,
                sample_rate: 48_000,
                frames: 2000,
            });
            clips.insert(c, Arc::<[f32]>::from(vec![level; 4000]));
            c
        };
        // 120 bpm, 48 kHz: one tick is 25 frames.
        let ca = add(&mut p, 1.0);
        let cb = add(&mut p, 0.5);
        let a = p.add_audio_event(t, 0, ca).expect("a");
        let b = p.add_audio_event(t, 40, cb).expect("b");
        assert!(p.crossfade(a, b));
        let song = Song::build(&p, &clips, 48_000);
        let (mut core, mut ctl) = engine(48_000, TempoMap::default());
        ctl.send(Command::SetSong(Arc::new(song)));
        ctl.send(Command::Play);
        let mut out = vec![0.0f32; 2 * 2400];
        core.process(&mut out);
        let at = |f: usize| out[2 * f];
        assert!((at(500) - 1.0).abs() < 1e-3, "before the overlap: a alone");
        // Overlap from frame 1000 to 2000; in the middle both at half gain.
        assert!((at(1500) - 0.75).abs() < 0.01, "{}", at(1500));
        assert!((at(2200) - 0.5).abs() < 1e-3, "after the overlap: b alone");
    }

    #[test]
    fn channel_eq_shapes_the_track_before_the_fader() {
        let mut eq = studiocraft_project::ChannelEq::default();
        eq.bands[2].gain = 6.0;
        eq.bands[2].freq = 1000.0;
        let samples: Vec<f32> = (0..48_000)
            .flat_map(|i| {
                let v = (std::f32::consts::TAU * 1000.0 * i as f32 / 48_000.0).sin() * 0.25;
                [v, v]
            })
            .collect();
        let mut song = Song::single(samples.into());
        song.tracks[0].eq = crate::eq::design(&eq, 48_000);
        let (mut core, mut ctl) = engine(48_000, TempoMap::default());
        ctl.send(Command::SetSong(Arc::new(song)));
        ctl.send(Command::Play);
        let mut out = vec![0.0f32; 2 * 4800];
        for _ in 0..5 {
            core.process(&mut out);
        }
        let peak = out.chunks(2).map(|f| f[0].abs()).fold(0.0, f32::max);
        assert!((peak - 0.5).abs() < 0.01, "{peak}");
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
    fn keyboard_notes_are_recorded_with_position() {
        let (mut core, mut ctl) = ramp(10);
        ctl.send(Command::SetMidiTarget(Some(0)));
        ctl.send(Command::Locate(1000));
        ctl.send(Command::Record);
        core.process(&mut [0.0; 8]);
        {
            let mut m = ctl.midi.lock().unwrap();
            m.push(crate::midi::MidiMsg::Note {
                key: 60,
                velocity: 90,
                on: true,
            })
            .unwrap();
        }
        core.process(&mut [0.0; 8]);
        let r = ctl.recorded_midi.pop().unwrap();
        assert_eq!((r.pos, r.key, r.velocity, r.on), (1004, 60, 90, true));
        assert!(r.recording && r.running);
        // Ohne Aufnahme wird gemeldet, aber nicht als Aufnahme (nachträgliche Aufnahme).
        ctl.send(Command::Stop);
        core.process(&mut [0.0; 8]);
        ctl.midi
            .lock()
            .unwrap()
            .push(crate::midi::MidiMsg::Note {
                key: 62,
                velocity: 90,
                on: true,
            })
            .unwrap();
        core.process(&mut [0.0; 8]);
        let r = ctl.recorded_midi.pop().unwrap();
        assert_eq!(r.key, 62);
        assert!(!r.recording && !r.running);
        assert!(r.clock > 0, "the free-running clock counts while stopped");
        assert!(ctl.recorded_midi.pop().is_err());
    }

    #[test]
    fn sustain_pedal_delays_recorded_note_off() {
        use crate::midi::MidiMsg;
        let (mut core, mut ctl) = ramp(10);
        ctl.send(Command::SetMidiTarget(Some(0)));
        ctl.send(Command::Record);
        core.process(&mut [0.0; 8]);
        let send = |ctl: &mut EngineControl, m: MidiMsg| ctl.midi.lock().unwrap().push(m).unwrap();
        send(
            &mut ctl,
            MidiMsg::Note {
                key: 60,
                velocity: 80,
                on: true,
            },
        );
        send(&mut ctl, MidiMsg::Other([0xb0, crate::midi::SUSTAIN, 127]));
        send(
            &mut ctl,
            MidiMsg::Note {
                key: 60,
                velocity: 0,
                on: false,
            },
        );
        core.process(&mut [0.0; 8]); // Position 4
        core.process(&mut [0.0; 8]); // Position 8
        send(&mut ctl, MidiMsg::Other([0xb0, crate::midi::SUSTAIN, 0]));
        core.process(&mut [0.0; 8]);
        let mut got = Vec::new();
        while let Ok(r) = ctl.recorded_midi.pop() {
            got.push((r.pos, r.key, r.on));
        }
        assert_eq!(
            got,
            [(4, 60, true), (12, 60, false)],
            "Aus erst beim Lösen des Pedals"
        );
    }

    #[test]
    fn metronome_clicks_on_beats_with_accent() {
        // 120 BPM, 4/4, 48 kHz: Zählzeit alle 24 000 Frames.
        let (mut core, mut ctl) = engine(48_000, TempoMap::default());
        ctl.send(Command::SetMetronome(true));
        ctl.send(Command::Play);
        let mut out = vec![0.0f32; 2 * 120_000];
        for block in out.chunks_mut(2 * 1000) {
            core.process(block);
        }
        let left: Vec<f32> = out.chunks(2).map(|f| f[0]).collect();
        // Klick beginnt genau auf den Zählzeiten 0, 24 000, 48 000 …, dazwischen Stille.
        for beat in 0..5 {
            let at = beat * 24_000;
            let energy: f32 = left[at..at + 100].iter().map(|v| v.abs()).sum();
            assert!(energy > 1.0, "Klick auf Zählzeit {beat}");
            let gap: f32 = left[at + 2_000..at + 20_000].iter().map(|v| v.abs()).sum();
            assert!(gap < 1e-6, "Stille nach Zählzeit {beat}");
        }
        // Takt-Eins höher: mehr Nulldurchgänge in den ersten 10 ms.
        let crossings = |s: &[f32]| {
            s.windows(2)
                .filter(|w| w[0].signum() != w[1].signum())
                .count()
        };
        assert!(crossings(&left[0..480]) > crossings(&left[24_000..24_480]));
        // Aus: Stille.
        ctl.send(Command::SetMetronome(false));
        let mut quiet = vec![0.0f32; 2 * 30_000];
        core.process(&mut quiet);
        assert!(quiet.iter().all(|v| *v == 0.0));
    }

    #[test]
    fn metronome_follows_a_tempo_jump() {
        use std::sync::Arc;
        // 120 BPM for one bar (beats every 24 000 frames), then 60 BPM (every 48 000).
        let mut tempo = TempoMap {
            use_track: true,
            ..TempoMap::default()
        };
        tempo.set_event(tempo.signature.bar_ticks(), 60.0, false);
        let (mut core, mut ctl) = engine(48_000, tempo.clone());
        ctl.send(Command::SetSong(Arc::new(Song {
            tempo,
            ..Song::default()
        })));
        ctl.send(Command::SetMetronome(true));
        ctl.send(Command::Play);
        let mut out = vec![0.0f32; 2 * 250_000];
        for block in out.chunks_mut(2 * 1000) {
            core.process(block);
        }
        let left: Vec<f32> = out.chunks(2).map(|f| f[0]).collect();
        let energy = |at: usize| left[at..at + 100].iter().map(|v| v.abs()).sum::<f32>();
        // Bar 1: 0, 24k, 48k, 72k; bar 2 starts at 96k, then every 48k.
        for at in [0, 24_000, 48_000, 72_000, 96_000, 144_000, 192_000, 240_000] {
            assert!(energy(at) > 1.0, "click at {at}");
        }
        // No click where the old tempo would have put one.
        assert!(energy(120_000) < 1e-6, "no click at 120 000 after the jump");
    }

    #[test]
    fn precount_clicks_then_records_exactly_at_its_end() {
        use std::sync::Arc;
        // Two bars of 4/4 at 120 BPM: 8 beats of 24 000 frames, then recording starts.
        let (mut core, mut ctl) = engine(48_000, TempoMap::default());
        let click = studiocraft_project::ClickSettings {
            precount_bars: 2,
            ..Default::default()
        };
        ctl.send(Command::SetSong(Arc::new(Song {
            click,
            ..Song::default()
        })));
        ctl.send(Command::RecordAfterCount {
            frames: 192_000,
            beat: 24_000,
            beats_per_bar: 4,
        });
        let mut out = vec![0.0f32; 2 * 191_000];
        for block in out.chunks_mut(2 * 1000) {
            core.process(block);
        }
        assert!(!ctl.status.rec_started(), "still counting");
        assert_eq!(ctl.status.state(), TransportState::Stopped);
        assert_eq!(ctl.status.cursor(), 0, "the cursor waits");
        assert_eq!(ctl.status.count_in(), 1_000);
        let left: Vec<f32> = out.chunks(2).map(|f| f[0]).collect();
        for beat in 0..8 {
            let at = beat * 24_000;
            let e: f32 = left[at..at + 100].iter().map(|v| v.abs()).sum();
            assert!(e > 1.0, "click on beat {beat}");
        }
        // The block that ends the count-in starts recording 1 000 frames into itself.
        let mut tail = vec![0.0f32; 2 * 1_500];
        core.process(&mut tail);
        assert!(ctl.status.rec_started());
        assert_eq!(ctl.status.state(), TransportState::Recording);
        assert_eq!(ctl.status.rec_start(), 0);
        assert_eq!(
            ctl.status.cursor(),
            500,
            "500 frames recorded after the count-in"
        );
        // Stop during a count-in cancels it.
        ctl.send(Command::Stop);
        ctl.send(Command::Stop);
        core.process(&mut [0.0; 2]);
        ctl.send(Command::RecordAfterCount {
            frames: 96_000,
            beat: 24_000,
            beats_per_bar: 4,
        });
        core.process(&mut [0.0; 2000]);
        ctl.send(Command::Stop);
        core.process(&mut [0.0; 2000]);
        assert!(!ctl.status.rec_started());
        assert_eq!(ctl.status.count_in(), 0);
        assert_eq!(ctl.status.state(), TransportState::Stopped);
    }

    #[test]
    fn click_level_sound_and_accent_follow_the_project_settings() {
        use std::sync::Arc;
        let render = |click: studiocraft_project::ClickSettings| {
            let (mut core, mut ctl) = engine(48_000, TempoMap::default());
            ctl.send(Command::SetSong(Arc::new(Song {
                click,
                ..Song::default()
            })));
            ctl.send(Command::SetMetronome(true));
            ctl.send(Command::Play);
            let mut out = vec![0.0f32; 2 * 30_000];
            core.process(&mut out);
            out.chunks(2).map(|f| f[0]).collect::<Vec<f32>>()
        };
        let peak = |s: &[f32]| s.iter().fold(0.0f32, |m, v| m.max(v.abs()));
        let crossings = |s: &[f32]| {
            s.windows(2)
                .filter(|w| w[0].signum() != w[1].signum())
                .count()
        };
        let base = render(Default::default());
        let quiet = render(studiocraft_project::ClickSettings {
            level_db: -26.0,
            ..Default::default()
        });
        assert!(
            peak(&quiet[..1000]) < peak(&base[..1000]) * 0.2,
            "20 dB quieter"
        );
        let dark = render(studiocraft_project::ClickSettings {
            sound: studiocraft_project::ClickSound::Dark,
            ..Default::default()
        });
        assert!(
            crossings(&dark[..480]) < crossings(&base[..480]),
            "lower pitch"
        );
        let flat = render(studiocraft_project::ClickSettings {
            accent: false,
            ..Default::default()
        });
        // Without accent the downbeat sounds like the other beats.
        assert_eq!(crossings(&flat[..480]), crossings(&flat[24_000..24_480]));
        assert!(crossings(&base[..480]) > crossings(&base[24_000..24_480]));
    }

    #[test]
    fn delay_compensation_shifts_a_track_without_latency() {
        use crate::delay::{Compensation, DelayBank};
        use crate::song::{PlayEvent, PlayTrack};
        use std::sync::Arc;
        // Two tracks with an impulse at frame 0; track 1 "has" 100 frames of plugin latency,
        // so track 0 must come 100 frames later. (Without a plugin track 1 itself is on time.)
        let mut imp = vec![0.0f32; 2 * 400];
        imp[0] = 1.0;
        imp[1] = 1.0;
        let samples: Arc<[f32]> = imp.into();
        let t = |slot| PlayTrack {
            gain_l: 1.0,
            gain_r: 1.0,
            slot,
            events: vec![PlayEvent::new(0, 400, 0, samples.clone())],
            ..PlayTrack::default()
        };
        let song = Song {
            tracks: vec![t(0), t(1)],
            master: 1.0,
            ..Song::default()
        };
        let comp = Compensation::compute(&song, &[0, 100], 2);
        assert_eq!(comp.total, 100);
        let (mut core, mut ctl) = engine(48_000, TempoMap::default());
        ctl.send(Command::SetSong(Arc::new(song)));
        ctl.send(Command::SetDelays(
            Box::new(DelayBank::new(&comp)),
            comp.total,
        ));
        ctl.send(Command::Play);
        let mut out = vec![0.0f32; 2 * 300];
        core.process(&mut out);
        let left: Vec<f32> = out.chunks(2).map(|f| f[0]).collect();
        assert_eq!(left[0], 1.0, "track 1 on time");
        assert_eq!(left[100], 1.0, "track 0 delayed by 100 frames");
        assert_eq!(left.iter().filter(|v| v.abs() > 0.0).count(), 2);
        assert_eq!(ctl.status.latency(), 100);
    }

    #[test]
    fn a_track_on_another_output_bus_skips_the_main_mix() {
        use crate::song::{Output, PlayEvent, PlayTrack};
        use std::sync::Arc;
        let samples: Arc<[f32]> = vec![0.5f32; 2 * 64].into();
        let t = |slot, output| PlayTrack {
            gain_l: 1.0,
            gain_r: 1.0,
            slot,
            output,
            events: vec![PlayEvent::new(0, 64, 0, samples.clone())],
            ..PlayTrack::default()
        };
        let song = Song {
            tracks: vec![t(0, Output::Master), t(1, Output::Hw(1))],
            master: 1.0,
            outputs: vec![(0, true), (2, true)],
            ..Song::default()
        };
        let (mut core, mut ctl) = engine(48_000, TempoMap::default());
        ctl.send(Command::SetSong(Arc::new(song)));
        ctl.send(Command::Play);
        let mut out = vec![0.0f32; 2 * 32];
        core.process(&mut out);
        assert!(
            out.iter().all(|&v| (v - 0.5).abs() < 1e-6),
            "main mix: only track 0"
        );
        assert_eq!(core.output_layout(), &[(0, true), (2, true)]);
        assert_eq!(core.hw_block(1).len(), 64);
        assert!(core.hw_block(1).iter().all(|&v| (v - 0.5).abs() < 1e-6));
        assert!(core.hw_block(2).iter().all(|&v| v == 0.0));
        assert!(core.hw_block(99).is_empty());
    }

    #[test]
    fn output_bus_strips_scale_their_bus_and_report_peaks() {
        use crate::song::{Output, PlayEvent, PlayTrack};
        use std::sync::Arc;
        let samples: Arc<[f32]> = vec![0.5f32; 2 * 64].into();
        let run = |gains: Vec<f32>| {
            let song = Song {
                tracks: vec![PlayTrack {
                    gain_l: 1.0,
                    gain_r: 1.0,
                    output: Output::Hw(1),
                    events: vec![PlayEvent::new(0, 64, 0, samples.clone())],
                    ..PlayTrack::default()
                }],
                master: 1.0,
                outputs: vec![(0, true), (2, true)],
                output_gains: gains,
                ..Song::default()
            };
            let (mut core, mut ctl) = engine(48_000, TempoMap::default());
            ctl.send(Command::SetSong(Arc::new(song)));
            ctl.send(Command::Play);
            core.process(&mut [0.0f32; 2 * 32]);
            (core.hw_block(1).to_vec(), ctl.meters.take_output(1))
        };
        let (block, peaks) = run(vec![1.0, 0.5]);
        assert!(block.iter().all(|&v| (v - 0.25).abs() < 1e-6), "{block:?}");
        assert_eq!(peaks, [0.25, 0.25]);
        let (block, peaks) = run(vec![1.0, 0.0]);
        assert!(block.iter().all(|&v| v == 0.0), "muted bus is silent");
        assert_eq!(peaks, [0.0, 0.0]);
        let (block, _) = run(Vec::new());
        assert!(
            block.iter().all(|&v| (v - 0.5).abs() < 1e-6),
            "missing gain = 1"
        );
    }

    #[test]
    fn song_takes_bus_strip_gain_and_mute_from_the_project() {
        let mut p = studiocraft_project::Project::default();
        let a = p.io.add(true, true).unwrap_or_default();
        let b = p.io.add(true, true).unwrap_or_default();
        for bus in &mut p.io.outputs {
            if bus.id == a {
                bus.gain_db = -6.0;
            }
            if bus.id == b {
                bus.mute = true;
            }
        }
        let song = Song::build(&p, &Default::default(), 48_000);
        assert_eq!(song.output_gains.len(), 3);
        assert_eq!(song.output_gains[0], 1.0);
        assert!((song.output_gains[1] - studiocraft_project::db_to_gain(-6.0)).abs() < 1e-6);
        assert_eq!(song.output_gains[2], 0.0);
    }

    /// Projekt mit einer Audio-Spur, deren Clip konstant 1,0 liefert, plus Bussen.
    fn routed(setup: impl FnOnce(&mut studiocraft_project::Project)) -> f32 {
        use std::collections::HashMap;
        use studiocraft_project::{PoolClip, Project, TrackKind};
        let mut p = Project::default();
        p.add_track(TrackKind::Audio, "Quelle");
        let c = p.add_clip(PoolClip {
            name: "c".into(),
            path: None,
            sample_rate: 48_000,
            frames: 48_000,
        });
        p.add_audio_event(0, 0, c).unwrap();
        setup(&mut p);
        let clips = HashMap::from([(c, vec![1.0f32; 96_000].into())]);
        let song = Song::build(&p, &clips, 48_000);
        let (mut core, mut ctl) = engine(48_000, TempoMap::default());
        ctl.send(Command::SetSong(Arc::new(song)));
        ctl.send(Command::Play);
        let mut out = [0.0f32; 64];
        core.process(&mut out);
        out[10]
    }

    #[test]
    fn extra_outputs_send_the_same_signal_to_several_destinations() {
        use studiocraft_project::{OutTarget, TrackKind};
        // Main mix and a group at −6 dB: 1 + 0.5.
        let v = routed(|p| {
            let g = p.add_track(TrackKind::Group, "Gruppe");
            p.tracks[g].volume_db = -6.0206;
            let id = p.tracks[g].id;
            p.tracks[0].extra_outputs.push(OutTarget::Group(id));
        });
        assert!((v - 1.5).abs() < 1e-3, "{v}");
        // Output to the group plus the main mix as an extra destination: the same.
        let v = routed(|p| {
            let g = p.add_track(TrackKind::Group, "Gruppe");
            p.tracks[g].volume_db = -6.0206;
            p.tracks[0].output = Some(p.tracks[g].id);
            p.tracks[0]
                .extra_outputs
                .push(OutTarget::Bus(studiocraft_project::MAIN_OUT));
        });
        assert!((v - 1.5).abs() < 1e-3, "{v}");
        // The fader applies to every destination; a doubled destination counts once.
        let v = routed(|p| {
            let g = p.add_track(TrackKind::Group, "Gruppe");
            let id = p.tracks[g].id;
            p.tracks[0].volume_db = -6.0206;
            p.tracks[0].extra_outputs.push(OutTarget::Group(id));
            p.tracks[0].extra_outputs.push(OutTarget::Group(id));
        });
        assert!((v - 1.0).abs() < 1e-3, "{v}");
        // A muted group as extra destination: only the main mix sounds.
        let v = routed(|p| {
            let g = p.add_track(TrackKind::Group, "Gruppe");
            p.tracks[g].mute = true;
            let id = p.tracks[g].id;
            p.tracks[0].extra_outputs.push(OutTarget::Group(id));
        });
        assert_eq!(v, 1.0);
    }

    #[test]
    fn extra_outputs_reach_further_output_buses() {
        use std::collections::HashMap;
        use studiocraft_project::{OutTarget, PoolClip, Project, TrackKind};
        let mut p = Project::default();
        let bus = p.io.add(true, true).unwrap();
        p.add_track(TrackKind::Audio, "Quelle");
        let c = p.add_clip(PoolClip {
            name: "c".into(),
            path: None,
            sample_rate: 48_000,
            frames: 48_000,
        });
        p.add_audio_event(0, 0, c).unwrap();
        p.tracks[0].extra_outputs.push(OutTarget::Bus(bus));
        let clips = HashMap::from([(c, vec![1.0f32; 96_000].into())]);
        let song = Song::build(&p, &clips, 48_000);
        assert_eq!(song.tracks[0].extra, vec![Output::Hw(1)]);
        let (mut core, mut ctl) = engine(48_000, TempoMap::default());
        ctl.send(Command::SetSong(Arc::new(song)));
        ctl.send(Command::Play);
        let mut out = [0.0f32; 64];
        core.process(&mut out);
        assert_eq!(out[10], 1.0, "main mix");
        assert_eq!(core.hw_block(1)[10], 1.0, "further output bus");
    }

    #[test]
    fn sends_groups_and_solo_safe_buses() {
        use studiocraft_project::{SendSlot, TrackKind};
        assert_eq!(routed(|_| {}), 1.0);
        // Post-Fader-Send 0 dB an einen FX-Kanal mit −6 dB: 1 + 0,5.
        let v = routed(|p| {
            let fx = p.add_track(TrackKind::Fx, "Hall");
            p.tracks[fx].volume_db = -6.0206;
            let id = p.tracks[fx].id;
            p.tracks[0].sends.push(SendSlot {
                target: id,
                level_db: 0.0,
                pre_fader: false,
                on: true,
            });
        });
        assert!((v - 1.5).abs() < 1e-3, "{v}");
        // Pre-Fader: Quell-Fader ganz zu, Send bleibt.
        let v = routed(|p| {
            let fx = p.add_track(TrackKind::Fx, "Hall");
            let id = p.tracks[fx].id;
            p.tracks[0].volume_db = studiocraft_project::MIN_DB;
            p.tracks[0].sends.push(SendSlot {
                target: id,
                level_db: 0.0,
                pre_fader: true,
                on: true,
            });
        });
        assert_eq!(v, 1.0, "nur der Pre-Fader-Send ist zu hören");
        // Ausgang auf eine Gruppe mit −6 dB.
        let v = routed(|p| {
            let g = p.add_track(TrackKind::Group, "Gruppe");
            p.tracks[g].volume_db = -6.0206;
            p.tracks[0].output = Some(p.tracks[g].id);
        });
        assert!((v - 0.5).abs() < 1e-3, "{v}");
        // Gruppe stumm: nichts zu hören (nicht an der Gruppe vorbei zur Summe).
        let v = routed(|p| {
            let g = p.add_track(TrackKind::Group, "Gruppe");
            p.tracks[g].mute = true;
            p.tracks[0].output = Some(p.tracks[g].id);
        });
        assert_eq!(v, 0.0);
        // Solo auf der Quelle: FX-Kanal bleibt hörbar.
        let v = routed(|p| {
            let fx = p.add_track(TrackKind::Fx, "Hall");
            let id = p.tracks[fx].id;
            p.add_track(TrackKind::Audio, "andere");
            p.tracks[0].solo = true;
            p.tracks[0].sends.push(SendSlot {
                target: id,
                level_db: 0.0,
                pre_fader: false,
                on: true,
            });
        });
        assert_eq!(v, 2.0);
        // Verschachtelt: Quelle → Gruppe A → Gruppe B → Summe (Reihenfolge stimmt).
        let v = routed(|p| {
            let a = p.add_track(TrackKind::Group, "B (später angelegt, aber Ziel)");
            let b = p.add_track(TrackKind::Group, "A");
            let (ia, ib) = (p.tracks[a].id, p.tracks[b].id);
            p.tracks[0].output = Some(ib);
            p.tracks[b].output = Some(ia);
        });
        assert_eq!(v, 1.0);
        // Schleife (sollte die Oberfläche verhindern): kein Hänger, Signal kommt an.
        let v = routed(|p| {
            let a = p.add_track(TrackKind::Group, "A");
            let b = p.add_track(TrackKind::Group, "B");
            let (ia, ib) = (p.tracks[a].id, p.tracks[b].id);
            p.tracks[0].output = Some(ia);
            p.tracks[a].output = Some(ib);
            p.tracks[b].output = Some(ia);
        });
        assert!(v.is_finite());
    }

    #[test]
    fn bar_to_frames_at_120_bpm() {
        let (_core, ctl) = ramp(1);
        let bar2 = ctl.tempo.position(2, 1);
        assert_eq!(ctl.frames(bar2), 96_000);
        assert_eq!(ctl.position(96_000), bar2);
    }
}
