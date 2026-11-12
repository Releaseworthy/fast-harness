//! Reads a session log back.

use crate::trace::{SessionHeader, TraceEvent, MIN_VERSION, VERSION};
use serde_json::Value;
use std::io;
use std::path::Path;

/// Reads the header and every event. A torn last line (a crash in the middle
/// of a write) is skipped; a damaged line before it is skipped too and
/// listed in [`Parsed::skipped`] ([`load`] keeps that list). A missing or
/// bad header is an `InvalidData` error naming line 1.
pub fn read(path: &Path) -> io::Result<(SessionHeader, Vec<TraceEvent>)> {
    let parsed = load(path)?;
    Ok((parsed.header, parsed.events))
}

/// [`read`] with the skipped lines.
pub fn load(path: &Path) -> io::Result<Parsed> {
    crate::ui_thread::forbid("trace::read");
    parse(&std::fs::read(path)?)
}

/// A log read back.
#[derive(Clone, Debug)]
pub struct Parsed {
    pub header: SessionHeader,
    pub events: Vec<TraceEvent>,
    /// Byte offset of the skipped torn last line.
    pub torn_at: Option<usize>,
    /// Damaged lines that are not the last, left out of `events` (G10, D60).
    pub skipped: Vec<Skipped>,
}

/// A damaged line that is not the last: 1-based line number, byte offset of
/// its start in the file.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Skipped {
    pub line: usize,
    pub offset: usize,
}

impl Parsed {
    /// One note naming the skipped lines, if any.
    pub fn note(&self) -> Option<String> {
        let first = self.skipped.first()?;
        let more = match self.skipped.len() {
            1 => String::new(),
            n => format!(" and {} more damaged lines", n - 1),
        };
        Some(format!(
            "session log: skipped damaged line {} (byte {}){more}; the log is left as it is",
            first.line, first.offset
        ))
    }
}

pub(crate) fn parse(bytes: &[u8]) -> io::Result<Parsed> {
    let lines = split_lines(bytes);
    let last = lines.iter().rposition(|(_, line)| !is_blank(line));
    let Some((_, first)) = lines.first() else {
        return Err(invalid("empty session log".to_string()));
    };
    let header = parse_header(first)?;
    let (mut events, mut torn_at, mut skipped) = (Vec::new(), None, Vec::new());
    for (index, (start, line)) in lines.iter().enumerate().skip(1) {
        if is_blank(line) {
            continue;
        }
        match serde_json::from_slice::<TraceEvent>(line) {
            Ok(event) => events.push(event),
            Err(_) if Some(index) == last => torn_at = Some(*start),
            Err(_) => skipped.push(Skipped {
                line: index + 1,
                offset: *start,
            }),
        }
    }
    Ok(Parsed {
        header,
        events,
        torn_at,
        skipped,
    })
}

fn parse_header(line: &[u8]) -> io::Result<SessionHeader> {
    let value: Value =
        serde_json::from_slice(line).map_err(|err| invalid(format!("line 1: {err}")))?;
    if value.get("type").and_then(Value::as_str) != Some("session") {
        return Err(invalid("line 1: not a session header".to_string()));
    }
    let header: SessionHeader =
        serde_json::from_value(value).map_err(|err| invalid(format!("line 1: {err}")))?;
    if !(MIN_VERSION..=VERSION).contains(&header.version) {
        let message = format!("line 1: unsupported trace version {}", header.version);
        return Err(invalid(message));
    }
    Ok(header)
}

/// `(start offset, line without its line ending)`; no entry for the empty
/// remainder after a final newline.
fn split_lines(bytes: &[u8]) -> Vec<(usize, &[u8])> {
    let mut lines = Vec::new();
    let mut start = 0;
    while start < bytes.len() {
        let end = bytes[start..]
            .iter()
            .position(|&b| b == b'\n')
            .map_or(bytes.len(), |pos| start + pos);
        let mut line = &bytes[start..end];
        if let Some(stripped) = line.strip_suffix(b"\r") {
            line = stripped;
        }
        lines.push((start, line));
        start = end + 1;
    }
    lines
}

fn is_blank(line: &[u8]) -> bool {
    line.iter().all(u8::is_ascii_whitespace)
}

fn invalid(message: String) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, message)
}
