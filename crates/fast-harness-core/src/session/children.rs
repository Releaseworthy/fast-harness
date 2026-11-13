//! The subagent sessions of a session (PLAN.md D74): every log of the folder
//! whose header names it as the parent, with what its log says about the
//! child's last turn, so a front end lists them after a restart. The label
//! (the `subagent` call's `description`) comes from the parent's log: the
//! `tool/result` of that call names the child's session id.

use super::{dir, index, summarize, Unfinished};
use crate::trace::{self, kind, Record, TraceEvent};
use serde_json::Value;
use std::collections::HashMap;
use std::io;
use std::path::Path;

/// One subagent session as its log tells it.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Child {
    pub id: String,
    /// The `description` of the `subagent` call, else the first prompt's
    /// title.
    pub label: String,
    /// 1 for a child of a top session (the header's `depth`).
    pub depth: u32,
    /// Unix milliseconds: the log's creation and its last event.
    pub created_at: u64,
    pub last_time: u64,
    /// The last step of the last turn, and its last tool call.
    pub step: u32,
    pub tool: Option<String>,
    /// The last turn's `turn/end` outcome (`done`, `failed`, ...), `None`
    /// while no turn ended.
    pub outcome: Option<String>,
    pub unfinished: Option<Unfinished>,
    /// The model and effort of the latest request (D79), empty and `None`
    /// before the first one.
    pub model: String,
    pub effort: Option<String>,
    /// Every token of the log: input, output, cache read and cache write.
    pub tokens: u64,
}

/// The children of session `parent` in `cwd`, oldest first. Reads the first
/// line of every log of the folder and all of the matching ones: never on
/// the UI thread. A missing folder is an empty list.
pub fn children(app_home: &Path, cwd: &Path, parent: &str) -> io::Result<Vec<Child>> {
    crate::ui_thread::forbid("session::children");
    let folder = dir(app_home, cwd);
    let Some(logs) = index::logs(&folder)? else {
        return Ok(Vec::new());
    };
    let mut found: Vec<Child> = logs
        .iter()
        .filter(|(_, log)| {
            super::read_header(log).is_some_and(|h| h.parent.as_deref() == Some(parent))
        })
        .filter_map(|(_, log)| child(log))
        .collect();
    if !found.is_empty() {
        let labels = labels(&folder.join(format!("{parent}.jsonl")));
        for kid in &mut found {
            if let Some(label) = labels.get(&kid.id) {
                kid.label.clone_from(label);
            }
        }
    }
    found.sort_by(|a, b| (a.created_at, &a.id).cmp(&(b.created_at, &b.id)));
    Ok(found)
}

fn child(log: &Path) -> Option<Child> {
    let (header, events) = trace::read(log).ok()?;
    let depth = header.depth;
    let summary = summarize(header, &events);
    let mut kid = Child {
        id: summary.id,
        label: summary.title,
        depth,
        created_at: summary.created_at,
        last_time: summary.last_time,
        unfinished: summary.unfinished,
        ..Child::default()
    };
    last_turn(&events, &mut kid);
    Some(kid)
}

/// The step, last tool and outcome of the last turn of `events`, the
/// latest model and effort, and the tokens of every step.
fn last_turn(events: &[TraceEvent], kid: &mut Child) {
    for event in events {
        match event.record() {
            Record::TurnStart(_) => (kid.step, kid.tool, kid.outcome) = (0, None, None),
            Record::StepStart(start) => kid.step = start.step,
            Record::ToolCall(call) => kid.tool = Some(call.name),
            Record::TurnEnd(end) => kid.outcome = Some(end.outcome),
            Record::RequestContext(request) => {
                kid.model = request.model;
                kid.effort = request.effort.flatten();
            }
            Record::AssistantMessage(message) => {
                let u = &message.usage;
                kid.tokens += u.input + u.output + u.cache_read + u.cache_write;
            }
            _ => {}
        }
    }
}

/// Child session id to the `description` of the `subagent` call whose
/// result names it, from the parent's log (empty when it cannot be read).
fn labels(parent_log: &Path) -> HashMap<String, String> {
    let Ok((_, events)) = trace::read(parent_log) else {
        return HashMap::new();
    };
    let mut calls: Vec<(String, String)> = Vec::new();
    let mut labels = HashMap::new();
    for event in &events {
        match event.record() {
            Record::ToolCall(call) if call.name == "subagent" => {
                let args: Value = serde_json::from_str(&call.arguments).unwrap_or_default();
                let label = args["description"].as_str().unwrap_or_default();
                calls.push((call.call_id, label.to_string()));
            }
            Record::ToolResult(result) if event.kind == kind::TOOL_RESULT => {
                let text = serde_json::to_string(&result.message).unwrap_or_default();
                let call = calls.iter().find(|(id, _)| text.contains(id.as_str()));
                if let (Some((_, label)), Some(session)) = (call, session_in(&text)) {
                    labels.insert(session, label.clone());
                }
            }
            _ => {}
        }
    }
    labels
}

/// The first session id (`yyyymmdd-hhmmss-xxxx`) after `session` in `text`
/// (`"session": "<id>"` of a background start, `session <id>` of the stats
/// line).
fn session_in(text: &str) -> Option<String> {
    text.match_indices("session").find_map(|(at, _)| {
        let rest = &text[at + "session".len()..];
        let rest = rest.trim_start_matches(|c: char| !c.is_ascii_digit());
        let id: String = rest.chars().take(20).collect();
        index::is_id_name(&format!("{id}.jsonl")).then_some(id)
    })
}

#[cfg(test)]
#[path = "children_tests.rs"]
mod tests;
