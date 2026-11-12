//! Agent events as Agent Client Protocol v2 `session/update` payloads
//! (PLAN.md D48). Shapes follow
//! <https://agentclientprotocol.com/protocol/v2/prompt-lifecycle.md> and
//! <https://agentclientprotocol.com/protocol/v2/tool-calls.md>.

use std::collections::VecDeque;
use std::sync::{Arc, Mutex};

use serde_json::{json, Value};

use crate::events::AgentEvent;
use crate::types::{Block, Message, Role};

/// A `session/update` notification carrying `update`.
pub fn notification(session: &str, update: Value) -> Value {
    json!({"jsonrpc": "2.0", "method": "session/update",
        "params": {"sessionId": session, "update": update}})
}

/// A `state_update`: `running`, or `idle` with its stop reason and, for
/// `error`, the failure as a JSON-RPC error object.
pub fn state(state: &str, stop: Option<&str>, error: Option<&str>) -> Value {
    let mut update = json!({"sessionUpdate": "state_update", "state": state});
    if let Some(stop) = stop {
        update["stopReason"] = json!(stop);
    }
    if let (Some(message), Some("error")) = (error, stop) {
        update["error"] = json!({"code": -32603, "message": message});
    }
    update
}

fn text(text: &str) -> Value {
    json!({"type": "text", "text": text})
}

/// A `user_message` with the whole text.
pub fn user_message(id: &str, body: &str) -> Value {
    json!({"sessionUpdate": "user_message", "messageId": id, "content": [text(body)]})
}

/// An `agent_message` with the whole text (a local command's answer).
pub fn agent_message(id: &str, body: &str) -> Value {
    json!({"sessionUpdate": "agent_message", "messageId": id, "content": [text(body)]})
}

/// A `notice` (live only, never replayed).
pub fn notice(severity: &str, title: &str) -> Value {
    json!({"sessionUpdate": "notice", "severity": severity, "title": title})
}

/// The id of the user message at `index` of the history: the same on a
/// replay as in the `session/prompt` response that inserted it.
pub fn user_id(index: usize) -> String {
    format!("msg_u{index}")
}

/// The ACP tool kind of a tool name.
pub fn tool_kind(name: &str) -> &'static str {
    match name {
        "read" => "read",
        "write" | "edit" => "edit",
        "grep" | "glob" | "find_tools" => "search",
        "shell" | "bash" | "powershell" => "execute",
        _ if name.starts_with("web") => "fetch",
        _ => "other",
    }
}

/// Ids of prompts queued while a turn ran: each follow-up the turn takes
/// from its inbox reports the id its `session/prompt` response returned.
pub type Queued = Arc<Mutex<VecDeque<String>>>;

/// Maps the events of one turn to updates; keeps the ids it hands out.
pub struct Updates {
    turn: u32,
    step: u32,
    inputs: u32,
    queued: Queued,
}

impl Updates {
    pub fn new(queued: Queued) -> Updates {
        Updates {
            turn: 0,
            step: 0,
            inputs: 0,
            queued,
        }
    }

    fn message_id(&self, kind: &str) -> String {
        format!("msg_{kind}{}_{}", self.turn, self.step)
    }

    /// The update of one event, if it has one.
    pub fn map(&mut self, event: &AgentEvent) -> Option<Value> {
        Some(match event {
            AgentEvent::TurnStart { turn } => {
                (self.turn, self.step) = (*turn, 0);
                return None;
            }
            AgentEvent::StepStart { step, .. } => {
                self.step = *step;
                return None;
            }
            AgentEvent::UserInput { text: body, steer } => {
                let id = self.input_id(*steer);
                user_message(&id, body)
            }
            AgentEvent::TextDelta(delta) => json!({"sessionUpdate": "agent_message_chunk",
                "messageId": self.message_id("a"), "content": text(delta)}),
            AgentEvent::ThinkingDelta(delta) => json!({"sessionUpdate": "agent_thought_chunk",
                "messageId": self.message_id("t"), "content": text(delta)}),
            AgentEvent::ToolCallStart { id, name } => json!({"sessionUpdate": "tool_call_update",
                "toolCallId": id, "title": name, "name": name, "kind": tool_kind(name),
                "status": "pending"}),
            other => return tail(other),
        })
    }

    fn input_id(&mut self, steer: bool) -> String {
        let queued = match steer {
            true => None,
            false => self.queued.lock().ok().and_then(|mut q| q.pop_front()),
        };
        queued.unwrap_or_else(|| {
            self.inputs += 1;
            format!("msg_s{}_{}_{}", self.turn, self.step, self.inputs)
        })
    }
}

fn tail(event: &AgentEvent) -> Option<Value> {
    Some(match event {
        AgentEvent::ToolCall {
            id,
            name,
            arguments,
        } => {
            let raw: Value = serde_json::from_str(arguments).unwrap_or_else(|_| json!(arguments));
            json!({"sessionUpdate": "tool_call_update", "toolCallId": id, "title": name,
                "name": name, "kind": tool_kind(name), "status": "in_progress", "rawInput": raw})
        }
        AgentEvent::ToolResult {
            id,
            content,
            is_error,
            ..
        } => json!({"sessionUpdate": "tool_call_update", "toolCallId": id,
            "status": if *is_error { "failed" } else { "completed" },
            "content": [{"type": "content", "content": text(content)}]}),
        AgentEvent::Retry { error, .. } => notice("warning", &format!("retrying: {error}")),
        AgentEvent::AccountSwitch { from, to, reason } => {
            notice("info", &format!("account {from} -> {to} ({reason})"))
        }
        AgentEvent::Error(message) => notice("error", message),
        AgentEvent::StepEnd {
            context, window, ..
        } if *window > 0 => {
            json!({"sessionUpdate": "usage_update", "used": context, "size": window})
        }
        AgentEvent::Compaction {
            summary,
            before,
            after,
            ..
        } => json!({"sessionUpdate": "compaction_update",
            "compactionId": format!("cmp_{}", crate::trace::now_ms()),
            "status": "completed", "summary": [text(summary)],
            "_meta": {"fh/before": before, "fh/after": after}}),
        _ => return None,
    })
}

/// The retained history as updates for `session/resume` with
/// `replayFrom: {"type": "start"}`: user and agent messages (text only)
/// and tool calls with their final status.
pub fn replay(messages: &[Message]) -> Vec<Value> {
    let mut out = Vec::new();
    for (index, message) in messages.iter().enumerate() {
        let texts: Vec<Value> = (message.content.iter())
            .filter_map(|block| match block {
                Block::Text { text: body } => Some(text(body)),
                _ => None,
            })
            .collect();
        if !texts.is_empty() {
            let (kind, id) = match message.role {
                Role::Assistant => ("agent_message", format!("msg_r{index}")),
                _ => ("user_message", user_id(index)),
            };
            out.push(json!({"sessionUpdate": kind, "messageId": id, "content": texts}));
        }
        out.extend(message.content.iter().filter_map(replay_tool));
    }
    out
}

fn replay_tool(block: &Block) -> Option<Value> {
    match block {
        Block::ToolCall { id, name, .. } => Some(json!({"sessionUpdate": "tool_call_update",
            "toolCallId": id, "title": name, "name": name, "kind": tool_kind(name),
            "status": "in_progress"})),
        Block::ToolResult {
            call_id,
            content,
            is_error,
        } => Some(
            json!({"sessionUpdate": "tool_call_update", "toolCallId": call_id,
            "status": if *is_error { "failed" } else { "completed" },
            "content": [{"type": "content", "content": text(content)}]}),
        ),
        _ => None,
    }
}
