//! The session log: one JSONL file per session holding the transcript and the
//! per-turn, per-step trace, plus raw wire capture of every model call when
//! the config turns it on (`Config::wire`, off by default).
//! The format is the "Trace format" section of PLAN.md.

pub mod detail;
mod events;
pub mod reader;
pub mod report;
pub mod wire;
pub mod writer;

pub use events::{kind, RequestContext};
pub use reader::read;
pub use writer::TraceWriter;

use serde::{Deserialize, Serialize};
use serde_json::Value;

/// Format version written in every new header line. Version 2: the`n/// `request/context` key `wire` is present only with wire capture on.
pub const VERSION: u32 = 2;

/// The oldest version the reader accepts; a resumed log keeps its own header.
pub const MIN_VERSION: u32 = 1;

/// The first line of a session log (`"type":"session"`).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SessionHeader {
    pub version: u32,
    pub id: String,
    /// Unix milliseconds.
    pub created_at: u64,
    pub cwd: String,
    /// `nh <version>`.
    pub harness: String,
    /// Parent session id of a subagent session.
    #[serde(default)]
    pub parent: Option<String>,
}

impl SessionHeader {
    /// A header of the current [`VERSION`] created now for `cwd`, with harness `nh <crate version>`.
    pub fn new(id: &str, cwd: &str, parent: Option<String>) -> SessionHeader {
        SessionHeader {
            version: VERSION,
            id: id.to_string(),
            created_at: now_ms(),
            cwd: cwd.to_string(),
            harness: format!("nh {}", env!("CARGO_PKG_VERSION")),
            parent,
        }
    }
}

/// The header as written: `type` first, then the header fields.
#[derive(Serialize)]
pub(crate) struct HeaderLine<'a> {
    #[serde(rename = "type")]
    pub kind: &'static str,
    #[serde(flatten)]
    pub header: &'a SessionHeader,
}

/// One event line `{type, seq, time, data}`.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct TraceEvent {
    #[serde(rename = "type")]
    pub kind: String,
    pub seq: u64,
    /// Unix milliseconds.
    pub time: u64,
    pub data: Value,
}

/// Unix time in milliseconds; 0 if the clock is before 1970.
pub fn now_ms() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|elapsed| elapsed.as_millis() as u64)
        .unwrap_or(0)
}
