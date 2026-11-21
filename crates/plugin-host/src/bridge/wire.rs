//! The sockets between app and helper: a listener the helper connects to twice (control, then
//! wake), the control connection with its line protocol, and the wake-up byte.
//!
//! Control protocol, one request line from the app, reply lines from the helper ending in
//! `END`. The helper opens with its parameters:
//! `PARAM\t<id>\t<min>\t<max>\t<default>\t<value>\t<stepped>\t<name>\t<module>` lines, then `END`
//! (or `ERR\t<message>`). Texts are escaped (`\t`, `\n`, `\\`); states travel as hex.

use std::{
    io::{BufRead, BufReader, ErrorKind, Read, Write},
    path::Path,
    process::Child,
    time::{Duration, Instant},
};

use crate::info::ParamInfo;

#[cfg(unix)]
pub(crate) type Stream = std::os::unix::net::UnixStream;
#[cfg(not(unix))]
pub(crate) type Stream = std::net::TcpStream;

/// Longest reply line (a state of up to `MAX_STATE` bytes as hex, with room).
const MAX_LINE: usize = 140 << 20;
/// Most reply lines (parameter lists).
const MAX_LINES: usize = 100_000;

pub(crate) struct Listener {
    #[cfg(unix)]
    inner: std::os::unix::net::UnixListener,
}

impl Listener {
    #[cfg(unix)]
    pub fn bind(path: &Path) -> Result<Self, String> {
        let inner = std::os::unix::net::UnixListener::bind(path)
            .map_err(|e| format!("{}: {e}", path.display()))?;
        {
            use std::os::unix::fs::PermissionsExt;
            let _ = std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o600));
        }
        inner.set_nonblocking(true).map_err(|e| e.to_string())?;
        Ok(Self { inner })
    }

    #[cfg(not(unix))]
    pub fn bind(_: &Path) -> Result<Self, String> {
        Err("the plugin bridge is not available on this system yet".into())
    }

    /// The next connection of the helper, or an error once it has exited or `deadline` passed.
    #[cfg(unix)]
    pub fn accept(&self, child: &mut Child, deadline: Instant) -> Result<Stream, String> {
        loop {
            match self.inner.accept() {
                Ok((s, _)) => {
                    s.set_nonblocking(false).map_err(|e| e.to_string())?;
                    return Ok(s);
                }
                Err(e) if e.kind() == ErrorKind::WouldBlock => {}
                Err(e) => return Err(format!("helper connection: {e}")),
            }
            gone(child)?;
            if Instant::now() > deadline {
                return Err("the helper process did not start in time".into());
            }
            std::thread::sleep(Duration::from_millis(5));
        }
    }

    #[cfg(not(unix))]
    pub fn accept(&self, _: &mut Child, _: Instant) -> Result<Stream, String> {
        Err("the plugin bridge is not available on this system yet".into())
    }
}

/// An error if the helper has exited.
fn gone(child: &mut Child) -> Result<(), String> {
    match child.try_wait() {
        Ok(None) => Ok(()),
        Ok(Some(status)) => Err(format!("the helper process ended ({status})")),
        Err(e) => Err(format!("helper process: {e}")),
    }
}

/// The control connection (app side).
pub(crate) struct Conn {
    reader: BufReader<Stream>,
    writer: Stream,
}

impl Conn {
    pub fn new(stream: Stream) -> Result<Self, String> {
        let writer = stream.try_clone().map_err(|e| e.to_string())?;
        Ok(Self {
            reader: BufReader::new(stream),
            writer,
        })
    }

    pub fn send(&mut self, line: &str) -> Result<(), String> {
        self.writer
            .write_all(line.as_bytes())
            .and_then(|()| self.writer.write_all(b"\n"))
            .map_err(|e| format!("helper: {e}"))
    }

    /// The reply lines up to `END` (without it); an `ERR` line becomes the error.
    pub fn read_reply(
        &mut self,
        child: &mut Child,
        deadline: Instant,
    ) -> Result<Vec<String>, String> {
        let mut lines = Vec::new();
        loop {
            let left = deadline.saturating_duration_since(Instant::now());
            if left.is_zero() {
                return Err("the helper did not answer in time".into());
            }
            // macOS refuses a timeout once the helper has closed its end (EINVAL); the read
            // then returns what is left and the end, so it cannot block.
            let _ = self
                .reader
                .get_ref()
                .set_read_timeout(Some(left.min(Duration::from_millis(200))));
            let mut line = String::new();
            match (&mut self.reader)
                .take(MAX_LINE as u64)
                .read_line(&mut line)
            {
                Ok(0) => return Err("the helper closed the connection".into()),
                Ok(_) if !line.ends_with('\n') => return Err("reply line too long".into()),
                Ok(_) => {}
                Err(e) if matches!(e.kind(), ErrorKind::WouldBlock | ErrorKind::TimedOut) => {
                    // A partial line stays in `line` only if nothing was read; retry.
                    gone(child)?;
                    if !line.is_empty() {
                        return Err("the helper stalled mid-line".into());
                    }
                    continue;
                }
                Err(e) => return Err(format!("helper: {e}")),
            }
            let line = line.trim_end_matches('\n').to_owned();
            if line == "END" {
                return Ok(lines);
            }
            if let Some(e) = line.strip_prefix("ERR\t") {
                return Err(unescape(e));
            }
            if lines.len() >= MAX_LINES {
                return Err("reply too long".into());
            }
            lines.push(line);
        }
    }
}

/// The wake-up connection (app side): one byte per block, never blocking.
pub(crate) struct Wake {
    stream: Stream,
}

impl Wake {
    pub fn new(stream: Stream) -> Result<Self, String> {
        stream.set_nonblocking(true).map_err(|e| e.to_string())?;
        Ok(Self { stream })
    }

    /// Wake the helper (audio thread): a full buffer means it has wake-ups pending anyway.
    pub fn ring(&mut self) {
        let _ = self.stream.write(&[1]);
    }
}

pub(crate) fn hex(data: &[u8]) -> String {
    const DIGITS: &[u8; 16] = b"0123456789abcdef";
    let mut s = String::with_capacity(data.len() * 2);
    for b in data {
        s.push(char::from(DIGITS[usize::from(b >> 4)]));
        s.push(char::from(DIGITS[usize::from(b & 15)]));
    }
    s
}

pub(crate) fn unhex(s: &str, max: usize) -> Option<Vec<u8>> {
    if s.len() % 2 != 0 || s.len() / 2 > max {
        return None;
    }
    let digit = |c: u8| match c {
        b'0'..=b'9' => Some(c - b'0'),
        b'a'..=b'f' => Some(c - b'a' + 10),
        _ => None,
    };
    s.as_bytes()
        .chunks_exact(2)
        .map(|p| Some(digit(p[0])? << 4 | digit(p[1])?))
        .collect()
}

pub(crate) fn escape(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for c in s.chars() {
        match c {
            '\\' => out.push_str("\\\\"),
            '\t' => out.push_str("\\t"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            c => out.push(c),
        }
    }
    out
}

pub(crate) fn unescape(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut it = s.chars();
    while let Some(c) = it.next() {
        if c != '\\' {
            out.push(c);
            continue;
        }
        match it.next() {
            Some('t') => out.push('\t'),
            Some('n') => out.push('\n'),
            Some('r') => out.push('\r'),
            Some(o) => out.push(o),
            None => {}
        }
    }
    out
}

pub(crate) fn param_line(p: &ParamInfo) -> String {
    format!(
        "PARAM\t{}\t{}\t{}\t{}\t{}\t{}\t{}\t{}",
        p.id,
        p.min,
        p.max,
        p.default,
        p.value,
        u8::from(p.stepped),
        escape(&p.name),
        escape(&p.module)
    )
}

/// The parameter list the helper opens with.
pub(crate) fn parse_hello(lines: &[String]) -> Result<Vec<ParamInfo>, String> {
    lines
        .iter()
        .filter_map(|l| l.strip_prefix("PARAM\t"))
        .map(|l| {
            let f: Vec<&str> = l.split('\t').collect();
            let bad = || format!("bad parameter line from the helper: {l}");
            let num = |i: usize| -> Result<f64, String> {
                f.get(i)
                    .and_then(|v| v.parse::<f64>().ok())
                    .filter(|v| v.is_finite())
                    .ok_or_else(bad)
            };
            Ok(ParamInfo {
                id: f.first().and_then(|v| v.parse().ok()).ok_or_else(bad)?,
                min: num(1)?,
                max: num(2)?,
                default: num(3)?,
                value: num(4)?,
                stepped: f.get(5) == Some(&"1"),
                name: unescape(f.get(6).copied().unwrap_or("")),
                module: unescape(f.get(7).copied().unwrap_or("")),
            })
        })
        .collect()
}

/// Whether the helper's plugin has a window (`GUI\t1` in the opening lines).
pub(crate) fn parse_gui(lines: &[String]) -> bool {
    lines.iter().any(|l| l == "GUI\t1")
}

pub(crate) fn values_lines(values: &[(u32, f64)]) -> Vec<String> {
    values
        .iter()
        .map(|(id, v)| format!("V\t{id}\t{v}"))
        .collect()
}

/// `V\t<id>\t<value>` lines; others and values that are not finite are skipped.
pub(crate) fn parse_values(lines: &[String]) -> Vec<(u32, f64)> {
    lines
        .iter()
        .filter_map(|l| {
            let mut f = l.strip_prefix("V\t")?.split('\t');
            let id = f.next()?.parse().ok()?;
            let v: f64 = f.next()?.parse().ok()?;
            v.is_finite().then_some((id, v))
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hex_and_text_round_trip() {
        let data: Vec<u8> = (0..=255).collect();
        assert_eq!(unhex(&hex(&data), 1 << 20).unwrap(), data);
        assert!(unhex("abc", 10).is_none());
        assert!(unhex("zz", 10).is_none());
        assert!(unhex("0011", 1).is_none(), "over the limit");
        let s = "a\tb\nc\\d";
        assert_eq!(unescape(&escape(s)), s);
    }

    #[test]
    fn parameter_lines_round_trip_and_bad_ones_are_errors() {
        let p = ParamInfo {
            id: 3,
            name: "Gain\tx".into(),
            module: "Main".into(),
            min: -1.5,
            max: 2.0,
            default: 0.0,
            value: 0.25,
            stepped: true,
        };
        let back = parse_hello(&[param_line(&p)]).unwrap();
        assert_eq!(back, [p]);
        assert!(parse_hello(&["PARAM\tx".into()]).is_err());
        assert!(parse_hello(&["PARAM\t1\tNaN\t1\t0\t0\t0\ta\tb".into()]).is_err());
        assert_eq!(
            parse_values(&["V\t1\t0.5".into(), "V\t2\tinf".into(), "junk".into()]),
            [(1, 0.5)]
        );
    }
}
