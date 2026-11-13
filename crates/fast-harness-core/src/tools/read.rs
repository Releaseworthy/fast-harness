//! `read`: a text file with numbered lines, or an image (PLAN.md D70).

use std::fs::File;
use std::io::{BufReader, Cursor, Read};
use std::path::Path;

use serde_json::{json, Value};

use super::args::{cut_chars, looks_binary, object_schema, opt_positive, req_str, resolve};
use super::budget::ReadKey;
use super::read_stream;
use super::{Effect, Tool, ToolCtx, ToolOutput};
use crate::llm::ToolSpec;
use crate::media;

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
                \"[unchanged since call ID]\". A png, jpeg, gif or webp file comes back as \
                an image with a line giving its size and token cost. Directories and other \
                binary files are refused."
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
            Ok(output) => output,
            Err(message) => ToolOutput::err(message),
        }
    }
}

/// Room kept for the range line at the end.
const RANGE_ROOM: usize = 96;

fn read(args: &Value, ctx: &ToolCtx<'_>) -> Result<ToolOutput, String> {
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
        return Ok(ToolOutput::ok(format!("[unchanged since call {earlier}]")));
    }
    let output = read_file(path_arg, &path, (offset, limit), meta.len(), ctx)?;
    if let Some(session) = session {
        session.remember_read(key, ctx.call_id);
    }
    Ok(output)
}

/// Bytes read first to tell an image, a binary and a text file apart.
const HEAD_BYTES: u64 = 8 * 1024;
/// A text file up to this size is read whole (its line count is exact); a
/// larger one is streamed only as far as the lines returned (`read_stream`).
const WHOLE_TEXT_MAX: u64 = 4 * 1024 * 1024;

/// The file's sniffed head decides how much more of it is read: an image
/// over the cap and a binary file are refused from the head and the size.
fn read_file(
    path_arg: &str,
    path: &Path,
    (offset, limit): (u64, u64),
    size: u64,
    ctx: &ToolCtx<'_>,
) -> Result<ToolOutput, String> {
    let fail = |err: std::io::Error| format!("cannot read {path_arg}: {err}");
    let mut file = File::open(path).map_err(fail)?;
    let mut bytes = Vec::new();
    (&mut file)
        .take(HEAD_BYTES)
        .read_to_end(&mut bytes)
        .map_err(fail)?;
    if media::sniff::media_type(&bytes).is_some() {
        if size > media::MAX_BYTES {
            let err = media::MediaError::TooLarge { bytes: size };
            return Err(format!("{path_arg} is an image; not read: {err}"));
        }
        let rest = media::MAX_BYTES + 1 - bytes.len() as u64;
        file.take(rest).read_to_end(&mut bytes).map_err(fail)?;
        return image(path_arg, &bytes, ctx);
    }
    if looks_binary(&bytes) {
        return Err(format!(
            "{path_arg} looks like a binary file (NUL byte in the first 8 KB); not shown"
        ));
    }
    let room = ctx.budget.saturating_sub(RANGE_ROOM);
    if size > WHOLE_TEXT_MAX {
        let reader = BufReader::new(Cursor::new(bytes).chain(file));
        let shape = (offset, limit, room, MAX_LINE_CHARS);
        return read_stream::prefix_lines(reader, shape, size).map(ToolOutput::ok);
    }
    file.take(WHOLE_TEXT_MAX)
        .read_to_end(&mut bytes)
        .map_err(fail)?;
    let text = String::from_utf8_lossy(&bytes);
    number_lines(&text, offset, limit, room).map(ToolOutput::ok)
}

/// An image file: kept in the session's media folder and returned as an
/// image part after a line with its size and token cost.
fn image(path_arg: &str, bytes: &[u8], ctx: &ToolCtx<'_>) -> Result<ToolOutput, String> {
    let dir = ctx.session.and_then(|s| s.media_dir()).ok_or_else(|| {
        format!("{path_arg} is an image; not shown: there is no session folder to keep it in")
    })?;
    let image = media::attach_bytes(dir, bytes, None)
        .map_err(|err| format!("{path_arg} is an image; not shown: {err}"))?;
    let line = format!("{path_arg}: image, {}", media::label(&image));
    Ok(ToolOutput::image(line, image))
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
