//! The conversation as Antigravity `contents` (pi-ai `convertMessages` in
//! google-shared.ts and DSH antigravity-switch `fixContents`):
//!
//! - user text and tool results go as role `user`, assistant messages as role
//!   `model`; consecutive contents of the same role are merged; a history
//!   that starts with `model` gets a short user turn first.
//! - thinking (as `thought: true` parts) and thought signatures go back only
//!   when the assistant message was produced by the requested model; Claude
//!   thinking without a real signature is dropped.
//! - the first `functionCall` of each model turn carries a thought
//!   signature: its own real one, else the `skip_thought_signature_validator`
//!   sentinel; later (parallel) calls carry none.

use std::collections::HashMap;

use serde_json::{json, Map, Value};

use super::names::{sanitize, NameTable};
use crate::types::{Block, Message, Role};

/// Sentinel that skips thought-signature validation.
pub const SKIP_THOUGHT_SIGNATURE: &str = "skip_thought_signature_validator";
/// A signature shorter than this is not a real one.
pub const MIN_SIGNATURE_LENGTH: usize = 50;
/// User text put first when the history starts with a model turn.
pub const LEADING_USER_TEXT: &str = "(continuing the conversation)";

/// A thought signature that is a real one.
pub fn real_signature(value: Option<&str>) -> bool {
    value.is_some_and(|s| s.len() >= MIN_SIGNATURE_LENGTH && s != SKIP_THOUGHT_SIGNATURE)
}

/// Models that need `id` on function calls and responses (pi-ai
/// `requiresToolCallId`): Claude, GPT-OSS and Gemini 3 and later.
pub fn requires_tool_call_id(bare: &str) -> bool {
    let lower = bare.to_lowercase();
    if lower.starts_with("claude-") || lower.starts_with("gpt-oss-") {
        return true;
    }
    let rest = lower
        .strip_prefix("gemini-live-")
        .or_else(|| lower.strip_prefix("gemini-"));
    let digits: String = rest
        .unwrap_or("")
        .chars()
        .take_while(char::is_ascii_digit)
        .collect();
    digits.parse::<u32>().is_ok_and(|major| major >= 3)
}

/// What the conversion needs to know about the request.
pub struct Target<'a> {
    /// The full Fast Harness id (`antigravity/<id>`) of the request.
    pub full_model: &'a str,
    /// The bare id Google gets.
    pub bare_model: &'a str,
}

/// `messages` as Antigravity contents; tool names go through `names`.
pub fn contents(messages: &[Message], target: &Target<'_>, names: &mut NameTable) -> Vec<Value> {
    let call_names: HashMap<&str, &str> = messages
        .iter()
        .flat_map(|message| message.content.iter())
        .filter_map(|block| match block {
            Block::ToolCall { id, name, .. } => Some((id.as_str(), name.as_str())),
            _ => None,
        })
        .collect();
    let mut out: Vec<(&'static str, Vec<Value>)> = Vec::new();
    for message in messages {
        let (role, parts) = match message.role {
            Role::Assistant => ("model", model_parts(message, target, names)),
            _ => ("user", user_parts(message, target, names, &call_names)),
        };
        if parts.is_empty() {
            continue;
        }
        match out.last_mut() {
            Some((last, have)) if *last == role => have.extend(parts),
            _ => out.push((role, parts)),
        }
    }
    if out.first().is_some_and(|(role, _)| *role == "model") {
        out.insert(0, ("user", vec![json!({"text": LEADING_USER_TEXT})]));
    }
    out.into_iter()
        .map(|(role, mut parts)| {
            if role == "model" {
                sign_first_call(&mut parts);
            }
            json!({"role": role, "parts": parts})
        })
        .collect()
}

/// The parts of an assistant message.
fn model_parts(message: &Message, target: &Target<'_>, names: &mut NameTable) -> Vec<Value> {
    let same_model = message.model.as_deref() == Some(target.full_model);
    let claude = target.bare_model.to_lowercase().starts_with("claude-");
    let mut parts = Vec::new();
    for block in &message.content {
        match block {
            Block::Text { text } if !text.is_empty() => parts.push(json!({"text": text})),
            Block::Thinking { text, signature } if same_model => {
                let signature = signature.as_deref().filter(|s| real_signature(Some(s)));
                if (claude && signature.is_none())
                    || (text.trim().is_empty() && signature.is_none())
                {
                    continue;
                }
                let mut part = json!({"thought": true, "text": text});
                if let Some(signature) = signature {
                    part["thoughtSignature"] = json!(signature);
                }
                parts.push(part);
            }
            Block::ToolCall {
                id,
                name,
                arguments,
                signature,
            } => {
                let signature = signature.as_deref().filter(|_| same_model);
                parts.push(call_part(id, name, arguments, signature, target, names));
            }
            _ => {}
        }
    }
    parts
}

fn call_part(
    id: &str,
    name: &str,
    arguments: &str,
    signature: Option<&str>,
    target: &Target<'_>,
    names: &mut NameTable,
) -> Value {
    let args = match serde_json::from_str::<Value>(arguments) {
        Ok(Value::Object(map)) => Value::Object(map),
        _ => Value::Object(Map::new()),
    };
    let mut call = json!({"name": names.sent(name), "args": args});
    if requires_tool_call_id(target.bare_model) {
        call["id"] = json!(id);
    }
    let mut part = json!({"functionCall": call});
    if let Some(signature) = signature.filter(|s| real_signature(Some(s))) {
        part["thoughtSignature"] = json!(signature);
    }
    part
}

/// The parts of a user or tool message: text and function responses.
fn user_parts(
    message: &Message,
    target: &Target<'_>,
    names: &mut NameTable,
    call_names: &HashMap<&str, &str>,
) -> Vec<Value> {
    let mut parts = Vec::new();
    for block in &message.content {
        match block {
            Block::Text { text } if !text.is_empty() => parts.push(json!({"text": text})),
            Block::ToolResult {
                call_id,
                content,
                is_error,
            } => {
                let name = match call_names.get(call_id.as_str()) {
                    Some(original) => names.sent(original),
                    None => sanitize("unknown_tool"),
                };
                let key = if *is_error { "error" } else { "output" };
                let mut response = json!({"name": name, "response": {key: content}});
                if requires_tool_call_id(target.bare_model) {
                    response["id"] = json!(call_id);
                }
                parts.push(json!({"functionResponse": response}));
            }
            _ => {}
        }
    }
    parts
}

/// The first function call of a model turn keeps its real signature or gets
/// the sentinel; later calls carry none.
fn sign_first_call(parts: &mut [Value]) {
    let mut first = true;
    for part in parts.iter_mut() {
        let Some(map) = part.as_object_mut() else {
            continue;
        };
        if !map.contains_key("functionCall") {
            continue;
        }
        if first {
            first = false;
            let real = real_signature(map.get("thoughtSignature").and_then(Value::as_str));
            if !real {
                map.insert("thoughtSignature".into(), json!(SKIP_THOUGHT_SIGNATURE));
            }
        } else {
            map.remove("thoughtSignature");
        }
    }
}
