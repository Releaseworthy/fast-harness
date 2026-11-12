//! The state of one session: messages, id, turn counter, log and wire paths.

use std::io;
use std::path::{Path, PathBuf};
use std::sync::Mutex;

use serde_json::Value;

use super::context::Context;
use crate::llm::ToolSpec;
use crate::session;
use crate::tools::ToolSession;
use crate::trace::{self, SessionHeader, TraceWriter};
use crate::types::{Block, Message, Role};

/// Content of the error result given to a tool call that has none.
pub const MISSING_RESULT: &str = "interrupted: no result was recorded";
/// The error of a resume whose history ends with the model's answer.
pub const NOTHING_TO_RESUME: &str =
    "nothing to resume: the last turn ended with the model's answer";

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
    /// The frozen system prompt (PLAN.md D36): set by the first turn of a
    /// new session, read from the last `request/header` on resume.
    pub system: Option<String>,
    /// The frozen tool list, set and restored the same way.
    pub tools: Option<Vec<ToolSpec>>,
    /// The context stamp of the prompt sources ([`super::context::Context::stamp`]).
    pub context: Option<Value>,
    /// Spill folder and read ranges of this session's tools (PLAN.md D37);
    /// a resume starts with no read ranges.
    pub tool_session: ToolSession,
    /// `context` of the last step (or the estimate after the last
    /// compaction); compared with the compaction threshold (PLAN.md D38).
    pub last_context: u64,
    /// True from a compaction until the next model answer: no second
    /// automatic compaction before a step measured the new context.
    pub compacted: bool,
    /// How the last turn of a reopened session stopped when it can be
    /// resumed (PLAN.md D39); `None` for a new session.
    pub unfinished: Option<session::Unfinished>,
    /// Nesting level of the session (0 top level; a subagent's is above 0).
    pub depth: u32,
    /// Notes of the reopen (a damaged log line skipped, D60), handed out
    /// once by [`Conversation::take_notes`].
    notes: Mutex<Vec<String>>,
}

impl Conversation {
    /// Starts a new session log for `cwd`. `parent` is set for a subagent session.
    pub fn create(app_home: &Path, cwd: &Path, parent: Option<&str>) -> io::Result<Conversation> {
        Conversation::create_at(app_home, cwd, parent, 0)
    }

    /// [`Conversation::create`] for a session at nesting `depth` (PLAN.md D40).
    pub fn create_at(
        app_home: &Path,
        cwd: &Path,
        parent: Option<&str>,
        depth: u32,
    ) -> io::Result<Conversation> {
        let id = session::new_id();
        let log_path = session::log_path(app_home, cwd, &id);
        let mut header =
            SessionHeader::new(&id, &cwd.to_string_lossy(), parent.map(str::to_string));
        header.depth = depth;
        let trace = TraceWriter::create(&log_path, &header)?;
        Ok(Conversation {
            wire_dir: session::wire_dir(app_home, cwd, &id),
            tool_session: ToolSession::new(Some(session::out_dir(app_home, cwd, &id))),
            id,
            messages: Vec::new(),
            turn: 0,
            log_path,
            trace,
            last_header: None,
            system: None,
            tools: None,
            context: None,
            last_context: 0,
            compacted: false,
            unfinished: None,
            depth,
            notes: Mutex::default(),
        })
    }

    /// The seq the next event of the log gets (the append position).
    pub fn next_seq(&self) -> u64 {
        self.trace.next_seq()
    }

    /// The notes of the reopen, each handed out once ([`crate::app::Runtime::agent`]
    /// adds them to the notes of the first agent built for this session).
    pub fn take_notes(&self) -> Vec<String> {
        let mut notes = self.notes.lock().unwrap_or_else(|p| p.into_inner());
        std::mem::take(&mut *notes)
    }

    /// Reopens session `id` of `cwd`: replays and repairs its messages and
    /// continues its log.
    pub fn resume(app_home: &Path, cwd: &Path, id: &str) -> io::Result<Conversation> {
        Ok(Conversation::reopen(app_home, cwd, id)?.0)
    }

    /// [`Conversation::resume`] that also returns the events of the log: the
    /// history, the header data, the append position and these events come
    /// from one parse (G10, D60), so a view builds its transcript from them
    /// (`Transcript::from_trace`) without reading the log again.
    pub fn reopen(
        app_home: &Path,
        cwd: &Path,
        id: &str,
    ) -> io::Result<(Conversation, Vec<trace::TraceEvent>)> {
        // The id comes from a command line or a `serve` client and names a
        // file that is appended to: it must stay inside the sessions folder.
        if !session::valid_id(id) {
            let message = format!("not a session id: {id}");
            return Err(io::Error::new(io::ErrorKind::InvalidInput, message));
        }
        let log_path = session::log_path(app_home, cwd, id);
        let (trace, parsed, mut messages) =
            TraceWriter::open_parsed(&log_path, |p| session::replay_events(&p.events))?;
        repair(&mut messages);
        let mut conv = Conversation::from_log(app_home, cwd, id, &parsed.events, trace);
        conv.messages = messages;
        conv.depth = parsed.header.depth;
        conv.notes = Mutex::new(parsed.note().into_iter().collect());
        Ok((conv, parsed.events))
    }

    /// The state of a reopened log but its messages, depth and notes.
    fn from_log(
        app_home: &Path,
        cwd: &Path,
        id: &str,
        events: &[trace::TraceEvent],
        trace: TraceWriter,
    ) -> Conversation {
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
        let header = last_header.as_ref();
        let (last_context, compacted) = last_context(events);
        Conversation {
            last_context,
            compacted,
            unfinished: session::unfinished(events),
            depth: 0,
            id: id.to_string(),
            messages: Vec::new(),
            turn,
            wire_dir: session::wire_dir(app_home, cwd, id),
            tool_session: ToolSession::new(Some(session::out_dir(app_home, cwd, id))),
            log_path: session::log_path(app_home, cwd, id),
            trace,
            system: header
                .and_then(|h| h["system"].as_str())
                .map(str::to_string),
            tools: header.map(|h| saved_tools(&h["tools"])),
            context: header.and_then(|h| h.get("context")).cloned(),
            last_header,
            notes: Mutex::default(),
        }
    }

    /// Freezes the system prompt and the tool list on the first turn (a
    /// resumed session keeps the logged ones) and moves the context stamp to
    /// `stamp`; returns the context update when the sources changed.
    pub(crate) fn freeze(
        &mut self,
        system: &str,
        tools: &[ToolSpec],
        current: Option<&Context>,
    ) -> Option<String> {
        self.system.get_or_insert_with(|| system.to_string());
        self.tools.get_or_insert_with(|| tools.to_vec());
        let current = current?;
        let note = self.context.as_ref().and_then(|old| current.update(old));
        self.context = Some(current.stamp());
        note
    }

    /// True when the history (repaired) ends with tool results or user text,
    /// so [`super::Agent::resume_turn`] can call the model on it.
    pub fn can_resume(&mut self) -> bool {
        repair(&mut self.messages);
        self.messages
            .last()
            .is_some_and(|m| m.role != Role::Assistant)
    }

    /// Writes `request/header` when the frozen system, tools or context
    /// stamp differ from the last one written.
    pub(crate) fn header_if_changed(&mut self) -> io::Result<()> {
        let (Some(system), Some(tools)) = (&self.system, &self.tools) else {
            return Ok(());
        };
        let data = trace::header_data(system, tools, self.context.as_ref());
        if self.last_header.as_ref() == Some(&data) {
            return Ok(());
        }
        self.trace.request_header_data(data.clone())?;
        self.last_header = Some(data);
        Ok(())
    }
}

/// `context` of the last `assistant/message`, or `after` of a later
/// `compaction` (then `true`).
fn last_context(events: &[trace::TraceEvent]) -> (u64, bool) {
    let mut out = (0, false);
    for event in events {
        match event.kind.as_str() {
            trace::kind::ASSISTANT_MESSAGE => {
                out = (event.data["context"].as_u64().unwrap_or(0), false);
            }
            trace::kind::COMPACTION => out = (event.data["after"].as_u64().unwrap_or(0), true),
            _ => {}
        }
    }
    out
}

/// The tools of a `request/header` (`[{name, description, schema}]`).
fn saved_tools(tools: &Value) -> Vec<ToolSpec> {
    let list = tools.as_array().map(Vec::as_slice).unwrap_or_default();
    list.iter()
        .filter_map(|tool| {
            Some(ToolSpec {
                name: tool["name"].as_str()?.to_string(),
                description: tool["description"].as_str().unwrap_or_default().to_string(),
                schema: tool.get("schema").cloned().unwrap_or(Value::Null),
            })
        })
        .collect()
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
