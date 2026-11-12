//! Assembles the SSE events of one Messages stream into a message.
//!
//! Event flow: <https://platform.claude.com/docs/en/build-with-claude/streaming>
//! (read 2026-11-11): `message_start`, per block `content_block_start`,
//! `content_block_delta`*, `content_block_stop`, then `message_delta`
//! (cumulative usage) and `message_stop`; `ping` anywhere; `error` events;
//! unknown event types are ignored.

use serde_json::Value;

use super::error;
use super::request::from_claude_code_name;
use crate::llm::{Delta, ErrorKind, LlmError, ToolSpec};
use crate::types::{Block, Message, Role, StopReason, Usage};

enum Part {
    Text(String),
    Thinking {
        text: String,
        signature: String,
    },
    Redacted(String),
    Tool {
        call: usize,
        id: String,
        name: String,
        json: String,
        /// `input` of the start event, used when no JSON delta arrives.
        initial: Value,
    },
    /// A block type this adapter does not keep (server tools, fallback markers).
    Skipped,
}

pub(crate) struct Assembler<'a> {
    tools: &'a [ToolSpec],
    oauth: bool,
    /// `(stream index, part)` in arrival order.
    parts: Vec<(u64, Part)>,
    tool_calls: usize,
    pub usage: Usage,
    pub stop: Option<StopReason>,
    pub done: bool,
    /// `message_start.message.input_transformations`, when present and not null.
    pub transformations: Option<Value>,
}

fn protocol(message: impl Into<String>) -> LlmError {
    LlmError::new(ErrorKind::Protocol, message)
}

fn map_stop(reason: &str) -> StopReason {
    match reason {
        "end_turn" | "stop_sequence" => StopReason::End,
        "tool_use" => StopReason::ToolUse,
        "max_tokens" => StopReason::MaxTokens,
        "refusal" => StopReason::Refusal,
        _ => StopReason::Other,
    }
}

/// Copies the usage fields present in `usage` (absent or null ones are kept).
fn read_usage(into: &mut Usage, usage: &Value) {
    let fields = [
        ("input_tokens", &mut into.input),
        ("cache_read_input_tokens", &mut into.cache_read),
        ("cache_creation_input_tokens", &mut into.cache_write),
        ("output_tokens", &mut into.output),
    ];
    for (key, slot) in fields {
        if let Some(value) = usage[key].as_u64() {
            *slot = value;
        }
    }
    if let Some(value) = usage["cache_creation"]["ephemeral_1h_input_tokens"].as_u64() {
        into.cache_write_1h = value;
    }
}

impl<'a> Assembler<'a> {
    pub fn new(tools: &'a [ToolSpec], oauth: bool) -> Assembler<'a> {
        Assembler {
            tools,
            oauth,
            parts: Vec::new(),
            tool_calls: 0,
            usage: Usage::default(),
            stop: None,
            done: false,
            transformations: None,
        }
    }

    /// Handles one SSE event; `Ok(true)` when it produced a delta.
    pub fn handle(
        &mut self,
        name: &str,
        data: &str,
        on_delta: &mut dyn FnMut(Delta<'_>),
    ) -> Result<bool, LlmError> {
        let event: Value = serde_json::from_str(data)
            .map_err(|err| protocol(format!("bad JSON in `{name}` event: {err}")))?;
        let kind = event["type"].as_str().unwrap_or(name);
        match kind {
            "message_start" => {
                read_usage(&mut self.usage, &event["message"]["usage"]);
                let changes = &event["message"]["input_transformations"];
                self.transformations = (!changes.is_null()).then(|| changes.clone());
            }
            "content_block_start" => return self.start(&event, on_delta),
            "content_block_delta" => return self.delta(&event, on_delta),
            "message_delta" => {
                if let Some(reason) = event["delta"]["stop_reason"].as_str() {
                    self.stop = Some(map_stop(reason));
                }
                read_usage(&mut self.usage, &event["usage"]);
            }
            "message_stop" => self.done = true,
            "error" => return Err(error::from_event(&event)),
            // `content_block_stop`, `ping` and unknown events change nothing here.
            _ => {}
        }
        Ok(false)
    }

    fn start(
        &mut self,
        event: &Value,
        on_delta: &mut dyn FnMut(Delta<'_>),
    ) -> Result<bool, LlmError> {
        let index = event["index"].as_u64().unwrap_or(self.parts.len() as u64);
        let block = &event["content_block"];
        let text = |key: &str| block[key].as_str().unwrap_or_default().to_string();
        let part = match block["type"].as_str().unwrap_or_default() {
            "text" => Part::Text(text("text")),
            "thinking" => Part::Thinking {
                text: text("thinking"),
                signature: text("signature"),
            },
            "redacted_thinking" => Part::Redacted(text("data")),
            "tool_use" => return Ok(self.start_tool(index, block, on_delta)),
            "fallback" if !self.parts.is_empty() => {
                return Err(protocol("unsupported mid-output model fallback"));
            }
            _ => Part::Skipped,
        };
        self.parts.push((index, part));
        Ok(false)
    }

    fn start_tool(
        &mut self,
        index: u64,
        block: &Value,
        on_delta: &mut dyn FnMut(Delta<'_>),
    ) -> bool {
        let raw_name = block["name"].as_str().unwrap_or_default();
        let name = if self.oauth {
            from_claude_code_name(raw_name, self.tools)
        } else {
            raw_name.to_string()
        };
        let id = block["id"].as_str().unwrap_or_default().to_string();
        let call = self.tool_calls;
        self.tool_calls += 1;
        on_delta(Delta::ToolStart {
            index: call,
            id: &id,
            name: &name,
        });
        let initial = block["input"].clone();
        let json = String::new();
        self.parts.push((
            index,
            Part::Tool {
                call,
                id,
                name,
                json,
                initial,
            },
        ));
        true
    }

    fn delta(
        &mut self,
        event: &Value,
        on_delta: &mut dyn FnMut(Delta<'_>),
    ) -> Result<bool, LlmError> {
        let index = event["index"].as_u64();
        let delta = &event["delta"];
        let kind = delta["type"].as_str().unwrap_or_default();
        let key = match kind {
            "text_delta" => "text",
            "thinking_delta" => "thinking",
            "signature_delta" => "signature",
            "input_json_delta" => "partial_json",
            _ => return Ok(false),
        };
        let piece = delta[key].as_str().unwrap_or_default();
        let Some((_, part)) = self.parts.iter_mut().rev().find(|(i, _)| Some(*i) == index) else {
            return Err(protocol("delta for a block that was never started"));
        };
        if piece.is_empty() {
            return Ok(false);
        }
        match (kind, part) {
            ("text_delta", Part::Text(text)) => {
                text.push_str(piece);
                on_delta(Delta::Text(piece));
            }
            ("thinking_delta", Part::Thinking { text, .. }) => {
                text.push_str(piece);
                on_delta(Delta::Thinking(piece));
            }
            ("signature_delta", Part::Thinking { signature, .. }) => {
                signature.push_str(piece);
                return Ok(false);
            }
            ("input_json_delta", Part::Tool { call, json, .. }) => {
                json.push_str(piece);
                let index = *call;
                on_delta(Delta::ToolArgs {
                    index,
                    chunk: piece,
                });
            }
            _ => return Ok(false),
        }
        Ok(true)
    }

    /// The assembled assistant message.
    pub fn message(self) -> Message {
        let content = self
            .parts
            .into_iter()
            .filter_map(|(_, part)| finish(part))
            .collect();
        Message {
            role: Role::Assistant,
            content,
            model: None,
        }
    }
}

fn finish(part: Part) -> Option<Block> {
    Some(match part {
        Part::Text(text) => Block::Text { text },
        Part::Thinking { text, signature } => Block::Thinking {
            text,
            signature: (!signature.is_empty()).then_some(signature),
        },
        Part::Redacted(data) => Block::RedactedThinking { data },
        Part::Tool {
            id,
            name,
            json,
            initial,
            ..
        } => {
            let arguments = if !json.trim().is_empty() {
                json
            } else if initial.as_object().is_some_and(|map| !map.is_empty()) {
                initial.to_string()
            } else {
                "{}".to_string()
            };
            Block::ToolCall {
                id,
                name,
                arguments,
                signature: None,
            }
        }
        Part::Skipped => return None,
    })
}
