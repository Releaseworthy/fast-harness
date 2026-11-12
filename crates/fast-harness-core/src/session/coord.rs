//! Coordination between processes without a daemon (PLAN.md D49). A running
//! session has `<id>.lock` ([`super::lock`]) and `<id>.inbox.jsonl` next to
//! its log: any process appends a [`Command`] line to the inbox, and the
//! running turn reads the new lines at its step boundaries with one size
//! check per step ([`FileInbox::poll`]).

use std::fs::{self, OpenOptions};
use std::io::{self, Read, Seek, SeekFrom, Write};
use std::path::{Path, PathBuf};

use serde_json::{json, Value};

/// One line of an inbox file.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Command {
    /// Joins the running turn at its next step boundary.
    Steer(String),
    /// A new turn after the running one ends `Done`.
    FollowUp(String),
    /// The step finishes its tools, then the turn ends `Paused`.
    Pause,
    /// The turn ends `Cancelled` at its next step boundary.
    Stop,
}

impl Command {
    pub fn to_json(&self) -> Value {
        match self {
            Command::Steer(text) => json!({"type": "steer", "text": text}),
            Command::FollowUp(text) => json!({"type": "follow_up", "text": text}),
            Command::Pause => json!({"type": "pause"}),
            Command::Stop => json!({"type": "stop"}),
        }
    }

    /// A line of the inbox; `None` for anything else (skipped).
    pub fn parse(line: &str) -> Option<Command> {
        let value: Value = serde_json::from_str(line.trim()).ok()?;
        let text = value["text"].as_str().map(str::to_string);
        match value["type"].as_str()? {
            "steer" => text.map(Command::Steer),
            "follow_up" => text.map(Command::FollowUp),
            "pause" => Some(Command::Pause),
            "stop" => Some(Command::Stop),
            _ => None,
        }
    }
}

/// `<id>.inbox.jsonl` of the session log `<id>.jsonl`.
pub fn inbox_path(log: &Path) -> PathBuf {
    log.with_extension("inbox.jsonl")
}

/// `<id>.lock` of the session log `<id>.jsonl`.
pub fn lock_path(log: &Path) -> PathBuf {
    log.with_extension("lock")
}

/// True for a session log (`<id>.jsonl`), false for an inbox file.
pub fn is_log(path: &Path) -> bool {
    let name = path.file_name().and_then(|n| n.to_str()).unwrap_or("");
    name.ends_with(".jsonl") && !name.ends_with(".inbox.jsonl")
}

/// Appends `command` as one line (one write call) to the inbox at `path`.
pub fn append(path: &Path, command: &Command) -> io::Result<()> {
    let mut file = OpenOptions::new().create(true).append(true).open(path)?;
    file.write_all(format!("{}\n", command.to_json()).as_bytes())
}

/// The reading end a running turn keeps: the inbox file and how far it read.
pub struct FileInbox {
    path: PathBuf,
    offset: u64,
}

impl FileInbox {
    /// Reads `path` from its start: lines written while no turn ran (or
    /// after the last turn's final read) belong to this turn.
    pub fn new(path: PathBuf) -> FileInbox {
        FileInbox { path, offset: 0 }
    }

    pub fn path(&self) -> &Path {
        &self.path
    }

    /// The commands of the whole lines added since the last call: one size
    /// check, and a read only when the file grew. A missing file is empty.
    pub fn poll(&mut self) -> Vec<Command> {
        let Ok(meta) = fs::metadata(&self.path) else {
            return Vec::new();
        };
        let len = meta.len();
        if len < self.offset {
            // Replaced by a shorter file: read it from its start.
            self.offset = 0;
        }
        if len == self.offset {
            return Vec::new();
        }
        let Ok(bytes) = self.read_from(len) else {
            return Vec::new();
        };
        let Some(end) = bytes.iter().rposition(|b| *b == b'\n') else {
            return Vec::new();
        };
        self.offset += end as u64 + 1;
        String::from_utf8_lossy(&bytes[..end])
            .lines()
            .filter_map(Command::parse)
            .collect()
    }

    fn read_from(&self, len: u64) -> io::Result<Vec<u8>> {
        let mut file = fs::File::open(&self.path)?;
        file.seek(SeekFrom::Start(self.offset))?;
        let mut bytes = Vec::with_capacity((len - self.offset) as usize);
        file.take(len - self.offset).read_to_end(&mut bytes)?;
        Ok(bytes)
    }

    /// Removes the file once the turn ended (after a last [`Self::poll`]).
    pub fn remove(&self) {
        let _ = fs::remove_file(&self.path);
    }
}
