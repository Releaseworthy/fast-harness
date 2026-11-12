//! The JSON lines of a panel (docs/panels.md): what the process sends
//! ([`Incoming`]) and what the host writes to its stdin.

use serde_json::{json, Value};

use super::Host;
use crate::events::AgentEvent;

/// One line from the panel process.
#[derive(Clone, Debug, PartialEq)]
pub enum Incoming {
    /// `{"type":"view","root":<node>}`: the whole tree.
    View(Value),
    /// `{"type":"patch","id","node"}`: replaces the node with that id.
    Patch { id: String, node: Value },
    /// `{"type":"subscribe","events":["turn","tool","usage"]}`.
    Subscribe(Vec<String>),
    /// `{"type":"notify","text"}`: shown as the panel's status line.
    Notify(String),
    /// A line that is not one of the above, and why.
    Bad(String),
}

/// The message of one stdout line; `None` for a blank line.
pub fn parse_line(line: &str) -> Option<Incoming> {
    let line = line.trim();
    if line.is_empty() {
        return None;
    }
    let Ok(value) = serde_json::from_str::<Value>(line) else {
        let start: String = line.chars().take(80).collect();
        return Some(Incoming::Bad(format!("not JSON: {start}")));
    };
    let field = |key: &str| value.get(key).cloned();
    let text = |key: &str| value.get(key).and_then(Value::as_str).map(String::from);
    Some(match value.get("type").and_then(Value::as_str) {
        Some("view") => match field("root") {
            Some(root) => Incoming::View(root),
            None => Incoming::Bad("view without root".into()),
        },
        Some("patch") => match (text("id"), field("node")) {
            (Some(id), Some(node)) => Incoming::Patch { id, node },
            _ => Incoming::Bad("patch needs id and node".into()),
        },
        Some("subscribe") => Incoming::Subscribe(
            (value.get("events").and_then(Value::as_array))
                .into_iter()
                .flatten()
                .filter_map(|e| e.as_str().map(String::from))
                .collect(),
        ),
        Some("notify") => Incoming::Notify(text("text").unwrap_or_default()),
        Some(other) => Incoming::Bad(format!("unknown message type {other}")),
        None => Incoming::Bad("message without type".into()),
    })
}

/// The first line the process reads.
pub fn init_line(panel: &str, host: Host<'_>) -> String {
    json!({
        "type": "init",
        "panel": panel,
        "theme": { "kind": if host.dark { "dark" } else { "light" } },
        "cwd": host.cwd.to_string_lossy(),
        "session": host.session,
    })
    .to_string()
}

/// A `click`, `submit` or `change` event of the widget `id`.
pub fn event_line(kind: &str, id: &str, value: Value) -> String {
    json!({ "type": kind, "id": id, "value": value }).to_string()
}

/// The subscription kind (`turn`, `tool` or `usage`) and the JSON body of
/// an agent event; `None` for events panels never get.
pub fn session_event(event: &AgentEvent) -> Option<(&'static str, Value)> {
    Some(match event {
        AgentEvent::TurnStart { turn } => ("turn", json!({ "kind": "turn_start", "turn": turn })),
        AgentEvent::TurnEnd {
            turn,
            usage,
            duration_ms,
            outcome,
        } => (
            "turn",
            json!({
                "kind": "turn_end", "turn": turn, "outcome": format!("{outcome:?}").to_lowercase(),
                "durationMs": duration_ms, "input": usage.input, "output": usage.output,
            }),
        ),
        AgentEvent::ToolCall { id, name, .. } => (
            "tool",
            json!({ "kind": "tool_call", "id": id, "name": name }),
        ),
        AgentEvent::ToolResult {
            id,
            is_error,
            duration_ms,
            ..
        } => (
            "tool",
            json!({ "kind": "tool_result", "id": id, "isError": is_error, "durationMs": duration_ms }),
        ),
        AgentEvent::StepEnd {
            usage,
            context,
            window,
            ..
        } => (
            "usage",
            json!({
                "kind": "usage", "input": usage.input, "output": usage.output,
                "context": context, "window": window,
            }),
        ),
        _ => return None,
    })
}
