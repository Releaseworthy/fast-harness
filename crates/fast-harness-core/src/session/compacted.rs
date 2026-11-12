//! The history that replaces a session's messages after a compaction
//! (PLAN.md D38), built the same way live and on replay from the
//! `compaction` event's data.

use serde_json::{json, Value};

use crate::llm::find::{loaded_line, FIND_TOOLS};
use crate::types::{Block, Message, Role};

/// Starts the summary user message.
pub const SUMMARY_PREFIX: &str = "[summary of the session so far]";
/// The user message that follows a compaction in the middle of a turn.
pub const CONTINUE: &str = "Continue the task.";
/// Second line of the synthetic find result.
pub const RESTORED: &str = "(tools found before the compaction, loaded again)";

/// The summary as a user message; then, when tools were found, a synthetic
/// `find_tools` call `call_id` naming them and its result (Claude gets its
/// `tool_reference` blocks, Antigravity its declarations); then, `mid` turn,
/// [`CONTINUE`]. No old thinking survives.
pub fn history(summary: &str, loaded: &[String], call_id: &str, mid: bool) -> Vec<Message> {
    let mut out = vec![Message::user(format!("{SUMMARY_PREFIX}\n\n{summary}"))];
    if !loaded.is_empty() {
        let names: Vec<&str> = loaded.iter().map(String::as_str).collect();
        out.push(Message {
            role: Role::Assistant,
            content: vec![Block::ToolCall {
                id: call_id.to_string(),
                name: FIND_TOOLS.into(),
                arguments: json!({ "names": names }).to_string(),
                signature: None,
            }],
            model: None,
        });
        out.push(Message {
            role: Role::Tool,
            content: vec![Block::ToolResult {
                call_id: call_id.to_string(),
                content: format!("{}\n{RESTORED}", loaded_line(&names)),
                is_error: false,
            }],
            model: None,
        });
    }
    if mid {
        out.push(Message::user(CONTINUE));
    }
    out
}

/// [`history`] from the data of a `compaction` event.
pub fn from_event(data: &Value) -> Vec<Message> {
    let text = |key: &str| data[key].as_str().unwrap_or_default();
    let loaded: Vec<String> = data["loaded"]
        .as_array()
        .map(|list| {
            let names = list.iter().filter_map(Value::as_str);
            names.map(str::to_string).collect()
        })
        .unwrap_or_default();
    let mid = data["mid"].as_bool().unwrap_or(false);
    history(text("summary"), &loaded, text("callId"), mid)
}
