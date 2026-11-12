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
/// Heads the list of skills and nested AGENTS.md files loaded before.
pub const KEPT: &str = "Skills and nested AGENTS.md files loaded before the compaction \
(their text is no longer in the history: find_tools with the skill name, or read on the \
file, brings it back when needed):";

/// The summary as a user message; then, when tools were found, a synthetic
/// `find_tools` call `call_id` naming them and its result (Claude gets its
/// `tool_reference` blocks, Antigravity its declarations); then, `mid` turn,
/// [`CONTINUE`]. No old thinking survives. `kept` are the mark lines of the
/// skills and nested AGENTS.md files loaded before (PLAN.md D43), listed
/// after the summary so they stay loaded once per session.
pub fn history(
    summary: &str,
    loaded: &[String],
    kept: &[String],
    call_id: &str,
    mid: bool,
) -> Vec<Message> {
    let mut text = format!("{SUMMARY_PREFIX}\n\n{summary}");
    if !kept.is_empty() {
        text.push_str(&format!("\n\n{KEPT}\n{}", kept.join("\n")));
    }
    let mut out = vec![Message::user(text)];
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
    let list = |key: &str| -> Vec<String> {
        data[key]
            .as_array()
            .map(|list| {
                let names = list.iter().filter_map(Value::as_str);
                names.map(str::to_string).collect()
            })
            .unwrap_or_default()
    };
    let mid = data["mid"].as_bool().unwrap_or(false);
    let (loaded, kept) = (list("loaded"), list("kept"));
    history(text("summary"), &loaded, &kept, text("callId"), mid)
}
