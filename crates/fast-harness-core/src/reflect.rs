//! `fast-harness reflect` and `/reflect` (PLAN.md D50): one subagent on the
//! cheapest configured model reads the stats and a digest of a session's
//! trace (tool names, errors, sizes and counts: never the transcript) and
//! proposes edits to memory notes, AGENTS.md lines, skills and tool
//! overrides as a unified diff. `selfImprove` decides what is applied:
//! `ask` applies nothing until asked, `auto` applies the memory edits only,
//! `off` turns reflect off. Nothing runs by itself at turn end.

pub mod patch;

use std::collections::BTreeMap;
use std::fmt::Write;

use crate::config::Config;
use crate::llm::custom;
use crate::trace::record::ToolCall;
use crate::trace::{Record, TraceEvent};
use crate::types::{Block, Message};

/// The system prompt of the reflect subagent.
pub const SYSTEM: &str = "You review how a coding agent harness session went and propose small, \
durable improvements. You see numbers and a digest of the trace, not the conversation. Answer \
with one unified diff (```diff fenced, `--- a/<path>` and `+++ b/<path>` headers, `@@` hunks, \
`--- /dev/null` for a new file) and at most five short lines of reasons. Paths: `memory/<file>` \
is the workspace memory folder (MEMORY.md, its index of one line per note, at most 2 KB, and \
notes/*.md); other paths are relative to the workspace (AGENTS.md, .fast-harness/skills/<name>/\
SKILL.md, .fast-harness/tools/*.json). Change only what the signals justify; an empty diff is fine.";

/// The longest error line kept in a digest.
pub const ERROR_CHARS: usize = 120;

/// `selfImprove`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Mode {
    Ask,
    Auto,
    Off,
}

impl Mode {
    /// `"auto"` and `"off"`; anything else is `ask` (the default).
    pub fn of(config: &Config) -> Mode {
        match config.self_improve.trim() {
            "auto" => Mode::Auto,
            "off" => Mode::Off,
            _ => Mode::Ask,
        }
    }
}

/// `reflect.model`, else the cheapest model with a price in the provider
/// files (input plus output per million tokens), else the session model.
pub fn pick_model(config: &Config) -> String {
    if let Some(model) = config
        .reflect
        .model
        .as_ref()
        .filter(|m| !m.trim().is_empty())
    {
        return model.clone();
    }
    let mut best: Option<(f64, String)> = None;
    for provider in custom::providers() {
        for model in &provider.models {
            let Some(price) = model.price else { continue };
            let cost = price.input + price.output;
            let name = match provider.id.as_str() {
                "anthropic" => model.id.clone(),
                id => format!("{id}/{}", model.id),
            };
            if best.as_ref().is_none_or(|(known, _)| cost < *known) {
                best = Some((cost, name));
            }
        }
    }
    best.map_or_else(|| config.model.clone(), |(_, name)| name)
}

/// One turn of a digest.
#[derive(Default)]
struct TurnDigest {
    steps: u64,
    /// name -> (calls, result bytes, errors)
    tools: BTreeMap<String, (u64, u64, u64)>,
    errors: Vec<String>,
}

/// The digest of a session's trace: per turn the step count, each tool's
/// calls, result bytes and errors, and the first line of each error result
/// (at most [`ERROR_CHARS`]). No user, assistant or tool result text.
pub fn digest(events: &[TraceEvent]) -> String {
    let mut turns: BTreeMap<u64, TurnDigest> = BTreeMap::new();
    let mut names: BTreeMap<String, String> = BTreeMap::new();
    let mut compactions = 0;
    for event in events {
        match event.record() {
            Record::StepStart(start) => turns.entry(u64::from(start.turn)).or_default().steps += 1,
            Record::ToolCall(call) => add_call(&mut turns, &mut names, &call),
            Record::ToolResult(result) => {
                let digest = turns.entry(u64::from(result.turn)).or_default();
                add_result(digest, &result.message, &names);
            }
            Record::Compaction(_) => compactions += 1,
            _ => {}
        }
    }
    let mut out = String::new();
    for (turn, digest) in &turns {
        turn_lines(&mut out, *turn, digest);
    }
    let _ = writeln!(out, "compactions {compactions}");
    out
}

/// Counts one `tool/call` in its turn and remembers the call id's tool name.
fn add_call(
    turns: &mut BTreeMap<u64, TurnDigest>,
    names: &mut BTreeMap<String, String>,
    call: &ToolCall,
) {
    let tools = &mut turns.entry(u64::from(call.turn)).or_default().tools;
    tools.entry(call.name.clone()).or_default().0 += 1;
    names.insert(call.call_id.clone(), call.name.clone());
}

fn add_result(digest: &mut TurnDigest, message: &Message, names: &BTreeMap<String, String>) {
    for block in &message.content {
        let Block::ToolResult {
            call_id,
            content,
            is_error,
        } = block
        else {
            continue;
        };
        let name = names.get(call_id).map_or("?", String::as_str);
        let entry = digest.tools.entry(name.to_string()).or_default();
        entry.1 += content.len() as u64;
        if *is_error {
            entry.2 += 1;
            let line: String = content
                .lines()
                .next()
                .unwrap_or("")
                .chars()
                .take(ERROR_CHARS)
                .collect();
            digest.errors.push(format!("{name}: {line}"));
        }
    }
}

fn turn_lines(out: &mut String, turn: u64, digest: &TurnDigest) {
    let tools: Vec<String> = digest
        .tools
        .iter()
        .map(|(name, (calls, bytes, errors))| match errors {
            0 => format!("{name} x{calls} ({bytes} B)"),
            e => format!("{name} x{calls} ({bytes} B, {e} errors)"),
        })
        .collect();
    let _ = writeln!(
        out,
        "turn {turn}: {} steps; {}",
        digest.steps,
        tools.join(", ")
    );
    for error in &digest.errors {
        let _ = writeln!(out, "  error {error}");
    }
}

/// The user message of the reflect subagent.
pub fn prompt(stats: &str, digest: &str, memory_index: &str) -> String {
    let index = if memory_index.trim().is_empty() {
        "(empty)"
    } else {
        memory_index.trim()
    };
    format!(
        "Session numbers:\n{stats}\nTrace digest:\n{digest}\nmemory/MEMORY.md now:\n{index}\n\n\
         Propose the diff."
    )
}

/// The diff of an answer: the first ```diff (or ```patch) fence, else the
/// text from the first `--- ` line; empty when there is none.
pub fn extract_diff(answer: &str) -> String {
    for fence in ["```diff", "```patch"] {
        if let Some(start) = answer.find(fence) {
            let body = &answer[start + fence.len()..];
            let body = body.split_once('\n').map_or("", |(_, rest)| rest);
            let end = body.find("```").unwrap_or(body.len());
            return body[..end].to_string();
        }
    }
    match answer.find("--- ") {
        Some(start) if start == 0 || answer[..start].ends_with('\n') => answer[start..].to_string(),
        _ => String::new(),
    }
}
