//! The state of one session: messages, id, turn counter, log and wire paths.

use std::io;
use std::path::{Path, PathBuf};

use serde_json::{json, Value};

use crate::llm::ToolSpec;
use crate::session;
use crate::trace::{self, SessionHeader, TraceWriter};
use crate::types::{Block, Message, Role};

/// Content of the error result given to a tool call that has none.
pub const MISSING_RESULT: &str = "interrupted: no result was recorded";

pub struct Conversation {
    pub id: String,
    pub messages: Vec<Message>,
    /// Turns started so far in this session.
    pub turn: u32,
    pub log_path: PathBuf,
    pub wire_dir: PathBuf,
    pub(crate) trace: TraceWriter,
    /// Data of the last `request/header` written in this log.
    pub(crate) last_header: Option<Value>,
}

impl Conversation {
    /// Starts a new session log for `cwd`. `parent` is set for a subagent session.
    pub fn create(app_home: &Path, cwd: &Path, parent: Option<&str>) -> io::Result<Conversation> {
        let id = session::new_id();
        let log_path = session::log_path(app_home, cwd, &id);
        let header = SessionHeader::new(&id, &cwd.to_string_lossy(), parent.map(str::to_string));
        let trace = TraceWriter::create(&log_path, &header)?;
        Ok(Conversation {
            wire_dir: session::wire_dir(app_home, cwd, &id),
            id,
            messages: Vec::new(),
            turn: 0,
            log_path,
            trace,
            last_header: None,
        })
    }

    /// Reopens session `id` of `cwd`: replays and repairs its messages and
    /// continues its log.
    pub fn resume(app_home: &Path, cwd: &Path, id: &str) -> io::Result<Conversation> {
        let log_path = session::log_path(app_home, cwd, id);
        let mut messages = session::replay(&log_path)?;
        repair(&mut messages);
        let (_, events) = trace::read(&log_path)?;
        let turn = events
            .iter()
            .filter(|e| e.kind == trace::kind::TURN_START)
            .filter_map(|e| e.data["turn"].as_u64())
            .max()
            .unwrap_or(0) as u32;
        let last_header = events
            .iter()
            .filter(|e| e.kind == trace::kind::REQUEST_HEADER)
            .max_by_key(|e| e.seq)
            .map(|e| e.data.clone());
        let trace = TraceWriter::open_append(&log_path)?;
        Ok(Conversation {
            id: id.to_string(),
            messages,
            turn,
            wire_dir: session::wire_dir(app_home, cwd, id),
            log_path,
            trace,
            last_header,
        })
    }

    /// Writes `request/header` when `system` or `tools` differ from the last one.
    pub(crate) fn header_if_changed(&mut self, system: &str, tools: &[ToolSpec]) -> io::Result<()> {
        let data = header_data(system, tools);
        if self.last_header.as_ref() == Some(&data) {
            return Ok(());
        }
        self.trace.request_header(system, tools)?;
        self.last_header = Some(data);
        Ok(())
    }
}

/// The data `TraceWriter::request_header` writes, for comparison.
fn header_data(system: &str, tools: &[ToolSpec]) -> Value {
    let tools: Vec<Value> = tools
        .iter()
        .map(|t| json!({ "name": t.name, "description": t.description, "schema": t.schema }))
        .collect();
    json!({ "system": system, "tools": tools })
}

/// Makes a replayed history valid for the model: consecutive tool messages
/// become one, and every tool call without a result gets an error result.
pub fn repair(messages: &mut Vec<Message>) {
    let mut out: Vec<Message> = Vec::with_capacity(messages.len());
    for message in messages.drain(..) {
        match out.last_mut() {
            Some(last) if last.role == Role::Tool && message.role == Role::Tool => {
                last.content.extend(message.content);
            }
            _ => out.push(message),
        }
    }
    let mut index = 0;
    while index < out.len() {
        if out[index].role == Role::Assistant {
            fill_missing(&mut out, index);
        }
        index += 1;
    }
    *messages = out;
}

/// Adds error results for the calls of assistant message `index` that have
/// none in the tool message after it (inserted when absent).
fn fill_missing(out: &mut Vec<Message>, index: usize) {
    let ids: Vec<String> = out[index]
        .tool_calls()
        .map(|(id, _, _)| id.to_string())
        .collect();
    if ids.is_empty() {
        return;
    }
    if out.get(index + 1).map(|m| m.role) != Some(Role::Tool) {
        let empty = Message {
            role: Role::Tool,
            content: Vec::new(),
            model: None,
        };
        out.insert(index + 1, empty);
    }
    let results = &mut out[index + 1].content;
    for id in ids {
        let present = results
            .iter()
            .any(|b| matches!(b, Block::ToolResult { call_id, .. } if *call_id == id));
        if !present {
            results.push(Block::ToolResult {
                call_id: id,
                content: MISSING_RESULT.into(),
                is_error: true,
            });
        }
    }
}
