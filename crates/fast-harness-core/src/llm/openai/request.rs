//! The Chat Completions request body: `messages` with the system first,
//! tool calls as `tool_calls`, results as `role: tool` messages, and the
//! tools as `functions`. With `find_tools` in the list only it and the tools
//! a find result named are sent (PLAN.md D30, as on Antigravity).

use serde_json::{json, Map, Value};

use crate::llm::{find, ChatRequest, ToolSpec};
use crate::types::{Block, Message, Role};

/// The tools of this request: every tool, or with deferred loading
/// `find_tools` plus the tools found so far, in list order.
pub fn visible<'a>(req: &ChatRequest<'a>) -> Vec<&'a ToolSpec> {
    if !find::deferring(req.tools) {
        return req.tools.iter().collect();
    }
    let found = find::found(req.messages);
    let shown = |tool: &&ToolSpec| tool.name == find::FIND_TOOLS || found.contains(&tool.name);
    req.tools.iter().filter(shown).collect()
}

fn tool_json(tool: &ToolSpec) -> Value {
    json!({
        "type": "function",
        "function": {
            "name": tool.name,
            "description": tool.description,
            "parameters": tool.schema,
        }
    })
}

fn assistant(message: &Message) -> Value {
    let text = message.text();
    let calls: Vec<Value> = message
        .tool_calls()
        .map(|(id, name, arguments)| {
            json!({"id": id, "type": "function", "function": {"name": name, "arguments": arguments}})
        })
        .collect();
    let mut out = Map::new();
    out.insert("role".into(), "assistant".into());
    out.insert(
        "content".into(),
        match text.is_empty() {
            true => Value::Null,
            false => text.into(),
        },
    );
    if !calls.is_empty() {
        out.insert("tool_calls".into(), calls.into());
    }
    Value::Object(out)
}

/// A user or tool message: one `role: tool` message per result, then the
/// text (a steer after results) as a user message.
fn user(message: &Message, out: &mut Vec<Value>) {
    let mut text = String::new();
    for block in &message.content {
        match block {
            Block::ToolResult {
                call_id, content, ..
            } => out.push(json!({"role": "tool", "tool_call_id": call_id, "content": content})),
            Block::Text { text: piece } => {
                if !text.is_empty() {
                    text.push_str("\n\n");
                }
                text.push_str(piece);
            }
            _ => {}
        }
    }
    if !text.is_empty() {
        out.push(json!({"role": "user", "content": text}));
    }
}

/// The `messages` array. Reasoning of earlier answers is not sent back.
pub fn messages(system: &str, history: &[Message]) -> Vec<Value> {
    let mut out = Vec::with_capacity(history.len() + 1);
    if !system.is_empty() {
        out.push(json!({"role": "system", "content": system}));
    }
    for message in history {
        match message.role {
            Role::Assistant => out.push(assistant(message)),
            Role::User | Role::Tool => user(message, &mut out),
        }
    }
    out
}

/// The whole body. `max_output` caps `max_tokens`; `efforts` are the
/// effort words the model takes (`reasoning_effort` is sent only for one).
pub fn build(
    req: &ChatRequest<'_>,
    model: &str,
    max_output: Option<u64>,
    efforts: &[String],
) -> Value {
    let mut max_tokens = u64::from(req.max_tokens);
    if let Some(cap) = max_output.filter(|cap| *cap > 0) {
        max_tokens = max_tokens.min(cap);
    }
    let mut body = json!({
        "model": model,
        "stream": true,
        "stream_options": {"include_usage": true},
        "max_tokens": max_tokens,
        "messages": messages(req.system, req.messages),
    });
    let tools: Vec<Value> = visible(req).into_iter().map(tool_json).collect();
    if !tools.is_empty() {
        body["tools"] = tools.into();
    }
    if let Some(effort) = req
        .effort
        .filter(|e| efforts.iter().any(|known| known == e))
    {
        body["reasoning_effort"] = effort.into();
    }
    body
}
