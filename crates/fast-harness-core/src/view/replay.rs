//! Building a [`Transcript`] from a replayed conversation or a session log.

use super::{
    compaction_text, new_tool, retry_text, switch_text, turn_end_text, Item, NoteKind, Transcript,
};
use crate::llm::Timing;
use crate::trace::{kind, TraceEvent};
use crate::types::{Block, Message, Role, StopReason, Usage};
use serde::de::DeserializeOwned;
use serde_json::Value;

impl Transcript {
    /// User text, assistant text and reasoning, and tool calls joined with
    /// their results by call id. Turns count user messages with text; steps
    /// count assistant messages within a turn.
    pub fn from_messages(messages: &[Message]) -> Transcript {
        let mut view = Transcript::new();
        for message in messages {
            match message.role {
                Role::Assistant => {
                    view.step += 1;
                    view.add_assistant(view.turn, view.step, message);
                }
                Role::User | Role::Tool => {
                    if view.add_user(message) {
                        view.turn += 1;
                        view.step = 0;
                    }
                    view.add_results(message, None);
                }
            }
        }
        view.revision = view.items.len() as u64;
        view
    }

    /// A saved session log: the same as [`Transcript::from_messages`] plus
    /// step footers, retry and account switch notes, and tool durations.
    pub fn from_trace(events: &[TraceEvent]) -> Transcript {
        let mut replay = Replay::default();
        for event in events {
            replay.event(event);
        }
        replay.flush();
        let mut view = replay.view;
        view.revision = view.items.len() as u64;
        view
    }

    /// Adds a user item when the message has text; true if it did.
    fn add_user(&mut self, message: &Message) -> bool {
        let text = message.text();
        if text.is_empty() {
            return false;
        }
        self.items.push(Item::User { text });
        true
    }

    fn add_assistant(&mut self, turn: u32, step: u32, message: &Message) {
        let mut thinking = String::new();
        for block in &message.content {
            if let Block::Thinking { text, .. } = block {
                thinking.push_str(text);
            }
        }
        let text = message.text();
        if !text.is_empty() || !thinking.is_empty() {
            self.items.push(Item::Assistant {
                turn,
                step,
                text,
                thinking,
            });
        }
        for (id, name, arguments) in message.tool_calls() {
            self.items.push(new_tool(id, name, arguments.to_string()));
        }
    }

    fn add_results(&mut self, message: &Message, duration_ms: Option<u64>) {
        for block in &message.content {
            if let Block::ToolResult {
                call_id,
                content,
                is_error,
            } = block
            {
                self.fill_result(call_id, content, *is_error, duration_ms);
            }
        }
    }
}

/// Replay state: the footer of the current step waits for `step/end`.
#[derive(Default)]
struct Replay {
    view: Transcript,
    footer: Option<(u32, u32, Usage, Timing, StopReason, String)>,
}

impl Replay {
    fn event(&mut self, event: &TraceEvent) {
        let data = &event.data;
        match event.kind.as_str() {
            kind::USER_MESSAGE => {
                self.flush();
                self.view.turn = int(data, "turn");
                if let Some(message) = parse::<Message>(data, "message") {
                    self.view.add_user(&message);
                    self.view.add_results(&message, None);
                }
            }
            kind::STEP_START => {
                self.flush();
                self.view.turn = int(data, "turn");
                self.view.step = int(data, "step");
            }
            kind::REQUEST_CONTEXT => {
                self.view.model = text(data, "model");
                self.view.account = text(data, "account");
            }
            kind::LLM_RETRY | kind::ACCOUNT_SWITCH => self.note(&event.kind, data),
            kind::ASSISTANT_MESSAGE => self.assistant(data),
            kind::TOOL_CALL => {
                let id = text(data, "callId");
                if self.view.tool_mut(&id).is_none() {
                    let tool = new_tool(&id, &text(data, "name"), text(data, "arguments"));
                    self.view.items.push(tool);
                }
            }
            kind::TOOL_RESULT => {
                if let Some(message) = parse::<Message>(data, "message") {
                    let duration = data.get("durationMs").and_then(Value::as_u64);
                    self.view.add_results(&message, duration);
                }
            }
            kind::COMPACTION => {
                self.flush();
                let auto = data["auto"].as_bool().unwrap_or(true);
                let text = compaction_text(auto, num(data, "before"), num(data, "after"));
                self.view.note(NoteKind::Info, text);
                self.view.context = num(data, "after");
            }
            kind::STEP_END => self.flush(),
            kind::TURN_END => self.turn_end(data),
            _ => {}
        }
    }

    /// A retry or account switch note.
    fn note(&mut self, event_kind: &str, data: &Value) {
        if event_kind == kind::LLM_RETRY {
            let note = retry_text(
                int(data, "attempt"),
                num(data, "delayMs"),
                &text(data, "error"),
            );
            self.view.note(NoteKind::Retry, note);
        } else {
            let to = text(data, "to");
            let note = switch_text(&text(data, "from"), &to, &text(data, "reason"));
            self.view.note(NoteKind::AccountSwitch, note);
            self.view.account = to;
        }
    }

    fn assistant(&mut self, data: &Value) {
        self.flush();
        let (turn, step) = (int(data, "turn"), int(data, "step"));
        if let Some(message) = parse::<Message>(data, "message") {
            self.view.add_assistant(turn, step, &message);
        }
        let usage: Usage = parse(data, "usage").unwrap_or_default();
        // Trace version 4 keys; older logs leave the window unknown.
        self.view.context = data["context"].as_u64().unwrap_or(usage.context());
        self.view.window = data["window"].as_u64().unwrap_or(0);
        let timing = parse(data, "timing").unwrap_or_default();
        let stop = parse(data, "stop").unwrap_or(StopReason::Other);
        let account = match data.get("account").and_then(Value::as_str) {
            Some(account) => account.to_string(),
            None => self.view.account.clone(),
        };
        self.footer = Some((turn, step, usage, timing, stop, account));
    }

    fn turn_end(&mut self, data: &Value) {
        self.flush();
        let outcome = text(data, "outcome");
        if !outcome.is_empty() && outcome != "done" {
            self.view.note(NoteKind::Info, turn_end_text(&outcome));
        }
    }

    /// Writes the pending footer, if any.
    fn flush(&mut self) {
        if let Some((turn, step, usage, timing, stop, account)) = self.footer.take() {
            let live_account = std::mem::replace(&mut self.view.account, account);
            self.view.footer(turn, step, usage, timing, stop);
            self.view.account = live_account;
        }
    }
}

fn parse<T: DeserializeOwned>(data: &Value, key: &str) -> Option<T> {
    data.get(key)
        .and_then(|value| serde_json::from_value(value.clone()).ok())
}

fn num(data: &Value, key: &str) -> u64 {
    data.get(key).and_then(Value::as_u64).unwrap_or(0)
}

fn int(data: &Value, key: &str) -> u32 {
    u32::try_from(num(data, key)).unwrap_or(u32::MAX)
}

fn text(data: &Value, key: &str) -> String {
    data.get(key)
        .and_then(Value::as_str)
        .unwrap_or_default()
        .to_string()
}
