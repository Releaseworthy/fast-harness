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
                Without offset and limit only the file's head comes back (grep first, then \
                read line ranges); whole lines fit one result budget; the last line names the \
                range and where to continue. Lines already returned and unchanged answer \
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
const RANGE_ROOM: usize = 128;
/// The end of a head's range note (PLAN.md D82).
const HEAD_HINT: &str = "; first grep it, then read the lines you need with offset and limit";

fn read(args: &Value, ctx: &ToolCtx<'_>) -> Result<ToolOutput, String> {
    let path_arg = req_str(args, "path")?;
    let ranged = !args["offset"].is_null() || !args["limit"].is_null();
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
    let session = ctx
        .session
        .filter(|_| !ctx.call_id.is_empty() && ctx.dedupe_reads);
    if let Some(earlier) = session.and_then(|s| s.read_by(&key)) {
        return Ok(ToolOutput::ok(format!("[unchanged since call {earlier}]")));
    }
    // A read without a range is a head (PLAN.md D82): at most `read_head`.
    let head = !ranged && ctx.read_head > 0;
    let cap = match head {
        true => ctx.budget.min(ctx.read_head),
        false => ctx.budget,
    };
    let shape = Shape {
        offset,
        limit,
        cap,
        head,
    };
    let output = read_file(path_arg, &path, shape, meta.len(), ctx)?;
    if let Some(session) = session {
        let end = returned_end(&output.text(), offset, limit);
        session.remember_span(key, end, ctx.call_id);
    }
    Ok(output)
}

/// What a read asks for: first line, line count, byte cap and whether it
/// is a head (no range given: the range note says how to read more).
#[derive(Clone, Copy)]
struct Shape {
    offset: u64,
    limit: u64,
    cap: usize,
    head: bool,
}

/// The last line `text` holds: the range note's end when the result was
/// cut, else the end of the request.
fn returned_end(text: &str, offset: u64, limit: u64) -> u64 {
    let note = text.lines().rev().find_map(|l| l.strip_prefix("[lines "));
    let end = note
        .and_then(|rest| rest.split_once(" to "))
        .map(|(_, rest)| rest.split([' ', ';', ']']).next().unwrap_or_default())
        .and_then(|number| number.parse().ok());
    end.unwrap_or_else(|| offset.saturating_add(limit.saturating_sub(1)))
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
    shape: Shape,
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
    let room = shape.cap.saturating_sub(RANGE_ROOM);
    let (offset, limit) = (shape.offset, shape.limit);
    if size > WHOLE_TEXT_MAX {
        let reader = BufReader::new(Cursor::new(bytes).chain(file));
        let stream = (offset, limit, room, MAX_LINE_CHARS);
        return read_stream::prefix_lines(reader, stream, size).map(ToolOutput::ok);
    }
    file.take(WHOLE_TEXT_MAX)
        .read_to_end(&mut bytes)
        .map_err(fail)?;
    let text = String::from_utf8_lossy(&bytes);
    number_lines(&text, (offset, limit, room), shape.head).map(ToolOutput::ok)
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
/// when lines remain; a head (no range asked) adds how to read less.
fn number_lines(
    text: &str,
    (offset, limit, room): (u64, u64, usize),
    head: bool,
) -> Result<String, String> {
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
            "[lines {offset} to {end} of {total}; continue with offset {}{}]\n",
            end + 1,
            if head { HEAD_HINT } else { "" }
        ));
    }
    Ok(out)
}
