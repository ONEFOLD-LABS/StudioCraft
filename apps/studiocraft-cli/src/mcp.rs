//! `studiocraft-cli mcp`: StudioCraft as an MCP server on stdin and stdout, for AI agents
//! (see `docs/mcp.md`). Only protocol messages go to stdout; errors go to stderr.

use std::{io, path::PathBuf, process::ExitCode};

use studiocraft_automation::{Roots, connect, control::APP, server};

pub const USAGE: &str = "\
Aufruf: studiocraft-cli mcp --automation-read-root <ordner> --automation-write-root <ordner>
                            [--connect [<port> | 127.0.0.1:<port>]]

  MCP-Server (Model Context Protocol) auf stdin/stdout für KI-Agenten.
  --automation-read-root   Ordner, aus dem der Agent lesen darf (mehrfach möglich)
  --automation-write-root  Ordner, in den der Agent schreiben darf (mehrfach möglich)
  Beide sind Pflicht; Pfade außerhalb werden abgelehnt.
  --connect  die laufende App steuern (gestartet mit `studiocraft --control <port>`);
             ohne Port der zuletzt gestartete Steuerkanal, sonst Port 7850";

/// What the command line asks for.
#[derive(Debug, Default, PartialEq)]
struct Args {
    read: Vec<PathBuf>,
    write: Vec<PathBuf>,
    /// `--connect`, with the address when one was given.
    connect: Option<Option<String>>,
}

/// The two root flags (`--flag dir` or `--flag=dir`, each repeatable) and `--connect`.
fn parse(args: impl IntoIterator<Item = String>) -> Result<Args, String> {
    const READ: &str = "--automation-read-root";
    const WRITE: &str = "--automation-write-root";
    let mut out = Args::default();
    let mut it = args.into_iter().peekable();
    while let Some(a) = it.next() {
        let (flag, inline) = match a.split_once('=') {
            Some((f, v)) => (f.to_owned(), Some(v.to_owned())),
            None => (a.clone(), None),
        };
        let list = match flag.as_str() {
            READ => &mut out.read,
            WRITE => &mut out.write,
            "--connect" => {
                let addr = match inline {
                    Some(v) => Some(v),
                    None => it.next_if(|n| !n.starts_with('-')),
                };
                out.connect = Some(addr.map(|a| loopback(&a)).transpose()?);
                continue;
            }
            "-h" | "--help" => return Err(String::new()),
            _ => return Err(format!("Unbekanntes Argument: {a}")),
        };
        let dir = match inline {
            Some(v) => v,
            None => it.next().ok_or(format!("{flag}: Ordner fehlt"))?,
        };
        if dir.is_empty() {
            return Err(format!("{flag}: Ordner fehlt"));
        }
        list.push(PathBuf::from(dir));
    }
    Ok(out)
}

/// `addr` as `host:port` on loopback: a bare port means `127.0.0.1`. The token never goes to
/// another computer.
fn loopback(addr: &str) -> Result<String, String> {
    let (host, port) = match addr.rsplit_once(':') {
        Some((h, p)) => (h.trim_start_matches('[').trim_end_matches(']'), p),
        None => ("127.0.0.1", addr),
    };
    let port: u16 = port
        .parse()
        .ok()
        .filter(|p| *p != 0)
        .ok_or(format!("--connect: kein gültiger Port: {addr}"))?;
    match host {
        "127.0.0.1" | "localhost" => Ok(format!("127.0.0.1:{port}")),
        "::1" => Ok(format!("[::1]:{port}")),
        _ => Err(format!("--connect: nur lokal (127.0.0.1), nicht {host}")),
    }
}

/// Runs the server until stdin closes.
pub fn run(args: impl IntoIterator<Item = String>) -> ExitCode {
    let parsed = parse(args).and_then(|a| Roots::new(&a.read, &a.write).map(|r| (a, r)));
    let (args, roots) = match parsed {
        Ok(r) => r,
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
    let stdin = io::stdin();
    let served = match args.connect {
        None => server(roots).serve(stdin.lock(), io::stdout().lock()),
        Some(addr) => {
            let addr = addr.unwrap_or_else(|| {
                let port = APP.read_current_port().unwrap_or(APP.default_port);
                format!("127.0.0.1:{port}")
            });
            eprintln!("StudioCraft-Steuerkanal: {addr}");
            connect(roots, &addr, None).serve(stdin.lock(), io::stdout().lock())
        }
    };
    match served {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => {
            eprintln!("Fehler: {e}");
            ExitCode::FAILURE
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn args(a: &[&str]) -> Vec<String> {
        a.iter().map(|s| (*s).to_owned()).collect()
    }

    #[test]
    fn both_flag_forms_repeat_and_misspellings_are_errors() {
        let a = parse(args(&[
            "--automation-read-root",
            "a",
            "--automation-read-root=b",
            "--automation-write-root=c",
        ]))
        .unwrap();
        assert_eq!(a.read, [PathBuf::from("a"), PathBuf::from("b")]);
        assert_eq!(a.write, [PathBuf::from("c")]);
        assert_eq!(a.connect, None);
        assert!(parse(args(&["--automation-read-roots", "a"])).is_err());
        assert!(parse(args(&["--automation-write-root"])).is_err());
        assert!(parse(args(&["--automation-write-root="])).is_err());
    }

    #[test]
    fn connect_takes_an_optional_loopback_address() {
        let c = |a: &[&str]| parse(args(a)).map(|a| a.connect);
        assert_eq!(c(&["--connect"]), Ok(Some(None)));
        assert_eq!(
            c(&["--connect", "--automation-read-root", "a"]),
            Ok(Some(None))
        );
        assert_eq!(
            c(&["--connect", "7851"]),
            Ok(Some(Some("127.0.0.1:7851".into())))
        );
        assert_eq!(
            c(&["--connect=localhost:7851"]),
            Ok(Some(Some("127.0.0.1:7851".into())))
        );
        assert!(c(&["--connect", "192.168.1.5:7851"]).is_err());
        assert!(c(&["--connect", "0"]).is_err());
    }
}
