//! The find tool's wire contract (PLAN.md D30), shared by the tool, the
//! agent and both providers.
//!
//! When a request's tool list holds [`FIND_TOOLS`], every other tool is
//! deferred: Claude gets it with `defer_loading: true` and Antigravity only
//! once it was found. A `find_tools` result that is not an error starts with
//! the line `Loaded: a, b`; the names on that line are the found tools, so the
//! found set is rebuilt from the conversation alone (resume needs nothing more).

use std::collections::HashSet;

use serde_json::Value;

use crate::llm::ToolSpec;
use crate::types::{Block, Message, Role};

/// The one tool loaded from the start.
pub const FIND_TOOLS: &str = "find_tools";

/// Starts the first line of a successful find result.
const LOADED: &str = "Loaded: ";

/// True when the tool list asks for deferred loading.
pub fn deferring(tools: &[ToolSpec]) -> bool {
    tools.iter().any(|tool| tool.name == FIND_TOOLS)
}

/// The first line of a find result that loaded `names`.
pub fn loaded_line(names: &[&str]) -> String {
    format!("{LOADED}{}", names.join(", "))
}

/// The tool names a find result loaded, in order; empty for other text.
pub fn loaded_names(content: &str) -> Vec<&str> {
    let first = content.lines().next().unwrap_or("");
    match first.strip_prefix(LOADED) {
        Some(rest) => rest
            .split(',')
            .map(str::trim)
            .filter(|name| !name.is_empty())
            .collect(),
        None => Vec::new(),
    }
}

/// Ids of the `find_tools` calls in `messages`.
pub fn find_call_ids(messages: &[Message]) -> HashSet<&str> {
    let assistants = messages.iter().filter(|m| m.role == Role::Assistant);
    assistants
        .flat_map(Message::tool_calls)
        .filter(|(_, name, _)| *name == FIND_TOOLS)
        .map(|(id, _, _)| id)
        .collect()
}

/// Every tool found so far, in first-found order, without duplicates.
pub fn found(messages: &[Message]) -> Vec<String> {
    let calls = find_call_ids(messages);
    let mut out: Vec<String> = Vec::new();
    for message in messages.iter().filter(|m| m.role != Role::Assistant) {
        for block in &message.content {
            let Block::ToolResult {
                call_id,
                content,
                is_error: false,
            } = block
            else {
                continue;
            };
            if !calls.contains(call_id.as_str()) {
                continue;
            }
            for name in loaded_names(content) {
                if !out.iter().any(|known| known == name) {
                    out.push(name.to_string());
                }
            }
        }
    }
    out
}

/// A short label of a find call's arguments: the names, else the quoted query.
pub fn call_label(arguments: &str) -> Option<String> {
    let args: Value = serde_json::from_str(arguments).ok()?;
    let names: Vec<&str> = args
        .get("names")
        .and_then(Value::as_array)
        .map(|list| list.iter().filter_map(Value::as_str).collect())
        .unwrap_or_default();
    if !names.is_empty() {
        return Some(names.join(", "));
    }
    let query = args.get("query").and_then(Value::as_str)?.trim();
    (!query.is_empty()).then(|| format!("\"{query}\""))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn call(id: &str, name: &str) -> Message {
        Message {
            role: Role::Assistant,
            content: vec![Block::ToolCall {
                id: id.into(),
                name: name.into(),
                arguments: "{}".into(),
                signature: None,
            }],
            model: None,
        }
    }

    fn result(id: &str, content: &str, is_error: bool) -> Message {
        Message {
            role: Role::Tool,
            content: vec![Block::ToolResult {
                call_id: id.into(),
                content: content.into(),
                is_error,
            }],
            model: None,
        }
    }

    #[test]
    fn found_set_comes_from_find_results_in_order() {
        let messages = vec![
            Message::user("go"),
            call("c1", FIND_TOOLS),
            result("c1", "Loaded: read, edit\nmore", false),
            call("c2", "read"),
            result("c2", "Loaded: shell", false),
            call("c3", FIND_TOOLS),
            result("c3", "Loaded: grep", true),
            call("c4", FIND_TOOLS),
            result("c4", "Loaded: shell, read", false),
        ];
        assert_eq!(found(&messages), ["read", "edit", "shell"]);
        assert!(loaded_names("No tool matched").is_empty());
        assert_eq!(loaded_line(&["a", "b"]), "Loaded: a, b");
    }

    #[test]
    fn call_labels() {
        assert_eq!(
            call_label(r#"{"names":["read","edit"]}"#).unwrap(),
            "read, edit"
        );
        assert_eq!(call_label(r#"{"query":" run "}"#).unwrap(), "\"run\"");
        assert_eq!(call_label("{oops"), None);
    }
}
