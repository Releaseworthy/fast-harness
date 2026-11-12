//! Assembles a Chat Completions stream: `data: {chunk}` events, then
//! `data: [DONE]`. A chunk carries `choices[0].delta` (`content`,
//! `reasoning_content`, `tool_calls` with streamed `function.arguments`),
//! `finish_reason`, and with `stream_options.include_usage` a last chunk
//! with `usage` (`prompt_tokens_details.cached_tokens` is the cache read).

use serde_json::Value;

use super::error;
use crate::llm::{Delta, LlmError};
use crate::types::{Block, Message, Role, StopReason, Usage};

#[derive(Default)]
struct Call {
    id: String,
    name: String,
    arguments: String,
}

#[derive(Default)]
pub struct Assembler {
    text: String,
    thinking: String,
    /// The stream's tool `index` and our call, in first-seen order.
    calls: Vec<(u64, Call)>,
    finish: Option<String>,
    pub usage: Usage,
    /// `[DONE]` arrived.
    pub done: bool,
}

impl Assembler {
    /// Handles one SSE data payload; `Ok(true)` when a delta was emitted.
    pub fn handle(
        &mut self,
        data: &str,
        on_delta: &mut dyn FnMut(Delta<'_>),
    ) -> Result<bool, LlmError> {
        let data = data.trim();
        if data == "[DONE]" {
            self.done = true;
            return Ok(false);
        }
        if data.is_empty() {
            return Ok(false);
        }
        let chunk: Value = serde_json::from_str(data)
            .map_err(|err| error::protocol(format!("bad stream chunk: {err}")))?;
        if chunk.get("error").is_some() {
            return Err(error::from_event(&chunk));
        }
        if let Some(usage) = chunk.get("usage").filter(|u| u.is_object()) {
            self.usage = usage_of(usage);
        }
        let Some(choice) = chunk["choices"].get(0) else {
            return Ok(false);
        };
        if let Some(reason) = choice["finish_reason"].as_str() {
            self.finish = Some(reason.to_string());
        }
        Ok(self.delta(&choice["delta"], on_delta))
    }

    fn delta(&mut self, delta: &Value, on_delta: &mut dyn FnMut(Delta<'_>)) -> bool {
        let mut emitted = false;
        let reasoning = delta["reasoning_content"]
            .as_str()
            .or(delta["reasoning"].as_str());
        if let Some(text) = reasoning.filter(|t| !t.is_empty()) {
            self.thinking.push_str(text);
            on_delta(Delta::Thinking(text));
            emitted = true;
        }
        if let Some(text) = delta["content"].as_str().filter(|t| !t.is_empty()) {
            self.text.push_str(text);
            on_delta(Delta::Text(text));
            emitted = true;
        }
        for call in delta["tool_calls"].as_array().into_iter().flatten() {
            emitted |= self.tool_delta(call, on_delta);
        }
        emitted
    }

    fn tool_delta(&mut self, call: &Value, on_delta: &mut dyn FnMut(Delta<'_>)) -> bool {
        let key = call["index"].as_u64().unwrap_or(self.calls.len() as u64);
        let position = match self.calls.iter().position(|(k, _)| *k == key) {
            Some(position) => position,
            None => {
                self.calls.push((key, Call::default()));
                self.calls.len() - 1
            }
        };
        let entry = &mut self.calls[position].1;
        let started = !entry.name.is_empty();
        if let Some(id) = call["id"].as_str().filter(|id| !id.is_empty()) {
            entry.id = id.to_string();
        }
        if let Some(name) = call["function"]["name"].as_str() {
            entry.name.push_str(name);
        }
        let mut emitted = false;
        if !started && !entry.name.is_empty() {
            if entry.id.is_empty() {
                entry.id = format!("call_{position}");
            }
            on_delta(Delta::ToolStart {
                index: position,
                id: &entry.id,
                name: &entry.name,
            });
            emitted = true;
        }
        if let Some(chunk) = call["function"]["arguments"]
            .as_str()
            .filter(|c| !c.is_empty())
        {
            entry.arguments.push_str(chunk);
            on_delta(Delta::ToolArgs {
                index: position,
                chunk,
            });
            emitted = true;
        }
        emitted
    }

    /// True once the answer is complete: `[DONE]`, or a finish reason when
    /// the server closes without `[DONE]`.
    pub fn complete(&self) -> bool {
        self.done || self.finish.is_some()
    }

    pub fn stop(&self) -> StopReason {
        match self.finish.as_deref() {
            _ if !self.calls.is_empty() => StopReason::ToolUse,
            Some("stop") => StopReason::End,
            Some("length") => StopReason::MaxTokens,
            Some("content_filter") => StopReason::Refusal,
            Some("tool_calls" | "function_call") => StopReason::ToolUse,
            _ => StopReason::Other,
        }
    }

    pub fn message(self) -> Message {
        let mut content = Vec::new();
        if !self.thinking.is_empty() {
            content.push(Block::Thinking {
                text: self.thinking,
                signature: None,
            });
        }
        if !self.text.is_empty() {
            content.push(Block::Text { text: self.text });
        }
        for (_, call) in self.calls {
            let arguments = match call.arguments.trim() {
                "" => "{}".to_string(),
                _ => call.arguments,
            };
            content.push(Block::ToolCall {
                id: call.id,
                name: call.name,
                arguments,
                signature: None,
            });
        }
        Message {
            role: Role::Assistant,
            content,
            model: None,
        }
    }
}

/// `prompt_tokens` holds the cached part; it is moved to `cache_read`.
fn usage_of(usage: &Value) -> Usage {
    let int = |value: &Value| value.as_u64().unwrap_or(0);
    let prompt = int(&usage["prompt_tokens"]);
    let cached = int(&usage["prompt_tokens_details"]["cached_tokens"])
        .max(int(&usage["prompt_cache_hit_tokens"]))
        .min(prompt);
    Usage {
        input: prompt - cached,
        output: int(&usage["completion_tokens"]),
        cache_read: cached,
        cache_write: 0,
        cache_write_1h: 0,
    }
}
