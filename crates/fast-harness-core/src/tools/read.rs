//! `read`: a text file with numbered lines.

use serde_json::{json, Value};

use super::args::{cut_chars, looks_binary, object_schema, opt_positive, req_str, resolve};
use super::budget::ReadKey;
use super::{Effect, Tool, ToolCtx, ToolOutput};
use crate::llm::ToolSpec;

const DEFAULT_LIMIT: u64 = 2000;
const MAX_LINE_CHARS: usize = 2000;

pub(crate) struct ReadTool;

impl Tool for ReadTool {
    fn effect(&self) -> Effect {
        Effect::Read
    }

    fn spec(&self) -> ToolSpec {
        ToolSpec {
            name: "read".into(),
            description: "Read a UTF-8 text file. Lines come numbered as \"N: text\" (1-based). \
                Whole lines from offset fit one result budget; the last line names \
                the range and where to continue. A range already returned and unchanged answers \
                \"[unchanged since call ID]\". Directories and binary files are refused."
                .into(),
            schema: object_schema(
                json!({
                    "path": {"type": "string", "description": "File path, absolute or relative to the working directory."},
                    "offset": {"type": "integer", "minimum": 1, "description": "First line to return (1-based). Default 1."},
                    "limit": {"type": "integer", "minimum": 1, "description": "Maximum number of lines. Default 2000."}
                }),
                &["path"],
            ),
        }
    }

    fn run(&self, args: &Value, ctx: &ToolCtx<'_>) -> ToolOutput {
        match read(args, ctx) {
            Ok(text) => ToolOutput::ok(text),
            Err(message) => ToolOutput::err(message),
        }
    }
}

/// Room kept for the range line at the end.
const RANGE_ROOM: usize = 96;

fn read(args: &Value, ctx: &ToolCtx<'_>) -> Result<String, String> {
    let path_arg = req_str(args, "path")?;
    let offset = opt_positive(args, "offset", 1)?;
    let limit = opt_positive(args, "limit", DEFAULT_LIMIT)?;
    let path = resolve(ctx, path_arg);
    let meta = std::fs::metadata(&path).map_err(|err| format!("cannot read {path_arg}: {err}"))?;
    if meta.is_dir() {
        return Err(format!(
            "{path_arg} is a directory, not a file; use glob to list files"
        ));
    }
    let key = ReadKey {
        path: std::fs::canonicalize(&path).unwrap_or_else(|_| path.clone()),
        offset,
        limit,
        size: meta.len(),
        modified: meta.modified().ok(),
    };
    let session = ctx.session.filter(|_| !ctx.call_id.is_empty());
    if let Some(earlier) = session.and_then(|s| s.read_by(&key)) {
        return Ok(format!("[unchanged since call {earlier}]"));
    }
    let bytes = std::fs::read(&path).map_err(|err| format!("cannot read {path_arg}: {err}"))?;
    if looks_binary(&bytes) {
        return Err(format!(
            "{path_arg} looks like a binary file (NUL byte in the first 8 KB); not shown"
        ));
    }
    let text = String::from_utf8_lossy(&bytes);
    let room = ctx.budget.saturating_sub(RANGE_ROOM);
    let out = number_lines(&text, offset, limit, room)?;
    if let Some(session) = session {
        session.remember_read(key, ctx.call_id);
    }
    Ok(out)
}

/// Whole lines from `offset`, at most `limit` of them and `room` bytes (at
/// least one line), then `[lines a to b of n; continue with offset b+1]`
/// when lines remain.
fn number_lines(text: &str, offset: u64, limit: u64, room: usize) -> Result<String, String> {
    let lines: Vec<&str> = text.lines().collect();
    let total = lines.len() as u64;
    if total == 0 {
        return Ok("(empty file)".to_string());
    }
    if offset > total {
        return Err(format!(
            "offset {offset} is past the end of the file ({total} lines)"
        ));
    }
    let start = offset - 1;
    let last = total.min(start.saturating_add(limit));
    let mut out = String::new();
    let mut end = start;
    for line in &lines[start as usize..last as usize] {
        let entry = format!("{}: {}\n", end + 1, cut_chars(line, MAX_LINE_CHARS));
        if end > start && out.len() + entry.len() > room {
            break;
        }
        out.push_str(&entry);
        end += 1;
    }
    if end < total {
        out.push_str(&format!(
            "[lines {offset} to {end} of {total}; continue with offset {}]\n",
            end + 1
        ));
    }
    Ok(out)
}
