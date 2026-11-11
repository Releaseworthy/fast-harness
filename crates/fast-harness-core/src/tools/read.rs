//! `read`: a text file with numbered lines.

use serde_json::{json, Value};

use super::args::{cut_chars, looks_binary, object_schema, opt_positive, req_str, resolve};
use super::{Tool, ToolCtx, ToolOutput};
use crate::llm::ToolSpec;

const DEFAULT_LIMIT: u64 = 2000;
const MAX_LINE_CHARS: usize = 2000;

pub(crate) struct ReadTool;

impl Tool for ReadTool {
    fn spec(&self) -> ToolSpec {
        ToolSpec {
            name: "read".into(),
            description: "Read a UTF-8 text file. Lines come numbered as \"N: text\" (1-based). \
                Use offset and limit to page through large files; the output says when more \
                lines remain. Directories and binary files are refused."
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
    let bytes = std::fs::read(&path).map_err(|err| format!("cannot read {path_arg}: {err}"))?;
    if looks_binary(&bytes) {
        return Err(format!(
            "{path_arg} looks like a binary file (NUL byte in the first 8 KB); not shown"
        ));
    }
    let text = String::from_utf8_lossy(&bytes);
    number_lines(&text, offset, limit)
}

fn number_lines(text: &str, offset: u64, limit: u64) -> Result<String, String> {
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
    let end = total.min(start.saturating_add(limit));
    let mut out = String::new();
    for (index, line) in lines[start as usize..end as usize].iter().enumerate() {
        let number = start + index as u64 + 1;
        out.push_str(&format!("{number}: {}\n", cut_chars(line, MAX_LINE_CHARS)));
    }
    if end < total {
        out.push_str(&format!(
            "[showing lines {offset}-{end} of {total}; {} more lines, continue with offset={}]\n",
            total - end,
            end + 1
        ));
    }
    Ok(out)
}
