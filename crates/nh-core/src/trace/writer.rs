use crate::trace::{SessionHeader, TraceEvent};
use serde_json::{json, Value};
use std::fs::{self, File, OpenOptions};
use std::io::{self, BufRead, BufReader, Write};
use std::path::{Path, PathBuf};

pub struct TraceWriter {
    file: File,
    #[allow(dead_code)]
    path: PathBuf,
    seq: u64,
}

impl TraceWriter {
    pub fn create(path: &Path, header: &SessionHeader) -> io::Result<Self> {
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent)?;
        }
        let mut file = File::create(path)?;
        let header_value = serde_json::to_value(header)?;
        let mut header_line = match header_value {
            Value::Object(map) => map,
            _ => {
                return Err(io::Error::new(
                    io::ErrorKind::InvalidData,
                    "Header is not an object",
                ))
            }
        };
        header_line.insert("type".to_string(), json!("session"));
        writeln!(file, "{}", Value::Object(header_line))?;
        Ok(Self {
            file,
            path: path.to_owned(),
            seq: 0,
        })
    }

    pub fn open_append(path: &Path) -> io::Result<Self> {
        let file = OpenOptions::new().append(true).open(path)?;
        let reader = BufReader::new(File::open(path)?);
        let mut max_seq = 0;
        for line in reader.lines() {
            let line = line?;
            if let Ok(event) = serde_json::from_str::<TraceEvent>(&line) {
                if event.seq > max_seq {
                    max_seq = event.seq;
                }
            }
        }
        Ok(Self {
            file,
            path: path.to_owned(),
            seq: max_seq + 1,
        })
    }

    pub fn emit(&mut self, kind: &str, data: Value) -> io::Result<u64> {
        let time = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_millis() as u64;
        let event = json!({
            "type": kind,
            "seq": self.seq,
            "time": time,
            "data": data
        });
        writeln!(self.file, "{}", event)?;
        self.file.flush()?;
        let seq = self.seq;
        self.seq += 1;
        Ok(seq)
    }

    // Event specific methods would go here based on the event list in PLAN.md
}
