//! The session log: one JSONL file per session holding the transcript and the
//! per-turn, per-step trace, plus raw wire capture of every model call when
//! the config turns it on (`Config::wire`, off by default).
//! The format is the "Trace format" section of docs/formats.md.

pub mod detail;
mod events;
pub mod reader;
pub mod record;
pub mod report;
pub mod wire;
pub mod writer;

pub use events::{header_data, kind, prefix_binding_mismatches, RequestContext};
pub use reader::read;
pub use record::Record;
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
/// Version 10: `hook` and `hook/error` events (PLAN.md D44); a user text of
/// kind `hook` (a hook's context or Stop continuation) joins the history
/// like a steer.
/// Version 11: `tool/approval` events (PLAN.md D64): how an approval request
/// ended, so a replay shows the same word a live transcript showed.
/// Version 12: `undo` events (PLAN.md D69).
/// Version 13: `image` blocks in user and tool messages (PLAN.md D70), an
/// image held by reference to `<session>.media/<sha>.<ext>`.
pub const VERSION: u32 = 13;

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

impl TraceEvent {
    /// The typed record of this line (PLAN.md D61); [`Record::Raw`] for a
    /// type or shape this build does not know.
    pub fn record(&self) -> Record {
        Record::parse(&self.kind, &self.data)
    }
}

/// Unix time in milliseconds; 0 if the clock is before 1970.
/// The clock itself is [`crate::types::now_ms`] (the bottom layer), so `tools`
/// reads it without importing this higher module; this re-export keeps
/// `trace::now_ms` for every caller.
pub use crate::types::now_ms;
