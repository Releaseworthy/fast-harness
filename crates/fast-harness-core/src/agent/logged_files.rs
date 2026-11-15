//! The files a session log shows a read, write or edit of (PLAN.md D90): a
//! log from before `<id>.files.json` still lets `edit` take a file the
//! session changed, unless it was modified after that call's result.

use std::collections::HashMap;
use std::path::{Path, PathBuf};

use crate::trace::{kind, Record, TraceEvent};
use crate::types::Block;

/// `(path, last call, time of its result in ms)` of each file a successful
/// `read`, `write` or `edit` of the log named, resolved against `cwd`.
pub(super) fn logged_files(events: &[TraceEvent], cwd: &Path) -> Vec<(PathBuf, String, u64)> {
    let mut calls: HashMap<String, PathBuf> = HashMap::new();
    let mut last: HashMap<PathBuf, (String, u64)> = HashMap::new();
    for event in events {
        if event.kind != kind::TOOL_CALL && event.kind != kind::TOOL_RESULT {
            continue;
        }
        match event.record() {
            Record::ToolCall(call) => {
                if let Some(path) = path_of(&call.name, &call.arguments, cwd) {
                    calls.insert(call.call_id, path);
                }
            }
            Record::ToolResult(result) => {
                for block in result.message.content {
                    let Block::ToolResult {
                        call_id, is_error, ..
                    } = block
                    else {
                        continue;
                    };
                    match calls.remove(&call_id) {
                        Some(path) if !is_error => last.insert(path, (call_id, event.time)),
                        _ => None,
                    };
                }
            }
            _ => {}
        }
    }
    let files = last.into_iter();
    files.map(|(path, (call, at))| (path, call, at)).collect()
}

/// The path argument of a `read`, `write` or `edit` call, as the tools
/// resolve it (`tools::args::resolve`).
fn path_of(name: &str, arguments: &str, cwd: &Path) -> Option<PathBuf> {
    if !matches!(name, "read" | "write" | "edit") {
        return None;
    }
    let args: serde_json::Value = serde_json::from_str(arguments).ok()?;
    let path = Path::new(args["path"].as_str()?);
    let joined = match path.is_absolute() {
        true => path.to_path_buf(),
        false => cwd.join(path),
    };
    Some(joined.components().collect())
}
