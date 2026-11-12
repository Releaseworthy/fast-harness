//! A readable timeline of a session log: what the user sent, what each model
//! call carried and what the model returned (thinking, text, tool calls),
//! the tool results, retries, account switches and turn totals.
//! The TUI, the GUI and `fast-harness trace` all show these entries.

use crate::llm::find;
use crate::trace::{kind, TraceEvent};

/// "Tool call read", or for a find "Tool call find_tools: read, edit".
fn call_title(name: &str, arguments: &str) -> String {
    match find::call_label(arguments) {
        Some(label) if name == find::FIND_TOOLS => format!("Tool call {name}: {label}"),
        _ => format!("Tool call {name}"),
    }
}
use crate::types::{Block, Message, Usage};
use serde_json::Value;
use std::collections::{HashMap, HashSet};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum EntryKind {
    Turn,
    User,
    Header,
    Request,
    Thinking,
    Response,
    ToolCall,
    ToolResult,
    Answer,
    Retry,
    Failure,
    Switch,
    TurnEnd,
}

/// The tool call behind a ToolCall or ToolResult entry.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct EntryTool {
    pub name: String,
    pub arguments: String,
    pub output: Option<String>,
    pub duration_ms: Option<u64>,
}

/// One line of the timeline with its full text.
#[derive(Clone, Debug, PartialEq)]
pub struct Entry {
    pub kind: EntryKind,
    pub turn: u32,
    /// 0 outside a step.
    pub step: u32,
    pub title: String,
    /// Full text, may be empty.
    pub body: String,
    pub is_error: bool,
    /// The call behind a ToolCall or ToolResult entry, else None.
    pub tool: Option<EntryTool>,
}

/// The timeline of `events`, in their order. Missing or odd data keys give
/// defaults (0, empty text), never a panic.
pub fn entries(events: &[TraceEvent]) -> Vec<Entry> {
    let mut state = State::default();
    for event in events {
        state.event(event);
    }
    state.out
}

/// Titles on their own line, bodies indented by 4 spaces, a blank line
/// before every `Turn` entry but the first.
pub fn format_text(entries: &[Entry]) -> String {
    let mut out = String::new();
    let mut turns = 0;
    for entry in entries {
        if entry.kind == EntryKind::Turn {
            if turns > 0 {
                out.push('\n');
            }
            turns += 1;
        }
        out.push_str(&entry.title);
        out.push('\n');
        for line in entry.body.lines() {
            if !line.is_empty() {
                out.push_str("    ");
                out.push_str(line);
            }
            out.push('\n');
        }
    }
    out
}

#[derive(Default)]
struct State {
    turn: u32,
    step: u32,
    /// Tool name by call id.
    names: HashMap<String, String>,
    /// Tool arguments by call id.
    arguments: HashMap<String, String>,
    /// Call ids already shown from an assistant message.
    shown: HashSet<String>,
    out: Vec<Entry>,
}

impl State {
    fn event(&mut self, event: &TraceEvent) {
        let data = &event.data;
        if let Some(turn) = num(data, "turn") {
            self.turn = turn as u32;
        }
        if let Some(step) = num(data, "step") {
            self.step = step as u32;
        }
        match event.kind.as_str() {
            kind::TURN_START => {
                self.step = 0;
                let title = format!("Turn {}", self.turn);
                self.push(EntryKind::Turn, title, String::new());
            }
            kind::USER_MESSAGE => {
                let body = message(data).map(|m| m.text()).unwrap_or_default();
                self.push(EntryKind::User, "User".into(), body);
            }
            kind::REQUEST_HEADER => self.header(data),
            kind::REQUEST_CONTEXT => self.request(data),
            kind::ASSISTANT_MESSAGE => self.assistant(data),
            kind::TOOL_CALL => self.tool_call(data),
            kind::TOOL_RESULT => self.tool_result(data),
            kind::ASSISTANT_ATTEMPT => self.failure(data),
            kind::LLM_RETRY => {
                let title = format!(
                    "Retry {} in {} ms",
                    int(data, "attempt"),
                    int(data, "delayMs")
                );
                self.push(EntryKind::Retry, title, text(data, "error").into());
            }
            kind::ACCOUNT_SWITCH => {
                let (from, to) = (text(data, "from"), text(data, "to"));
                let title = format!("Account switch {from} → {to} · {}", text(data, "reason"));
                self.push(EntryKind::Switch, title, String::new());
            }
            kind::TURN_END => self.turn_end(data),
            _ => {}
        }
    }

    fn push(&mut self, kind: EntryKind, title: String, body: String) {
        let (turn, step) = (self.turn, self.step);
        self.out.push(Entry {
            kind,
            turn,
            step,
            title,
            body,
            is_error: false,
            tool: None,
        });
    }

    fn header(&mut self, data: &Value) {
        let tools = data.get("tools").and_then(Value::as_array);
        let names: Vec<&str> = tools
            .map(|list| list.iter().map(|tool| text(tool, "name")).collect())
            .unwrap_or_default();
        let system = text(data, "system");
        let title = format!(
            "Request header · system prompt {} chars · {} tools: {}",
            system.chars().count(),
            names.len(),
            names.join(", ")
        );
        self.push(EntryKind::Header, title, system.into());
    }

    fn request(&mut self, data: &Value) {
        let effort = data
            .get("effort")
            .and_then(Value::as_str)
            .unwrap_or("default");
        let mut title = format!(
            "Step {} request · {} · effort {effort} · account {} · attempt {}",
            self.step,
            text(data, "model"),
            text(data, "account"),
            int(data, "attempt")
        );
        let wire = text(data, "wire");
        if !wire.is_empty() {
            title.push_str(&format!(" · wire {wire}"));
        }
        self.push(EntryKind::Request, title, String::new());
    }

    fn assistant(&mut self, data: &Value) {
        for block in message(data).map(|m| m.content).unwrap_or_default() {
            match block {
                Block::Thinking { text, .. } => {
                    self.push(EntryKind::Thinking, "Thinking".into(), text)
                }
                Block::RedactedThinking { .. } => self.push(
                    EntryKind::Thinking,
                    "Thinking (redacted)".into(),
                    String::new(),
                ),
                Block::Text { text } if !text.is_empty() => {
                    self.push(EntryKind::Response, "Response".into(), text)
                }
                Block::ToolCall {
                    id,
                    name,
                    arguments,
                    ..
                } => {
                    self.shown.insert(id.clone());
                    self.call(&id, &name, &arguments);
                }
                _ => {}
            }
        }
        let timing = data.get("timing").unwrap_or(&Value::Null);
        let usage = usage(data);
        let title = format!(
            "Step {} answer · stop {} · TTFB {} ms · total {} ms{}",
            self.step,
            data.get("stop").and_then(Value::as_str).unwrap_or("other"),
            int(timing, "ttfbMs"),
            int(timing, "totalMs"),
            usage_text(&usage)
        );
        self.push(EntryKind::Answer, title, String::new());
    }

    fn tool_call(&mut self, data: &Value) {
        let (id, name) = (text(data, "callId"), text(data, "name"));
        if self.shown.contains(id) {
            self.names.insert(id.to_string(), name.to_string());
        } else {
            self.call(id, name, text(data, "arguments"));
        }
    }

    /// A ToolCall entry; remembers the name and arguments for its result.
    fn call(&mut self, id: &str, name: &str, arguments: &str) {
        self.names.insert(id.to_string(), name.to_string());
        self.arguments.insert(id.to_string(), arguments.to_string());
        let title = call_title(name, arguments);
        self.push(EntryKind::ToolCall, title, pretty(arguments));
        self.attach(EntryTool {
            name: name.to_string(),
            arguments: arguments.to_string(),
            ..EntryTool::default()
        });
    }

    fn attach(&mut self, tool: EntryTool) {
        if let Some(entry) = self.out.last_mut() {
            entry.tool = Some(tool);
        }
    }

    fn tool_result(&mut self, data: &Value) {
        let duration = int(data, "durationMs");
        for block in message(data).map(|m| m.content).unwrap_or_default() {
            if let Block::ToolResult {
                call_id,
                content,
                is_error,
            } = block
            {
                let name = self.names.get(&call_id).cloned().unwrap_or(call_id.clone());
                let word = if is_error { "error" } else { "result" };
                let title = format!("Tool {word} {name} · {duration} ms");
                self.push(EntryKind::ToolResult, title, content.clone());
                self.mark_error(is_error);
                self.attach(EntryTool {
                    name,
                    arguments: self.arguments.get(&call_id).cloned().unwrap_or_default(),
                    output: Some(content),
                    duration_ms: num(data, "durationMs"),
                });
            }
        }
    }

    fn failure(&mut self, data: &Value) {
        let total = int(data.get("timing").unwrap_or(&Value::Null), "totalMs");
        let title = format!("Attempt {} failed after {total} ms", int(data, "attempt"));
        self.push(EntryKind::Failure, title, text(data, "error").into());
        self.mark_error(true);
    }

    fn turn_end(&mut self, data: &Value) {
        self.step = 0;
        let steps = int(data, "steps");
        let plural = if steps == 1 { "step" } else { "steps" };
        let title = format!(
            "Turn {} {} · {steps} {plural} · {} ms{}",
            self.turn,
            text(data, "outcome"),
            int(data, "durationMs"),
            usage_text(&usage(data))
        );
        self.push(EntryKind::TurnEnd, title, String::new());
    }

    fn mark_error(&mut self, is_error: bool) {
        if let Some(last) = self.out.last_mut() {
            last.is_error = is_error;
        }
    }
}

fn num(data: &Value, key: &str) -> Option<u64> {
    data.get(key).and_then(Value::as_u64)
}

fn int(data: &Value, key: &str) -> u64 {
    num(data, key).unwrap_or(0)
}

fn text<'a>(data: &'a Value, key: &str) -> &'a str {
    data.get(key).and_then(Value::as_str).unwrap_or("")
}

fn message(data: &Value) -> Option<Message> {
    serde_json::from_value(data.get("message")?.clone()).ok()
}

fn usage(data: &Value) -> Usage {
    data.get("usage")
        .and_then(|value| serde_json::from_value(value.clone()).ok())
        .unwrap_or_default()
}

fn usage_text(usage: &Usage) -> String {
    format!(
        " · in {} · out {} · cache read {} · cache write {}",
        usage.input, usage.output, usage.cache_read, usage.cache_write
    )
}

/// Tool arguments pretty-printed when they are JSON, else as written.
fn pretty(arguments: &str) -> String {
    serde_json::from_str::<Value>(arguments)
        .ok()
        .and_then(|value| serde_json::to_string_pretty(&value).ok())
        .unwrap_or_else(|| arguments.to_string())
}
