//! The Chat Completions request body: `messages` with the system first,
//! tool calls as `tool_calls`, results as `role: tool` messages, and the
//! tools as `functions`. With `find_tools` in the list only it and the tools
//! a find result named are sent (PLAN.md D30).

use serde_json::{json, Map, Value};

use crate::llm::images::{self, Prepared};
use crate::llm::{find, ChatRequest, ToolSpec};
use crate::types::{Block, ImageRef, Message, Role};

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

/// A data URL `image_url` part (PLAN.md D70).
fn image_part(image: &ImageRef, prepared: &Prepared<'_>) -> Value {
    let url = format!("data:{};base64,{}", image.media_type, prepared.data(image));
    json!({"type": "image_url", "image_url": {"url": url}})
}

/// The user content: a string without images, else text and image parts.
struct UserContent {
    text: String,
    parts: Vec<Value>,
}

impl UserContent {
    fn push_text(&mut self, piece: &str) {
        if !self.text.is_empty() {
            self.text.push_str("\n\n");
        }
        self.text.push_str(piece);
    }

    fn push_image(&mut self, part: Value) {
        if !self.text.is_empty() {
            let text = std::mem::take(&mut self.text);
            self.parts.push(json!({"type": "text", "text": text}));
        }
        self.parts.push(part);
    }

    fn message(mut self) -> Option<Value> {
        if self.parts.is_empty() {
            return (!self.text.is_empty()).then(|| json!({"role": "user", "content": self.text}));
        }
        if !self.text.is_empty() {
            self.parts.push(json!({"type": "text", "text": self.text}));
        }
        Some(json!({"role": "user", "content": self.parts}))
    }
}

/// A user or tool message: one `role: tool` message per result, then the
/// images of the results as a user message (a tool message takes text
/// only), then the text (a steer after results) as a user message.
fn user(message: &Message, prepared: &Prepared<'_>, out: &mut Vec<Value>) {
    let empty = || UserContent {
        text: String::new(),
        parts: Vec::new(),
    };
    let (mut content, mut results) = (empty(), empty());
    let mut call: Option<&str> = None;
    for block in &message.content {
        match block {
            Block::ToolResult {
                call_id, content, ..
            } => {
                out.push(json!({"role": "tool", "tool_call_id": call_id, "content": content}));
                call = Some(call_id);
            }
            Block::Text { text: piece } => content.push_text(piece),
            Block::Image(image) => match call.filter(|_| message.role == Role::Tool) {
                Some(id) => {
                    results.push_text(&format!("[image of tool result {id}]"));
                    results.push_image(image_part(image, prepared));
                }
                None => content.push_image(image_part(image, prepared)),
            },
            _ => {}
        }
    }
    out.extend(results.message());
    out.extend(content.message());
}

/// The `messages` array. Reasoning of earlier answers is not sent back;
/// images go only when the model takes them (`vision`), else as text.
pub fn messages(system: &str, history: &[Message], vision: bool) -> Vec<Value> {
    let prepared = images::prepare(history, vision);
    let mut out = Vec::with_capacity(history.len() + 1);
    if !system.is_empty() {
        out.push(json!({"role": "system", "content": system}));
    }
    for message in prepared.messages.iter() {
        match message.role {
            Role::Assistant => out.push(assistant(message)),
            Role::User | Role::Tool => user(message, &prepared, &mut out),
        }
    }
    out
}

/// The whole body. `max_output` caps `max_tokens`; `efforts` are the
/// effort words the model takes (`reasoning_effort` is sent only for one);
/// `vision`: the model takes images (`"images": true` in its file).
pub fn build(
    req: &ChatRequest<'_>,
    model: &str,
    max_output: Option<u64>,
    efforts: &[String],
    vision: bool,
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
        "messages": messages(req.system, req.messages, vision),
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
