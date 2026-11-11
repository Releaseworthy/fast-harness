//! The `streamGenerateContent?alt=sse` answer assembled into a completion:
//! text, thoughts, function calls with their thought signatures, usage, the
//! finish reason and errors.
//!
//! Each SSE `data:` line is one `GenerateContentResponse` wrapped as
//! `{"response": {...}, "traceId": ...}` (an array gives its first object).
//! Only `candidates[0]` is read. Part shapes and the finish reasons:
//! <https://ai.google.dev/api/generate-content> (read 2026-11-12); handling
//! as pi-ai's `google-generative-ai.ts` and the DSH proxy's `unwrapResponse`.

mod finish;

use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{SystemTime, UNIX_EPOCH};

use serde_json::Value;

use super::provider::error::{first_object, from_event};
use super::ToolNames;
use crate::llm::{Delta, ErrorKind, LlmError};
use crate::types::{Block, Message, Role, StopReason, Usage};

pub use finish::{stop_reason, usage_from};

/// The parts of one answer as they arrive.
pub struct Assembler<'n> {
    names: &'n ToolNames,
    blocks: Vec<Block>,
    calls: usize,
    finish: Option<String>,
    usage: Usage,
}

/// The `GenerateContentResponse` inside one event.
fn unwrap_response(value: &Value) -> &Value {
    let body = first_object(value);
    match body.get("response") {
        Some(inner) if inner.is_object() => inner,
        _ => body,
    }
}

fn protocol(message: &str) -> LlmError {
    LlmError::new(ErrorKind::Protocol, message)
}

/// A unique tool call id for a call Google sent without one.
fn new_call_id() -> String {
    static COUNTER: AtomicU64 = AtomicU64::new(0);
    let count = COUNTER.fetch_add(1, Ordering::Relaxed);
    let ms = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis() as u64)
        .unwrap_or(0);
    format!("call_{ms:x}_{count}")
}

fn non_empty(value: &Value) -> Option<&str> {
    value.as_str().filter(|text| !text.is_empty())
}

impl<'n> Assembler<'n> {
    /// `names` maps the tool names sent to Google back to the agent's.
    pub fn new(names: &'n ToolNames) -> Assembler<'n> {
        Assembler {
            names,
            blocks: Vec::new(),
            calls: 0,
            finish: None,
            usage: Usage::default(),
        }
    }

    /// One SSE `data` payload. `Ok(true)` when a delta went out.
    pub fn handle(
        &mut self,
        data: &str,
        on_delta: &mut dyn FnMut(Delta<'_>),
    ) -> Result<bool, LlmError> {
        let data = data.trim();
        if data.is_empty() || data == "[DONE]" {
            return Ok(false);
        }
        let value: Value = serde_json::from_str(data)
            .map_err(|_| protocol("Antigravity sent a stream event that is not JSON"))?;
        let response = unwrap_response(&value);
        if let Some(error) = response.get("error").filter(|e| e.is_object()) {
            return Err(from_event(error));
        }
        if let Some(meta) = response.get("usageMetadata").filter(|m| m.is_object()) {
            self.usage = usage_from(meta);
        }
        if let Some(reason) = non_empty(&response["promptFeedback"]["blockReason"]) {
            self.finish = Some(format!("PROMPT_{reason}"));
        }
        let candidate = &response["candidates"][0];
        let mut emitted = false;
        if let Some(parts) = candidate["content"]["parts"].as_array() {
            for part in parts {
                emitted |= self.part(part, on_delta)?;
            }
        }
        if let Some(reason) = non_empty(&candidate["finishReason"]) {
            self.finish = Some(reason.to_string());
        }
        Ok(emitted)
    }

    fn part(
        &mut self,
        part: &Value,
        on_delta: &mut dyn FnMut(Delta<'_>),
    ) -> Result<bool, LlmError> {
        if part["functionCall"].is_object() {
            return self.call(part, on_delta).map(|()| true);
        }
        let signature = non_empty(&part["thoughtSignature"]);
        let text = part["text"].as_str().unwrap_or("");
        if part["thought"].as_bool() == Some(true) {
            self.push_thinking(text, signature);
            if !text.is_empty() {
                on_delta(Delta::Thinking(text));
            }
        } else if !text.is_empty() {
            self.push_text(text);
            on_delta(Delta::Text(text));
        }
        Ok(!text.is_empty())
    }

    fn push_text(&mut self, piece: &str) {
        if let Some(Block::Text { text }) = self.blocks.last_mut() {
            text.push_str(piece);
        } else {
            let text = piece.to_string();
            self.blocks.push(Block::Text { text });
        }
    }

    /// Thought text joins the open thinking block; a signature that arrives
    /// replaces the kept one, a missing one keeps it (some backends send it
    /// only once per block: pi-ai `retainThoughtSignature`).
    fn push_thinking(&mut self, piece: &str, incoming: Option<&str>) {
        if let Some(Block::Thinking { text, signature }) = self.blocks.last_mut() {
            text.push_str(piece);
            if let Some(incoming) = incoming {
                *signature = Some(incoming.to_string());
            }
        } else if !piece.is_empty() || incoming.is_some() {
            self.blocks.push(Block::Thinking {
                text: piece.to_string(),
                signature: incoming.map(str::to_string),
            });
        }
    }

    fn call(&mut self, part: &Value, on_delta: &mut dyn FnMut(Delta<'_>)) -> Result<(), LlmError> {
        let call = &part["functionCall"];
        let sent = non_empty(&call["name"])
            .ok_or_else(|| protocol("Antigravity sent a function call without a name"))?;
        let name = self.names.original(sent).to_string();
        let id = non_empty(&call["id"])
            .map(str::to_string)
            .unwrap_or_else(new_call_id);
        let arguments = match &call["args"] {
            Value::Null => "{}".to_string(),
            args => args.to_string(),
        };
        let index = self.calls;
        self.calls += 1;
        on_delta(Delta::ToolStart {
            index,
            id: &id,
            name: &name,
        });
        on_delta(Delta::ToolArgs {
            index,
            chunk: &arguments,
        });
        self.blocks.push(Block::ToolCall {
            id,
            name,
            arguments,
            signature: non_empty(&part["thoughtSignature"]).map(str::to_string),
        });
        Ok(())
    }

    /// The assembled message, its usage and stop reason once the body ended.
    /// No finish reason at all, or no content without a refusal or token
    /// limit to explain it, is a `Protocol` error (retryable).
    pub fn finish(self) -> Result<(Message, Usage, StopReason), LlmError> {
        let reason = self
            .finish
            .ok_or_else(|| protocol("stream ended without a finish reason"))?;
        let stop = stop_reason(&reason, self.calls > 0);
        let explained = matches!(stop, StopReason::Refusal | StopReason::MaxTokens);
        if self.blocks.is_empty() && !explained {
            let message = format!("Antigravity answered without any content ({reason})");
            return Err(LlmError::new(ErrorKind::Protocol, message));
        }
        let message = Message {
            role: Role::Assistant,
            content: self.blocks,
            model: None,
        };
        Ok((message, self.usage, stop))
    }
}
