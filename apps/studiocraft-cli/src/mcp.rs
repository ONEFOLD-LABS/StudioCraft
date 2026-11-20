//! `studiocraft-cli mcp`: StudioCraft as an MCP server on stdin and stdout, for AI agents
//! (see `docs/mcp.md`). Only protocol messages go to stdout; errors go to stderr.

use std::{io, path::PathBuf, process::ExitCode};

use studiocraft_automation::{Roots, server};

pub const USAGE: &str = "\
Aufruf: studiocraft-cli mcp --automation-read-root <ordner> --automation-write-root <ordner>

  MCP-Server (Model Context Protocol) auf stdin/stdout für KI-Agenten.
  --automation-read-root   Ordner, aus dem der Agent lesen darf (mehrfach möglich)
  --automation-write-root  Ordner, in den der Agent schreiben darf (mehrfach möglich)
  Beide sind Pflicht; Pfade außerhalb werden abgelehnt.";

/// The folders given with the two flags (`--flag dir` or `--flag=dir`, each repeatable).
fn parse(args: impl IntoIterator<Item = String>) -> Result<(Vec<PathBuf>, Vec<PathBuf>), String> {
    const READ: &str = "--automation-read-root";
    const WRITE: &str = "--automation-write-root";
    let (mut read, mut write) = (Vec::new(), Vec::new());
    let mut it = args.into_iter();
    while let Some(a) = it.next() {
        let (flag, inline) = match a.split_once('=') {
            Some((f, v)) => (f.to_owned(), Some(v.to_owned())),
            None => (a.clone(), None),
        };
        let list = match flag.as_str() {
            READ => &mut read,
            WRITE => &mut write,
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
    Ok((read, write))
}

/// Runs the server until stdin closes.
pub fn run(args: impl IntoIterator<Item = String>) -> ExitCode {
    let roots = parse(args).and_then(|(read, write)| Roots::new(&read, &write));
    let roots = match roots {
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
    match server(roots).serve(stdin.lock(), io::stdout().lock()) {
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
        let (r, w) = parse(args(&[
            "--automation-read-root",
            "a",
            "--automation-read-root=b",
            "--automation-write-root=c",
        ]))
        .unwrap();
        assert_eq!(r, [PathBuf::from("a"), PathBuf::from("b")]);
        assert_eq!(w, [PathBuf::from("c")]);
        assert!(parse(args(&["--automation-read-roots", "a"])).is_err());
        assert!(parse(args(&["--automation-write-root"])).is_err());
        assert!(parse(args(&["--automation-write-root="])).is_err());
    }
}
