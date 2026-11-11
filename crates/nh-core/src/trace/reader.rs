//! Reads a session log back.

use crate::trace::{SessionHeader, TraceEvent, VERSION};
use serde_json::Value;
use std::io;
use std::path::Path;

/// Reads the header and every event. A torn last line (a crash in the middle
/// of a write) is skipped; any other line that is not a valid event is an
/// `InvalidData` error naming its 1-based line number.
pub fn read(path: &Path) -> io::Result<(SessionHeader, Vec<TraceEvent>)> {
    let bytes = std::fs::read(path)?;
    let parsed = parse(&bytes)?;
    Ok((parsed.header, parsed.events))
}

pub(crate) struct Parsed {
    pub header: SessionHeader,
    pub events: Vec<TraceEvent>,
    /// Byte offset of the skipped torn last line.
    pub torn_at: Option<usize>,
}

pub(crate) fn parse(bytes: &[u8]) -> io::Result<Parsed> {
    let lines = split_lines(bytes);
    let last = lines.iter().rposition(|(_, line)| !is_blank(line));
    let Some((_, first)) = lines.first() else {
        return Err(invalid("empty session log".to_string()));
    };
    let header = parse_header(first)?;
    let mut events = Vec::new();
    let mut torn_at = None;
    for (index, (start, line)) in lines.iter().enumerate().skip(1) {
        if is_blank(line) {
            continue;
        }
        match serde_json::from_slice::<TraceEvent>(line) {
            Ok(event) => events.push(event),
            Err(_) if Some(index) == last => torn_at = Some(*start),
            Err(err) => return Err(invalid(format!("line {}: {err}", index + 1))),
        }
    }
    Ok(Parsed {
        header,
        events,
        torn_at,
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
    if header.version != VERSION {
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
