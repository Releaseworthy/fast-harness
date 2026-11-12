//! Nested AGENTS.md (PLAN.md D43). The AGENTS.md files from the repository
//! root down to the working directory are in the frozen system prompt; a
//! folder below the working directory with its own AGENTS.md has it
//! appended (at most [`NESTED_BYTES`]) to the first tool result whose call
//! touches a path inside it, once per session.
//!
//! The text joins the result after the output budget (it is not counted in
//! it) and before the result is first sent or traced. Each block starts
//! with the line `[nested AGENTS.md <path>]`, so the loaded set is rebuilt
//! from the conversation: a resumed session and a compacted one (whose
//! summary lists these lines) never append a file twice.

use std::path::{Component, Path, PathBuf};

use serde_json::Value;

use crate::tools::skills::{marked_lines, MARK as SKILL_MARK};
use crate::tools::ToolOutput;
use crate::types::Message;

/// Starts the first line of a nested AGENTS.md block.
pub const MARK: &str = "[nested AGENTS.md ";
/// Most bytes of one nested AGENTS.md text.
pub const NESTED_BYTES: usize = 6 * 1024;

/// The nested AGENTS.md files the conversation holds, by path.
pub fn loaded(messages: &[Message]) -> Vec<String> {
    marked_lines(messages, MARK)
        .into_iter()
        .filter_map(|line| line.strip_prefix(MARK)?.strip_suffix(']'))
        .map(str::to_string)
        .collect()
}

/// The skill and nested AGENTS.md mark lines of `messages`, for the
/// compaction summary.
pub fn instruction_lines(messages: &[Message]) -> Vec<String> {
    let skills = marked_lines(messages, SKILL_MARK);
    let nested = marked_lines(messages, MARK);
    (skills.into_iter().chain(nested))
        .map(str::to_string)
        .collect()
}

/// Appends to each output the nested AGENTS.md files its call (`args`, the
/// raw arguments) reaches for the first time in the session.
pub fn attach(messages: &[Message], cwd: &Path, args: &[&str], outputs: &mut [ToolOutput]) {
    let mut known: Option<Vec<String>> = None;
    for (raw, output) in args.iter().zip(outputs.iter_mut()) {
        let Some(path) = touched(cwd, raw) else {
            continue;
        };
        for file in candidates(cwd, &path) {
            let key = file.display().to_string();
            let known = known.get_or_insert_with(|| loaded(messages));
            if known.contains(&key) {
                continue;
            }
            let Ok(text) = std::fs::read_to_string(&file) else {
                continue;
            };
            output.content.push_str(&block(&key, &text));
            known.push(key);
        }
    }
}

/// The `path` argument (read, write, edit, grep, glob and any tool with
/// one) resolved against `cwd`.
fn touched(cwd: &Path, raw: &str) -> Option<PathBuf> {
    let args: Value = serde_json::from_str(raw).ok()?;
    let path = args.get("path")?.as_str()?.trim();
    if path.is_empty() {
        return None;
    }
    Some(cwd.join(path))
}

/// The AGENTS.md files of the folders strictly below `cwd` on the way to
/// `path` (`path` itself when it is a folder), outermost first.
fn candidates(cwd: &Path, path: &Path) -> Vec<PathBuf> {
    let Ok(rel) = path.strip_prefix(cwd) else {
        return Vec::new();
    };
    let parts: Vec<Component> = rel.components().collect();
    if parts
        .iter()
        .any(|part| !matches!(part, Component::Normal(_)))
    {
        return Vec::new();
    }
    let mut dir = cwd.to_path_buf();
    let mut out = Vec::new();
    for (index, part) in parts.iter().enumerate() {
        dir.push(part);
        if index + 1 == parts.len() && !dir.is_dir() {
            break;
        }
        let file = dir.join("AGENTS.md");
        if file.is_file() {
            out.push(file);
        }
    }
    out
}

fn block(key: &str, text: &str) -> String {
    let text = text.trim();
    let mut out = format!("\n\n{MARK}{key}]\n");
    if text.len() <= NESTED_BYTES {
        out.push_str(text);
        return out;
    }
    let mut end = NESTED_BYTES;
    while !text.is_char_boundary(end) {
        end -= 1;
    }
    out.push_str(&text[..end]);
    out.push_str("\n[cut at 6 KB; read the file for the rest]");
    out
}
