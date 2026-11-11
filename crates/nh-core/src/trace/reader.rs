use crate::trace::{SessionHeader, TraceEvent};
use std::fs::File;
use std::io::{self, BufRead, BufReader};
use std::path::Path;

pub fn read(path: &Path) -> io::Result<(SessionHeader, Vec<TraceEvent>)> {
    let file = File::open(path)?;
    let reader = BufReader::new(file);
    let mut lines = reader.lines();

    let header_line = lines
        .next()
        .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidData, "Empty trace file"))??;
    let header: SessionHeader = serde_json::from_str(&header_line)
        .map_err(|e| io::Error::new(io::ErrorKind::InvalidData, format!("Invalid header: {e}")))?;

    let mut events = Vec::new();
    for (i, line) in lines.enumerate() {
        let line = line?;
        if line.trim().is_empty() {
            continue;
        }
        match serde_json::from_str::<TraceEvent>(&line) {
            Ok(event) => events.push(event),
            Err(_) if i == 0 => {} // Skip potential torn last line if it's the only line after header
            Err(e) => {
                return Err(io::Error::new(
                    io::ErrorKind::InvalidData,
                    format!("Invalid event at line {}: {e}", i + 2),
                ))
            }
        }
    }
    Ok((header, events))
}
