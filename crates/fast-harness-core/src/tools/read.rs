//! `read`: a text file with numbered lines, or an image (PLAN.md D70).

use serde_json::{json, Value};

use super::args::{object_schema, opt_positive, req_str, resolve};
use super::budget::{ReadKey, Seen, ToolSession};
use super::file_state::{self, file_key};
use super::read_diff;
use super::read_text::{self, Body, Page};
use super::{Effect, Tool, ToolCtx, ToolOutput};
use crate::llm::ToolSpec;

const DEFAULT_LIMIT: u64 = 2000;

pub(crate) struct ReadTool;

impl Tool for ReadTool {
    fn effect(&self) -> Effect {
        Effect::Read
    }

    fn spec(&self) -> ToolSpec {
        ToolSpec {
            name: "read".into(),
            description: "Read a text file (UTF-8 or UTF-16). Lines come numbered as \"N: text\" (1-based). \
                Without offset and limit only the file's head comes back (grep first, then \
                read line ranges); whole lines fit one result budget; the last line names the \
                range and where to continue; a line over 800 characters is cut (its note says how to \
                read on with `column`). Lines already returned and unchanged answer \
                \"[unchanged since call ID]\"; a range overlapping them returns only the new or changed lines. A png, jpeg, gif or webp file comes back as \
                an image with a line giving its size and token cost. Directories and other \
                binary files are refused."
                .into(),
            schema: object_schema(
                json!({
                    "path": {"type": "string", "description": "File path, absolute or relative to the working directory."},
                    "offset": {"type": "integer", "minimum": 1, "description": "First line to return (1-based). Default 1."},
                    "limit": {"type": "integer", "minimum": 1, "description": "Maximum number of lines. Default 2000."},
                    "column": {"type": "integer", "minimum": 1, "description": "Show each line from this character on (1-based), up to 2000 characters."}
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

/// Bytes of the budget kept free for the notes around the lines (range,
/// lines already shown): the generic cut must never take the range line.
const NOTE_ROOM: usize = 1024;

fn read(args: &Value, ctx: &ToolCtx<'_>) -> Result<ToolOutput, String> {
    let path_arg = req_str(args, "path")?;
    let ranged = !args["offset"].is_null() || !args["limit"].is_null();
    let offset = opt_positive(args, "offset", 1)?;
    let limit = opt_positive(args, "limit", DEFAULT_LIMIT)?;
    let column = opt_positive(args, "column", 0)?;
    let path = resolve(ctx, path_arg);
    let meta = std::fs::metadata(&path).map_err(|err| format!("cannot read {path_arg}: {err}"))?;
    if meta.is_dir() {
        return Err(format!(
            "{path_arg} is a directory, not a file; use glob to list files"
        ));
    }
    let (body, hash) = read_text::load(path_arg, &path, meta.len())?;
    let file = file_key(&path);
    let key = ReadKey {
        path: file.clone(),
        offset,
        limit,
        hash,
    };
    let page = page(offset, limit, ranged, column, ctx);
    let run = |offset, limit| {
        let page = Page {
            offset,
            limit,
            ..page
        };
        read_text::render(path_arg, &path, &body, page, ctx)
    };
    let dedupe = ctx
        .session
        .filter(|_| !ctx.call_id.is_empty() && ctx.dedupe_reads && column == 0);
    let output = match dedupe {
        None => run(offset, limit)?,
        Some(session) => match changed_lines(session, &key, &body, page, ctx.call_id) {
            Some(output) => output,
            None => read_new(session, key, run, ctx.call_id)?,
        },
    };
    if let Some(session) = ctx.session {
        let text = match &body {
            Body::Text(text) => Some(text.as_str()),
            _ => None,
        };
        file_state::record_read(session, file, hash, text, ctx.call_id);
    }
    Ok(output)
}

/// Only the changed lines of `key` when the file changed since this
/// session last saw it and an earlier read returned the whole range.
fn changed_lines(
    session: &ToolSession,
    key: &ReadKey,
    body: &Body,
    page: Page,
    call_id: &str,
) -> Option<ToolOutput> {
    let Body::Text(text) = body else {
        return None;
    };
    let before = session.files().get(&key.path)?;
    // After own edits the ranges are at the version read before them.
    let (old_hash, lines) = match before.base {
        Some(base) => base,
        None => (before.hash, before.lines?),
    };
    if old_hash == key.hash || session.read_by(key).is_some() {
        return None;
    }
    read_diff::changed(session, key, (&lines, text, old_hash), page.room, call_id)
}

/// Only the lines of `key` no earlier read of the unchanged file returned,
/// read by `run` and remembered as returned by `call_id`.
fn read_new(
    session: &ToolSession,
    key: ReadKey,
    run: impl Fn(u64, u64) -> Result<ToolOutput, String>,
    call_id: &str,
) -> Result<ToolOutput, String> {
    // Only the lines no earlier read of the unchanged file returned.
    let seen = session.read_seen(&key);
    let earlier = session
        .read_by(&key)
        .or_else(|| seen.covered.then(|| seen.before.join(", ")));
    if let Some(calls) = earlier {
        return unchanged_inside(&run, key.offset, &calls);
    }
    let (offset, limit) = (seen.offset, seen.end - seen.offset + 1);
    let output = match run(offset, limit) {
        // The lines left start past the end: all of the file's were shown.
        Err(err) if !seen.before.is_empty() && err.contains("past the end") => {
            return unchanged_inside(&run, key.offset, &seen.before.join(", "));
        }
        output => output?,
    };
    let end = returned_end(&output.text());
    session.remember_span(
        ReadKey {
            offset,
            limit,
            ..key.clone()
        },
        end,
        call_id,
    );
    Ok(with_seen_notes(output, &key, &seen))
}

pub(super) fn unchanged(calls: &str) -> ToolOutput {
    ToolOutput::ok(format!("[unchanged since call {calls}]"))
}

/// [`unchanged`] since `calls`, unless `offset` is past the end of the
/// file: that fails as it does for a file never read (a range read to the
/// end covers every later line number too).
fn unchanged_inside(
    run: &impl Fn(u64, u64) -> Result<ToolOutput, String>,
    offset: u64,
    calls: &str,
) -> Result<ToolOutput, String> {
    if offset > 1 {
        if let Err(err) = run(offset, 1) {
            if err.contains("past the end") {
                return Err(err);
            }
        }
    }
    Ok(unchanged(calls))
}

/// A read without a range is a head (PLAN.md D82): at most `read_head`.
/// The lines get the cap less [`NOTE_ROOM`] (at most a quarter of it).
fn page(offset: u64, limit: u64, ranged: bool, column: u64, ctx: &ToolCtx<'_>) -> Page {
    let head = !ranged && ctx.read_head > 0;
    let cap = match head {
        true => ctx.budget.min(ctx.read_head),
        false => ctx.budget,
    };
    Page {
        offset,
        limit,
        room: cap.saturating_sub(NOTE_ROOM.min(cap / 4)),
        head,
        column,
    }
}

/// `output` with a line before it naming the lines of `key` earlier calls
/// already returned at its start, and one after it for those at its end.
fn with_seen_notes(output: ToolOutput, key: &ReadKey, seen: &Seen) -> ToolOutput {
    let trimmed = !seen.before.is_empty() || !seen.after.is_empty();
    if !trimmed || output.images().next().is_some() {
        return output;
    }
    let note = |from: u64, to: String, calls: &[String]| match calls.is_empty() {
        true => String::new(),
        false => format!(
            "[already shown, unchanged: lines {from}-{to} (call {})]\n",
            calls.join(", ")
        ),
    };
    let before = note(key.offset, (seen.offset - 1).to_string(), &seen.before);
    let wanted_end = key.offset.saturating_add(key.limit.saturating_sub(1));
    let to = match seen.to_end {
        true => "end".to_string(),
        false => wanted_end.to_string(),
    };
    let after = note(seen.end + 1, to, &seen.after);
    let text = output.text();
    let (lines, range) = split_range_note(&text);
    let mut text = before + lines;
    if !text.ends_with('\n') {
        text.push('\n');
    }
    text += &after;
    text += &whole_range(range, lines, key, seen);
    ToolOutput::ok(text)
}

/// `text` and its last line when that is the range note `[lines a to b…]`.
fn split_range_note(text: &str) -> (&str, Option<&str>) {
    let body = text.trim_end_matches('\n');
    let (lines, last) = match body.rfind('\n') {
        Some(at) => (&body[..at + 1], &body[at + 1..]),
        None => ("", body),
    };
    match last.starts_with("[lines ") {
        true => (lines, Some(last)),
        false => (text, None),
    }
}

/// The last line of a read trimmed by `seen`: the range note `range` of the
/// new `lines` rewritten for the whole request `key` (PLAN.md D87).
fn whole_range(range: Option<&str>, lines: &str, key: &ReadKey, seen: &Seen) -> String {
    let from = key.offset;
    let Some(note) = range else {
        // The new lines reached the end of the file.
        let last = lines.lines().rev().find_map(|l| l.split_once(": "));
        return match last.and_then(|(n, _)| n.parse::<u64>().ok()) {
            Some(n) => format!("[lines {from} to {n} of {n}]\n"),
            None => String::new(),
        };
    };
    let after_from = note.find(" to ").map_or(note.len(), |at| at + 4);
    let rest = &note[after_from..];
    let digits = rest
        .find(|c: char| !c.is_ascii_digit())
        .unwrap_or(rest.len());
    let (Ok(shown_end), tail) = (rest[..digits].parse::<u64>(), &rest[digits..]) else {
        return format!("{note}\n");
    };
    if shown_end < seen.end {
        // Cut by the budget: the read goes on after the new lines.
        return format!("[lines {from} to {shown_end}{tail}\n");
    }
    let total = tail
        .strip_prefix(" of ")
        .and_then(|t| t.split([';', ']']).next())
        .and_then(|t| t.parse::<u64>().ok());
    let wanted_end = key.offset.saturating_add(key.limit.saturating_sub(1));
    match (seen.to_end, total) {
        (true, Some(total)) => format!("[lines {from} to {total} of {total}]\n"),
        (true, None) => format!("[lines {from} to the end of the file]\n"),
        (false, Some(total)) if wanted_end >= total => {
            format!("[lines {from} to {total} of {total}]\n")
        }
        (false, _) => {
            let tail = tail.replace(
                &format!("offset {}", shown_end + 1),
                &format!("offset {}", wanted_end.saturating_add(1)),
            );
            format!("[lines {from} to {wanted_end}{tail}\n")
        }
    }
}

/// The last line `text` holds: the range note's end when lines remain, else
/// `u64::MAX` (the read reached the end of the file: every later line too).
fn returned_end(text: &str) -> u64 {
    let note = text.lines().rev().find_map(|l| l.strip_prefix("[lines "));
    let end = note
        .and_then(|rest| rest.split_once(" to "))
        .map(|(_, rest)| rest.split([' ', ';', ']']).next().unwrap_or_default())
        .and_then(|number| number.parse().ok());
    end.unwrap_or(u64::MAX)
}
