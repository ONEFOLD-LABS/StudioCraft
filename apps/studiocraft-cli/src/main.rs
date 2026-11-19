//! Audio-Durchstich: spielt eine WAV-Datei mit Transport und Cycle ab.
//! Läuft über cpal auf Linux, FreeBSD, Windows und macOS.
// Kein unwrap/expect/panic außerhalb von Tests (AGENTS.md „Robustness“, Issue #49).
#![cfg_attr(
    not(test),
    deny(
        clippy::unwrap_used,
        clippy::expect_used,
        clippy::panic,
        clippy::todo,
        clippy::unimplemented,
        clippy::unreachable
    )
)]

use std::{
    io::{self, BufRead, Write},
    process::ExitCode,
    sync::Arc,
    thread,
    time::Duration,
};

use studiocraft_engine::{
    AudioClip, Command, EngineControl, PreparedOutput, Song, TransportState, engine, output,
};
use studiocraft_project::TempoMap;

const USAGE: &str = "\
Aufruf: studiocraft <datei.wav> [--bpm <tempo>] [--loop <von_takt> <bis_takt>] [--seconds <s>]
                   [--audio-host <name>]
       studiocraft --list-hosts

  --bpm      Projekttempo für die Takt-Anzeige (Standard 120)
  --loop     Locatoren auf Takt A bis B setzen und Cycle einschalten
  --seconds  sofort starten, nach s Sekunden beenden (ohne Eingabe)
  --audio-host  Audio-Host wählen, z. B. PipeWire, ALSA, JACK, WASAPI, ASIO, CoreAudio
  --list-hosts  einkompilierte Audio-Hosts anzeigen

Befehle im laufenden Programm (jeweils mit Enter):
  <Enter>    Start/Stopp
  s          Stopp (zweimal: zum Anfang)
  c          Cycle an/aus
  l A B      Locatoren auf Takt A bis B
  g T        zu Takt T springen
  q          beenden";

struct Args {
    file: String,
    bpm: f64,
    loop_bars: Option<(u64, u64)>,
    seconds: Option<f64>,
    host: Option<String>,
}

fn parse_args() -> Result<Args, String> {
    let mut it = std::env::args().skip(1);
    let mut file = None;
    let mut args = Args {
        file: String::new(),
        bpm: 120.0,
        loop_bars: None,
        seconds: None,
        host: None,
    };
    let num = |v: Option<String>, what: &str| -> Result<f64, String> {
        v.ok_or(format!("{what} fehlt"))?
            .parse()
            .map_err(|_| format!("{what} ist keine Zahl"))
    };
    while let Some(a) = it.next() {
        match a.as_str() {
            "-h" | "--help" => return Err(String::new()),
            "-V" | "--version" => {
                println!("studiocraft-cli {}", env!("CARGO_PKG_VERSION"));
                std::process::exit(0);
            }
            "--bpm" => args.bpm = num(it.next(), "--bpm")?,
            "--audio-host" => args.host = Some(it.next().ok_or("--audio-host: Name fehlt")?),
            "--list-hosts" => {
                println!("{}", output::available_hosts().join("\n"));
                std::process::exit(0);
            }
            "--seconds" => args.seconds = Some(num(it.next(), "--seconds")?),
            "--loop" => {
                let a = num(it.next(), "--loop von")? as u64;
                let b = num(it.next(), "--loop bis")? as u64;
                args.loop_bars = Some((a, b));
            }
            _ if file.is_none() && !a.starts_with('-') => file = Some(a),
            _ => return Err(format!("Unbekanntes Argument: {a}")),
        }
    }
    args.file = file.ok_or("Keine WAV-Datei angegeben")?;
    if args.bpm <= 0.0 {
        return Err("--bpm muss größer als 0 sein".into());
    }
    Ok(args)
}

fn set_loop(ctl: &mut EngineControl, from: u64, to: u64) {
    let l = ctl.frames(ctl.tempo.position(from.max(1), 1));
    let r = ctl.frames(ctl.tempo.position(to.max(1), 1));
    ctl.send(Command::SetLocators(l, r));
}

fn status_line(ctl: &EngineControl) -> String {
    let cursor = ctl.status.cursor();
    let (bar, beat, six, tick) = ctl.tempo.bars_beats(ctl.position(cursor));
    let secs = cursor as f64 / f64::from(ctl.sample_rate);
    let state = match ctl.status.state() {
        TransportState::Stopped => "Stopp",
        TransportState::Playing => "Wiedergabe",
        TransportState::Recording => "Aufnahme",
    };
    let cycle = if ctl.status.cycle() {
        "Cycle an"
    } else {
        "Cycle aus"
    };
    format!("{bar}.{beat}.{six}.{tick:03}  ({secs:.2} s)  {state}, {cycle}")
}

fn main() -> ExitCode {
    let args = match parse_args() {
        Ok(a) => a,
        Err(e) => {
            if !e.is_empty() {
                eprintln!("Fehler: {e}\n");
            }
            eprintln!("{USAGE}");
            return if e.is_empty() {
                ExitCode::SUCCESS
            } else {
                ExitCode::from(2)
            };
        }
    };

    let clip = match AudioClip::load_wav(&args.file) {
        Ok(c) => c,
        Err(e) => {
            eprintln!("Fehler: {}: {e}", args.file);
            return ExitCode::FAILURE;
        }
    };
    let length = clip.frames() as f64 / f64::from(clip.sample_rate);
    println!("{}: {:.2} s, {} Hz", args.file, length, clip.sample_rate);

    let prepared = match PreparedOutput::open(args.host.as_deref(), clip.sample_rate) {
        Ok(p) => p,
        Err(e) => {
            eprintln!("Fehler: {e}");
            return ExitCode::FAILURE;
        }
    };
    let info = prepared.info.clone();
    let block = info
        .block_frames
        .map_or("Treiber-Standard".into(), |b| format!("{b} Frames"));
    println!(
        "Ausgabe: {} über {}, {} Hz, {} Kanäle, {}, Block {block}",
        info.device, info.host, info.sample_rate, info.channels, info.sample_format
    );
    if info.sample_rate != clip.sample_rate {
        println!(
            "Datei wird von {} auf {} Hz umgerechnet",
            clip.sample_rate, info.sample_rate
        );
    }
    let clip = clip.resampled(info.sample_rate);

    let tempo = TempoMap {
        bpm: args.bpm,
        ..TempoMap::default()
    };
    let (core, mut ctl) = engine(clip.sample_rate, tempo);
    ctl.send(Command::SetSong(Arc::new(Song::single(clip.samples))));
    let output = match prepared.start(core) {
        Ok(o) => o,
        Err(b) => {
            eprintln!("Fehler: {}", b.1);
            return ExitCode::FAILURE;
        }
    };

    if let Some((a, b)) = args.loop_bars {
        set_loop(&mut ctl, a, b);
        ctl.send(Command::ToggleCycle);
    }

    if let Some(secs) = args.seconds {
        ctl.send(Command::Play);
        thread::sleep(Duration::from_secs_f64(secs));
        ctl.send(Command::Stop);
        thread::sleep(Duration::from_millis(100));
        println!("{}", status_line(&ctl));
        println!("{} Blöcke gerendert", ctl.status.blocks());
        drop(output);
        return ExitCode::SUCCESS;
    }

    println!("Enter = Start/Stopp, q = beenden, h = Hilfe");
    let stdin = io::stdin();
    for line in stdin.lock().lines() {
        let Ok(line) = line else { break };
        let parts: Vec<&str> = line.split_whitespace().collect();
        let bar = |i: usize| parts.get(i).and_then(|s| s.parse::<u64>().ok());
        match parts.as_slice() {
            [] => {
                let cmd = if ctl.status.state() == TransportState::Stopped {
                    Command::Play
                } else {
                    Command::Stop
                };
                ctl.send(cmd);
            }
            ["s"] => {
                ctl.send(Command::Stop);
            }
            ["c"] => {
                ctl.send(Command::ToggleCycle);
            }
            ["l", _, _] => match (bar(1), bar(2)) {
                (Some(a), Some(b)) => set_loop(&mut ctl, a, b),
                _ => println!("l erwartet zwei Taktnummern"),
            },
            ["g", _] => match bar(1) {
                Some(t) => {
                    let f = ctl.frames(ctl.tempo.position(t.max(1), 1));
                    ctl.send(Command::Locate(f));
                }
                None => println!("g erwartet eine Taktnummer"),
            },
            ["q"] => break,
            ["h"] => {
                println!("{USAGE}");
                continue;
            }
            _ => {
                println!("Unbekannter Befehl, h für Hilfe");
                continue;
            }
        }
        thread::sleep(Duration::from_millis(30));
        println!("{}", status_line(&ctl));
        let _ = io::stdout().flush();
    }

    drop(output);
    ExitCode::SUCCESS
}
