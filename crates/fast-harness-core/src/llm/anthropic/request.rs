//! The `POST /v1/messages` body, built the way pi-ai's `buildParams`,
//! `convertMessages` and `convertTools` build it (bundled in DeepSeek Harness,
//! read 2026-11-11).
//!
//! Prompt caching: <https://platform.claude.com/docs/en/build-with-claude/prompt-caching>
//! (search excerpt 2026-11-11): `cache_control: {"type": "ephemeral"}` caches
//! for 5 minutes, `{"type": "ephemeral", "ttl": "1h"}` for one hour. As in
//! pi-ai, breakpoints go on every system block, the last tool and the last
//! block of a trailing user message.

use serde_json::{json, Map, Value};

use super::models::ModelSpec;
use super::thinking;
use super::wire;
use crate::llm::{ChatRequest, ToolSpec};
use crate::types::{Block, Message, Role};

/// The identity block that must open the system prompt in OAuth mode.
pub(crate) const CLAUDE_CODE_IDENTITY: &str =
    "You are Claude Code, Anthropic's official CLI for Claude.";

/// Claude Code's tool names. In OAuth mode a tool whose name matches one of
/// these case-insensitively is sent with this casing and mapped back on the
/// response (pi-ai `toClaudeCodeName` / `fromClaudeCodeName`).
const CLAUDE_CODE_TOOLS: &[&str] = &[
    "Read",
    "Write",
    "Edit",
    "Bash",
    "Grep",
    "Glob",
    "AskUserQuestion",
    "EnterPlanMode",
    "ExitPlanMode",
    "KillShell",
    "NotebookEdit",
    "Skill",
    "Task",
    "TaskOutput",
    "TodoWrite",
    "WebFetch",
    "WebSearch",
];

pub(crate) fn to_claude_code_name(name: &str) -> &str {
    CLAUDE_CODE_TOOLS
        .iter()
        .find(|known| known.eq_ignore_ascii_case(name))
        .copied()
        .unwrap_or(name)
}

/// The caller's own name for a tool name the model used.
pub(crate) fn from_claude_code_name(name: &str, tools: &[ToolSpec]) -> String {
    tools
        .iter()
        .find(|tool| tool.name.eq_ignore_ascii_case(name))
        .map_or_else(|| name.to_string(), |tool| tool.name.clone())
}

/// Tool call ids limited to Anthropic's pattern `[a-zA-Z0-9_-]{1,64}`.
pub(crate) fn normalize_tool_id(id: &str) -> String {
    id.chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || c == '_' || c == '-' {
                c
            } else {
                '_'
            }
        })
        .take(64)
        .collect()
}

pub(crate) fn cache_control(long: bool) -> Value {
    if long {
        json!({"type": "ephemeral", "ttl": "1h"})
    } else {
        json!({"type": "ephemeral"})
    }
}

/// The JSON body and the `anthropic-beta` list of one streaming call.
pub(crate) fn build(
    req: &ChatRequest<'_>,
    spec: Option<&ModelSpec>,
    oauth: bool,
) -> (Value, Vec<&'static str>) {
    let cache = cache_control(req.cache_long);
    let plan = thinking::plan_model(req.model, spec, req.effort, req.max_tokens);
    let mut messages = convert_messages(req.messages, oauth, &cache);
    if let Some(effort) = &plan.trailing_effort {
        messages
            .push(json!({"role": "system", "content": [], "output_config": {"effort": effort}}));
    }
    let mut body = Map::new();
    body.insert("model".into(), json!(req.model));
    body.insert("messages".into(), Value::Array(messages));
    body.insert("max_tokens".into(), json!(plan.max_tokens));
    body.insert("stream".into(), json!(true));
    let system = system_blocks(req.system, oauth, &cache);
    if !system.is_empty() {
        body.insert("system".into(), Value::Array(system));
    }
    if !req.tools.is_empty() {
        body.insert("tools".into(), convert_tools(req.tools, oauth, &cache));
    }
    for (key, value) in [
        ("thinking", plan.thinking),
        ("output_config", plan.output_config),
    ] {
        if let Some(value) = value {
            body.insert(key.into(), value);
        }
    }
    let mut betas = if oauth {
        vec![wire::BETA_CLAUDE_CODE, wire::BETA_OAUTH]
    } else {
        Vec::new()
    };
    betas.extend(plan.betas);
    if let Some(spec) = spec.filter(|spec| !spec.fallbacks.is_empty()) {
        betas.push(wire::BETA_SERVER_SIDE_FALLBACK);
        let list: Vec<Value> = spec.fallbacks.iter().map(|m| json!({"model": m})).collect();
        body.insert("fallbacks".into(), Value::Array(list));
    }
    (Value::Object(body), betas)
}

fn system_blocks(system: &str, oauth: bool, cache: &Value) -> Vec<Value> {
    let mut blocks = Vec::new();
    let block = |text: &str| json!({"type": "text", "text": text, "cache_control": cache});
    if oauth {
        blocks.push(block(CLAUDE_CODE_IDENTITY));
    }
    if !system.is_empty() {
        blocks.push(block(system));
    }
    blocks
}

fn convert_tools(tools: &[ToolSpec], oauth: bool, cache: &Value) -> Value {
    let last = tools.len().saturating_sub(1);
    let list = tools.iter().enumerate().map(|(index, tool)| {
        let name = if oauth {
            to_claude_code_name(&tool.name)
        } else {
            &tool.name
        };
        let properties = tool.schema.get("properties").cloned();
        let required = tool.schema.get("required").cloned();
        let mut out = json!({
            "name": name,
            "description": tool.description,
            "eager_input_streaming": true,
            "input_schema": {
                "type": "object",
                "properties": properties.unwrap_or_else(|| json!({})),
                "required": required.unwrap_or_else(|| json!([])),
            },
        });
        if index == last {
            out["cache_control"] = cache.clone();
        }
        out
    });
    Value::Array(list.collect())
}

fn convert_messages(messages: &[Message], oauth: bool, cache: &Value) -> Vec<Value> {
    let mut out: Vec<Value> = Vec::new();
    let mut previous_was_tool = false;
    for message in messages {
        let (role, blocks) = match message.role {
            Role::User => ("user", user_blocks(&message.content)),
            Role::Tool => ("user", user_blocks(&message.content)),
            Role::Assistant => ("assistant", assistant_blocks(&message.content, oauth)),
        };
        let is_tool = message.role == Role::Tool;
        if blocks.is_empty() {
            continue;
        }
        // Consecutive tool messages become one user message of results.
        if is_tool && previous_was_tool {
            if let Some(Value::Array(content)) = out.last_mut().map(|m| &mut m["content"]) {
                content.extend(blocks);
                continue;
            }
        }
        previous_was_tool = is_tool;
        out.push(json!({"role": role, "content": blocks}));
    }
    mark_last_message(&mut out, cache);
    out
}

/// Puts the cache breakpoint on the last block of a trailing user message.
fn mark_last_message(messages: &mut [Value], cache: &Value) {
    let Some(last) = messages.last_mut() else {
        return;
    };
    if last["role"] != "user" {
        return;
    }
    if let Some(block) = last["content"].as_array_mut().and_then(|c| c.last_mut()) {
        if block["type"] == "text" || block["type"] == "tool_result" {
            block["cache_control"] = cache.clone();
        }
    }
}

fn user_blocks(content: &[Block]) -> Vec<Value> {
    let mut out = Vec::new();
    for block in content {
        match block {
            Block::Text { text } if !text.trim().is_empty() => {
                out.push(json!({"type": "text", "text": text}));
            }
            Block::ToolResult {
                call_id,
                content,
                is_error,
            } => out.push(json!({
                "type": "tool_result",
                "tool_use_id": normalize_tool_id(call_id),
                "content": content,
                "is_error": is_error,
            })),
            _ => {}
        }
    }
    out
}

fn assistant_blocks(content: &[Block], oauth: bool) -> Vec<Value> {
    let mut out = Vec::new();
    for block in content {
        match block {
            Block::Text { text } if !text.trim().is_empty() => {
                out.push(json!({"type": "text", "text": text}));
            }
            Block::Thinking { text, signature } => {
                let signature = signature.as_deref().filter(|s| !s.trim().is_empty());
                match signature {
                    Some(signature) => out.push(
                        json!({"type": "thinking", "thinking": text, "signature": signature}),
                    ),
                    // Unsigned reasoning (an aborted stream) goes back as plain text.
                    None if !text.trim().is_empty() => {
                        out.push(json!({"type": "text", "text": text}));
                    }
                    None => {}
                }
            }
            Block::RedactedThinking { data } => {
                out.push(json!({"type": "redacted_thinking", "data": data}));
            }
            Block::ToolCall {
                id,
                name,
                arguments,
            } => out.push(tool_use(id, name, arguments, oauth)),
            _ => {}
        }
    }
    out
}

fn tool_use(id: &str, name: &str, arguments: &str, oauth: bool) -> Value {
    let input = serde_json::from_str::<Value>(arguments)
        .ok()
        .filter(Value::is_object)
        .unwrap_or_else(|| json!({}));
    let name = if oauth {
        to_claude_code_name(name)
    } else {
        name
    };
    json!({"type": "tool_use", "id": normalize_tool_id(id), "name": name, "input": input})
}
