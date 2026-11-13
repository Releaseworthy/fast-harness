//! Provider-neutral conversation model. The same shapes are written to the
//! session log, so keys follow the DSH log style (camelCase fields).

use serde::{Deserialize, Serialize};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
#[non_exhaustive]
pub enum Role {
    User,
    Assistant,
    /// Carries only `Block::ToolResult` blocks: every result of one step.
    Tool,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(
    tag = "type",
    rename_all = "snake_case",
    rename_all_fields = "camelCase"
)]
#[non_exhaustive]
pub enum Block {
    Text {
        text: String,
    },
    /// Model reasoning. `signature` must go back to the provider unchanged.
    Thinking {
        text: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        signature: Option<String>,
    },
    /// Encrypted reasoning, sent back verbatim.
    RedactedThinking {
        data: String,
    },
    /// `arguments` is the raw JSON object text exactly as the model produced it.
    ToolCall {
        id: String,
        name: String,
        arguments: String,
        /// The Gemini thought signature of this call (written by branch `antigravity`, D53), sent back
        /// unchanged only to the model that produced it.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        signature: Option<String>,
    },
    ToolResult {
        call_id: String,
        content: String,
        #[serde(default)]
        is_error: bool,
    },
    /// An image held by reference (trace version 13, PLAN.md D70). In a user
    /// message it is the user's; in a tool message it belongs to the
    /// `ToolResult` before it. The bytes are read when a request is built.
    Image(ImageRef),
}

/// An image file kept under the session folder (`<session>.media/<sha>.<ext>`,
/// PLAN.md D70): never inline in the trace.
#[derive(Clone, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
#[non_exhaustive]
pub struct ImageRef {
    /// Absolute path of the media file.
    pub path: String,
    /// `image/png`, `image/jpeg`, `image/gif` or `image/webp`.
    pub media_type: String,
    pub bytes: u64,
    pub width: u32,
    pub height: u32,
}

impl ImageRef {
    pub fn new(
        path: impl Into<String>,
        media_type: impl Into<String>,
        bytes: u64,
        (width, height): (u32, u32),
    ) -> ImageRef {
        ImageRef {
            path: path.into(),
            media_type: media_type.into(),
            bytes,
            width,
            height,
        }
    }
}

impl Block {
    /// A text block.
    pub fn text(text: impl Into<String>) -> Block {
        Block::Text { text: text.into() }
    }

    /// A tool result block.
    pub fn tool_result(
        call_id: impl Into<String>,
        content: impl Into<String>,
        is_error: bool,
    ) -> Block {
        Block::ToolResult {
            call_id: call_id.into(),
            content: content.into(),
            is_error,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[non_exhaustive]
pub struct Message {
    pub role: Role,
    pub content: Vec<Block>,
    /// The full model id that produced an assistant message (`claude-...` or
    /// `<provider id>/<id>`); None for user and tool messages and for logs
    /// written before trace version 3.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub model: Option<String>,
}

impl Message {
    /// A message without a model id (set it with [`Message::with_model`]).
    pub fn new(role: Role, content: Vec<Block>) -> Message {
        Message {
            role,
            content,
            model: None,
        }
    }

    pub fn user(text: impl Into<String>) -> Message {
        Message::new(Role::User, vec![Block::text(text)])
    }

    /// The images of this message, in order.
    pub fn images(&self) -> impl Iterator<Item = &ImageRef> {
        self.content.iter().filter_map(|block| match block {
            Block::Image(image) => Some(image),
            _ => None,
        })
    }

    /// An assistant message of one text block.
    pub fn assistant(text: impl Into<String>) -> Message {
        Message::new(Role::Assistant, vec![Block::text(text)])
    }

    /// The model id that produced this (assistant) message.
    pub fn with_model(mut self, model: impl Into<String>) -> Message {
        self.model = Some(model.into());
        self
    }

    /// All text blocks joined, without reasoning or tool blocks.
    pub fn text(&self) -> String {
        let mut out = String::new();
        for block in &self.content {
            if let Block::Text { text } = block {
                out.push_str(text);
            }
        }
        out
    }

    /// `(id, name, raw arguments)` of every tool call, in order.
    pub fn tool_calls(&self) -> impl Iterator<Item = (&str, &str, &str)> {
        self.content.iter().filter_map(|block| match block {
            Block::ToolCall {
                id,
                name,
                arguments,
                ..
            } => Some((id.as_str(), name.as_str(), arguments.as_str())),
            _ => None,
        })
    }
}

/// Token counts of one model call. The first four fields are disjoint:
/// the prompt size is `input + cache_read + cache_write`. `cache_write_1h` is
/// the part of `cache_write` written with the 1 hour lifetime (Anthropic
/// `usage.cache_creation.ephemeral_1h_input_tokens`; 0 for other providers).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
#[non_exhaustive]
pub struct Usage {
    #[serde(rename = "inputTokens")]
    pub input: u64,
    #[serde(rename = "outputTokens")]
    pub output: u64,
    #[serde(rename = "cacheReadTokens")]
    pub cache_read: u64,
    #[serde(rename = "cacheWriteTokens")]
    pub cache_write: u64,
    #[serde(rename = "cacheWrite1hTokens", skip_serializing_if = "is_zero")]
    pub cache_write_1h: u64,
}

fn is_zero(value: &u64) -> bool {
    *value == 0
}

impl Usage {
    /// The four disjoint counts; `cache_write_1h` is 0.
    pub fn new(input: u64, output: u64, cache_read: u64, cache_write: u64) -> Usage {
        Usage {
            input,
            output,
            cache_read,
            cache_write,
            cache_write_1h: 0,
        }
    }

    /// The part of `cache_write` written with the 1 hour lifetime.
    pub fn with_cache_write_1h(mut self, tokens: u64) -> Usage {
        self.cache_write_1h = tokens;
        self
    }

    pub fn add(&mut self, other: &Usage) {
        self.input += other.input;
        self.output += other.output;
        self.cache_read += other.cache_read;
        self.cache_write += other.cache_write;
        self.cache_write_1h += other.cache_write_1h;
    }

    pub fn prompt(&self) -> u64 {
        self.input + self.cache_read + self.cache_write
    }

    /// The context after this call: the prompt plus the output.
    pub fn context(&self) -> u64 {
        self.prompt() + self.output
    }

    /// `cache_read / prompt` in percent; `None` for an empty prompt.
    pub fn cache_hit_percent(&self) -> Option<u64> {
        let prompt = self.prompt();
        (prompt > 0).then(|| self.cache_read * 100 / prompt)
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
#[non_exhaustive]
pub enum StopReason {
    /// The model finished its answer.
    End,
    /// The model asked for tool calls: the loop runs them and takes another step.
    ToolUse,
    /// The output token limit cut the message (PLAN.md D36: outcome `Truncated`).
    MaxTokens,
    Refusal,
    /// Claude `pause_turn`: the turn goes on with the paused message sent back.
    PauseTurn,
    /// Claude `model_context_window_exceeded`: the prompt fills the window.
    ContextWindow,
    Other,
}

/// FNV-1a 64 of `text`: a stable hash (same value in every process and
/// version) for the context stamp of a session and derived ids.
pub fn stable_hash(text: &str) -> u64 {
    text.bytes().fold(0xcbf2_9ce4_8422_2325, |hash, byte| {
        (hash ^ u64::from(byte)).wrapping_mul(0x0000_0100_0000_01b3)
    })
}

/// Unix time in milliseconds; 0 if the clock is before 1970.
/// Here, in the bottom layer, because `tools` and `trace` both need it and
/// `tools` may not import `trace` (the layering test of PLAN.md phase 30);
/// `trace` re-exports it.
pub fn now_ms() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|elapsed| elapsed.as_millis() as u64)
        .unwrap_or(0)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn message_serializes_in_log_shape() {
        let message = Message {
            role: Role::Assistant,
            content: vec![
                Block::Thinking {
                    text: "plan".into(),
                    signature: Some("sig".into()),
                },
                Block::Text { text: "hi".into() },
                Block::ToolCall {
                    id: "c1".into(),
                    name: "read".into(),
                    arguments: "{}".into(),
                    signature: None,
                },
            ],
            model: None,
        };
        let json = serde_json::to_string(&message).unwrap();
        assert_eq!(
            json,
            r#"{"role":"assistant","content":[{"type":"thinking","text":"plan","signature":"sig"},{"type":"text","text":"hi"},{"type":"tool_call","id":"c1","name":"read","arguments":"{}"}]}"#
        );
        let back: Message = serde_json::from_str(&json).unwrap();
        assert_eq!(back, message);
        assert_eq!(
            back.tool_calls().collect::<Vec<_>>(),
            vec![("c1", "read", "{}")]
        );
    }

    #[test]
    fn message_and_tool_call_round_trip_with_model_and_signature() {
        let with_provenance = Message {
            role: Role::Assistant,
            content: vec![Block::ToolCall {
                id: "c1".into(),
                name: "read".into(),
                arguments: "{}".into(),
                signature: Some("gemini-sig".into()),
            }],
            model: Some("antigravity/gemini-2.5-pro".into()),
        };
        let json = serde_json::to_string(&with_provenance).unwrap();
        assert!(json.contains(r#""model":"antigravity/gemini-2.5-pro""#));
        assert!(json.contains(r#""signature":"gemini-sig""#));
        let back: Message = serde_json::from_str(&json).unwrap();
        assert_eq!(back, with_provenance);

        let without_provenance = Message {
            role: Role::Assistant,
            content: vec![Block::ToolCall {
                id: "c1".into(),
                name: "read".into(),
                arguments: "{}".into(),
                signature: None,
            }],
            model: None,
        };
        let json_without = serde_json::to_string(&without_provenance).unwrap();
        assert!(!json_without.contains("model"));
        assert!(!json_without.contains("signature"));
        let back_without: Message = serde_json::from_str(&json_without).unwrap();
        assert_eq!(back_without, without_provenance);
    }

    #[test]
    fn tool_result_and_usage_keys() {
        let block = Block::ToolResult {
            call_id: "c1".into(),
            content: "ok".into(),
            is_error: true,
        };
        assert_eq!(
            serde_json::to_string(&block).unwrap(),
            r#"{"type":"tool_result","callId":"c1","content":"ok","isError":true}"#
        );
        let usage = Usage {
            input: 4,
            output: 9,
            cache_read: 100,
            cache_write: 20,
            cache_write_1h: 0,
        };
        let json = serde_json::to_string(&usage).unwrap();
        assert_eq!(
            json,
            r#"{"inputTokens":4,"outputTokens":9,"cacheReadTokens":100,"cacheWriteTokens":20}"#
        );
        assert_eq!(serde_json::from_str::<Usage>(&json).unwrap(), usage);
        assert_eq!(usage.prompt(), 124);
        assert_eq!(
            serde_json::to_string(&StopReason::ToolUse).unwrap(),
            r#""tool_use""#
        );
    }
}
