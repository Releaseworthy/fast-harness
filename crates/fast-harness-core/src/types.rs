//! Provider-neutral conversation model. The same shapes are written to the
//! session log, so keys follow the DSH log style (camelCase fields).

use serde::{Deserialize, Serialize};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
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
    },
    ToolResult {
        call_id: String,
        content: String,
        #[serde(default)]
        is_error: bool,
    },
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Message {
    pub role: Role,
    pub content: Vec<Block>,
}

impl Message {
    pub fn user(text: impl Into<String>) -> Message {
        Message {
            role: Role::User,
            content: vec![Block::Text { text: text.into() }],
        }
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
            } => Some((id.as_str(), name.as_str(), arguments.as_str())),
            _ => None,
        })
    }
}

/// Token counts of one model call. The four fields are disjoint:
/// the prompt size is `input + cache_read + cache_write`.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct Usage {
    #[serde(rename = "inputTokens")]
    pub input: u64,
    #[serde(rename = "outputTokens")]
    pub output: u64,
    #[serde(rename = "cacheReadTokens")]
    pub cache_read: u64,
    #[serde(rename = "cacheWriteTokens")]
    pub cache_write: u64,
}

impl Usage {
    pub fn add(&mut self, other: &Usage) {
        self.input += other.input;
        self.output += other.output;
        self.cache_read += other.cache_read;
        self.cache_write += other.cache_write;
    }

    pub fn prompt(&self) -> u64 {
        self.input + self.cache_read + self.cache_write
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum StopReason {
    /// The model finished its answer.
    End,
    /// The model asked for tool calls: the loop runs them and takes another step.
    ToolUse,
    MaxTokens,
    Refusal,
    Other,
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
                },
            ],
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
