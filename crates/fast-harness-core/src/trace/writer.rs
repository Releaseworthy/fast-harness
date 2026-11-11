//! Appends events to a session log, one flushed line per event.

use crate::trace::{now_ms, reader, HeaderLine, SessionHeader, TraceEvent};
use serde_json::Value;
use std::fs::{self, File, OpenOptions};
use std::io::{self, Read, Seek, SeekFrom, Write};
use std::path::Path;

/// Writer of one session log. Every event is written with a single
/// `write_all` and flushed before the call returns, so a crash leaves at most
/// one torn last line, which [`reader::read`] skips and
/// [`TraceWriter::open_append`] cuts off.
pub struct TraceWriter {
    file: File,
    next_seq: u64,
}

impl TraceWriter {
    /// Creates the parent folders and a new log holding only the header line.
    /// Fails with `AlreadyExists` instead of overwriting an existing log.
    pub fn create(path: &Path, header: &SessionHeader) -> io::Result<TraceWriter> {
        if let Some(parent) = path.parent().filter(|p| !p.as_os_str().is_empty()) {
            fs::create_dir_all(parent)?;
        }
        let mut file = OpenOptions::new().write(true).create_new(true).open(path)?;
        let line = HeaderLine {
            kind: "session",
            header,
        };
        write_line(&mut file, &serde_json::to_vec(&line)?)?;
        Ok(TraceWriter { file, next_seq: 0 })
    }

    /// Opens an existing log to add events after its last one: `seq`
    /// continues from the highest seq present. A torn last line is removed.
    pub fn open_append(path: &Path) -> io::Result<TraceWriter> {
        let mut file = OpenOptions::new().read(true).write(true).open(path)?;
        let mut bytes = Vec::new();
        file.read_to_end(&mut bytes)?;
        let parsed = reader::parse(&bytes)?;
        let keep = parsed.torn_at.unwrap_or(bytes.len());
        if keep < bytes.len() {
            file.set_len(keep as u64)?;
        }
        file.seek(SeekFrom::Start(keep as u64))?;
        if keep > 0 && bytes[keep - 1] != b'\n' {
            file.write_all(b"\n")?;
        }
        let next_seq = parsed.events.iter().map(|e| e.seq + 1).max().unwrap_or(0);
        Ok(TraceWriter { file, next_seq })
    }

    /// Writes `{"type","seq","time","data"}` and a newline, flushes, and
    /// returns the seq of the event.
    pub fn emit(&mut self, kind: &str, data: Value) -> io::Result<u64> {
        let seq = self.next_seq;
        let event = TraceEvent {
            kind: kind.to_string(),
            seq,
            time: now_ms(),
            data,
        };
        write_line(&mut self.file, &serde_json::to_vec(&event)?)?;
        self.next_seq += 1;
        Ok(seq)
    }

    /// The seq the next event will get.
    pub fn next_seq(&self) -> u64 {
        self.next_seq
    }
}

fn write_line(file: &mut File, json: &[u8]) -> io::Result<()> {
    let mut line = Vec::with_capacity(json.len() + 1);
    line.extend_from_slice(json);
    line.push(b'\n');
    file.write_all(&line)?;
    file.flush()
}
