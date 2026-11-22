//! MIDI-Ausgang zu externen Geräten (Spec „Spurtypen“: MIDI-Spur; Issue #12).
//!
//! Der Audio-Thread darf nicht senden (das ist I/O). Er legt jede Nachricht mit ihrem
//! Fälligkeitszeitpunkt in einen lock-freien Puffer: Zeitpunkt, zu dem der Block hörbar wird
//! (inklusive Ausgabelatenz), plus Versatz im Block. Ein eigener Thread liest den Puffer,
//! wartet bis zur Fälligkeit und sendet über midir. So klingt ein externer Klangerzeuger
//! gleichzeitig mit dem Audio aus der Software (Latenzausgleich).

use std::collections::HashMap;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc;
use std::thread::JoinHandle;
use std::time::Duration;

/// Eine geplante MIDI-Nachricht: Fälligkeit in [`crate::clock::now_nanos`]-Zeit, Port-Platz
/// (Index in der Port-Liste des Songs) und Nachricht.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct OutMsg {
    pub due: u64,
    pub port: u8,
    pub data: [u8; 3],
}

/// Größe des Puffers zwischen Audio-Thread und Ausgabe-Thread.
pub const OUT_RING: usize = 8192;
/// Höchstens so viele Nachrichten warten im Ausgabe-Thread (Schutz vor Überlauf).
const MAX_PENDING: usize = 65_536;

/// Befehle der Oberfläche an den Ausgabe-Thread.
enum OutCmd {
    /// Port-Namen in der Reihenfolge der Plätze (aus dem Song).
    Ports(Vec<String>),
    /// Sofort an einen Port senden (z. B. Programmwechsel).
    Now(String, [u8; 3]),
}

/// Der laufende Ausgabe-Thread. Beim Drop hält er an.
pub struct MidiOutThread {
    cmd: mpsc::Sender<OutCmd>,
    stop: Arc<AtomicBool>,
    handle: Option<JoinHandle<()>>,
}

/// Namen der vorhandenen MIDI-Ausgänge (ohne Durchschleif-Ports).
pub fn output_ports() -> Vec<String> {
    let Ok(out) = crate::midi_backend::MidiOutput::new("StudioCraft") else {
        return Vec::new();
    };
    out.ports()
        .iter()
        .filter_map(|p| out.port_name(p).ok())
        .filter(|n| !n.to_lowercase().contains("through"))
        .collect()
}

impl MidiOutThread {
    /// Startet den Thread, der `rx` (vom Audio-Thread) abarbeitet.
    pub fn spawn(rx: rtrb::Consumer<OutMsg>) -> Result<Self, crate::EngineError> {
        let (cmd, cmd_rx) = mpsc::channel();
        let stop = Arc::new(AtomicBool::new(false));
        let flag = stop.clone();
        let handle = std::thread::Builder::new()
            .name("midi-out".into())
            .spawn(move || run(rx, cmd_rx, flag))
            .map_err(|e| crate::EngineError::Midi {
                output: true,
                detail: e.to_string(),
            })?;
        Ok(Self {
            cmd,
            stop,
            handle: Some(handle),
        })
    }

    /// Port-Namen in der Reihenfolge, in der der Song sie nummeriert.
    pub fn set_ports(&self, ports: Vec<String>) {
        let _ = self.cmd.send(OutCmd::Ports(ports));
    }

    /// Sendet `data` sofort an `port` (nicht aus dem Audio-Thread).
    pub fn send_now(&self, port: &str, data: [u8; 3]) {
        let _ = self.cmd.send(OutCmd::Now(port.to_owned(), data));
    }
}

impl Drop for MidiOutThread {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Relaxed);
        if let Some(h) = self.handle.take() {
            let _ = h.join();
        }
    }
}

/// Offene Verbindungen, nach Port-Name; eine fehlgeschlagene wird erst nach einer Sekunde
/// erneut versucht.
struct Connections {
    open: HashMap<String, crate::midi_backend::MidiOutputConnection>,
    failed: HashMap<String, u64>,
}

impl Connections {
    fn send(&mut self, port: &str, data: &[u8]) {
        if !self.open.contains_key(port) {
            let now = crate::clock::now_nanos();
            if self
                .failed
                .get(port)
                .is_some_and(|&t| now.saturating_sub(t) < 1_000_000_000)
            {
                return;
            }
            match connect(port) {
                Some(c) => {
                    self.open.insert(port.to_owned(), c);
                }
                None => {
                    self.failed.insert(port.to_owned(), now);
                    return;
                }
            }
        }
        if let Some(c) = self.open.get_mut(port)
            && c.send(data).is_err()
        {
            self.open.remove(port);
        }
    }
}

fn connect(name: &str) -> Option<crate::midi_backend::MidiOutputConnection> {
    let out = crate::midi_backend::MidiOutput::new("StudioCraft").ok()?;
    let port = out
        .ports()
        .into_iter()
        .find(|p| out.port_name(p).is_ok_and(|n| n == name))?;
    out.connect(&port, "StudioCraft Ausgang").ok()
}

fn run(mut rx: rtrb::Consumer<OutMsg>, cmd: mpsc::Receiver<OutCmd>, stop: Arc<AtomicBool>) {
    let mut ports: Vec<String> = Vec::new();
    let mut conns = Connections {
        open: HashMap::new(),
        failed: HashMap::new(),
    };
    let mut pending: Vec<OutMsg> = Vec::with_capacity(4096);
    while !stop.load(Ordering::Relaxed) {
        while let Ok(c) = cmd.try_recv() {
            match c {
                OutCmd::Ports(p) => ports = p,
                OutCmd::Now(port, data) => conns.send(&port, &data[..len(data[0])]),
            }
        }
        let before = pending.len();
        while let Ok(m) = rx.pop() {
            if pending.len() < MAX_PENDING {
                pending.push(m);
            }
        }
        if pending.len() != before {
            pending.sort_by_key(|m| m.due);
        }
        let now = crate::clock::now_nanos();
        let due = pending.partition_point(|m| m.due <= now);
        for m in pending.drain(..due) {
            if let Some(name) = ports.get(usize::from(m.port)) {
                conns.send(name, &m.data[..len(m.data[0])]);
            }
        }
        let wait = pending
            .first()
            .map_or(2_000_000, |m| m.due.saturating_sub(now))
            .clamp(200_000, 2_000_000);
        std::thread::sleep(Duration::from_nanos(wait));
    }
}

/// Länge einer Kanalnachricht nach ihrem Status-Byte (Programm und Aftertouch: zwei Bytes).
fn len(status: u8) -> usize {
    match status & 0xf0 {
        0xc0 | 0xd0 => 2,
        _ => 3,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn message_lengths() {
        assert_eq!(len(0x90), 3);
        assert_eq!(len(0xb5), 3);
        assert_eq!(len(0xc0), 2);
        assert_eq!(len(0xdf), 2);
    }
}
