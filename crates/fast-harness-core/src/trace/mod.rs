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

pub use events::{header_data, kind, prefix_binding_mismatches, RequestContext};
pub use reader::read;
pub use writer::TraceWriter;

use serde::{Deserialize, Serialize};
use serde_json::Value;

/// Format version written in every new header line. Version 2: the
/// `request/context` key `wire` is present only with wire capture on.
/// Version 3: assistant messages carry `model`; `tool_call` blocks may carry
/// `signature`. Version 4: `assistant/message` has `context` and `window`,
/// usage may have `cacheWrite1hTokens`, and `request/transformations` exists.
/// Version 5: `request/header` may carry `context` (the context stamp of
/// PLAN.md D36) and is the frozen system and tools replayed on resume; a
/// user message may start with a `[context update]` text block.
/// Version 6: `tool/result` has `bytes` (the full output size) and, for a
/// result cut to its budget, `spilled` (the spill file, PLAN.md D37).
/// Version 7: `compaction` events (PLAN.md D38) replace the history before
/// them on resume; `request/header` is written before `user/message`.
/// Version 9: the header may carry `depth` (a subagent session's nesting
/// level, PLAN.md D40; absent means 0).
pub const VERSION: u32 = 9;

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
    /// `fast-harness <version>`.
    pub harness: String,
    /// Parent session id of a subagent session.
    #[serde(default)]
    pub parent: Option<String>,
    /// Nesting level: 0 for a session a person started, 1 for its
    /// subagents, and so on (version 9; absent means 0).
    #[serde(default, skip_serializing_if = "is_zero")]
    pub depth: u32,
}

fn is_zero(depth: &u32) -> bool {
    *depth == 0
}

impl SessionHeader {
    /// A header of the current [`VERSION`] created now for `cwd`, with harness `fast-harness <crate version>`.
    pub fn new(id: &str, cwd: &str, parent: Option<String>) -> SessionHeader {
        SessionHeader {
            version: VERSION,
            id: id.to_string(),
            created_at: now_ms(),
            cwd: cwd.to_string(),
            harness: format!("fast-harness {}", env!("CARGO_PKG_VERSION")),
            parent,
            depth: 0,
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
