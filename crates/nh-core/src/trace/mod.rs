//! The session log: one JSONL file per session holding the transcript and the
//! per-turn, per-step trace, plus raw wire capture of every model call (chunk 1C).

pub mod reader;
pub mod report;
pub mod wire;
pub mod writer;

use serde::{Deserialize, Serialize};
use serde_json::Value;

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct SessionHeader {
    pub version: u32,
    pub id: String,
    #[serde(rename = "createdAt")]
    pub created_at: u64,
    pub cwd: String,
    pub harness: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub parent: Option<String>,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct TraceEvent {
    pub kind: String,
    pub seq: u64,
    pub time: u64,
    pub data: Value,
}
