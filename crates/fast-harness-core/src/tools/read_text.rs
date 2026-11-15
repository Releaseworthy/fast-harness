//! `read`'s file loading and line numbering: the encoding is decoded (a
//! BOM is never shown), a long line is cut with its length and the way to
//! read on (`column`).

use std::fs::File;
use std::io::{BufReader, Read, Seek, SeekFrom};
use std::path::Path;

use super::file_state::{hash_of, meta_hash, HASHED_MAX};
use super::read_stream;
use super::text_codec::{self, Encoding};
use super::{ToolCtx, ToolOutput};
use crate::media;

/// A longer line is cut (minified code, logs): `column` reads the rest.
pub(crate) const MAX_LINE_CHARS: usize = 800;
/// Characters of a line shown from `column` on, and the byte cap of them.
const COLUMN_CHARS: usize = 2000;
const COLUMN_BYTES: usize = 4000;
/// Bytes read first to tell an image, a binary and a text file apart.
const HEAD_BYTES: u64 = 8 * 1024;

/// A loaded file: an image, a whole decoded text, or a large text file
/// streamed from disk later (after `skip` BOM bytes).
pub(super) enum Body {
    Image(Vec<u8>),
    Text(String),
    Large { skip: u64, size: u64 },
}

/// What a read returns: lines from `offset`, at most `limit` and `room`
/// bytes; `head` when no range was asked; `column` cuts every line.
#[derive(Clone, Copy)]
pub(super) struct Page {
    pub offset: u64,
    pub limit: u64,
    pub room: usize,
    pub head: bool,
    pub column: u64,
}

/// The file's body and content hash. An image over the cap, a binary file
/// and a UTF-16 file over 4 MB are refused.
pub(super) fn load(path_arg: &str, path: &Path, size: u64) -> Result<(Body, u64), String> {
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
        let hash = hash_of(path, &bytes);
        return Ok((Body::Image(bytes), hash));
    }
    let encoding = text_codec::detect(&bytes).ok_or_else(|| {
        format!("{path_arg} looks like a binary file (NUL byte in the first 8 KB); not shown")
    })?;
    if size > HASHED_MAX {
        if encoding.is_utf16() {
            return Err(format!("{path_arg} is UTF-16 and over 4 MB; not shown"));
        }
        let skip = if encoding == Encoding::Utf8Bom { 3 } else { 0 };
        return Ok((Body::Large { skip, size }, meta_hash(path)));
    }
    file.take(HASHED_MAX)
        .read_to_end(&mut bytes)
        .map_err(fail)?;
    let hash = hash_of(path, &bytes);
    Ok((Body::Text(text_codec::decode_lossy(&bytes, encoding)), hash))
}

/// `page` of `body` as the tool's output.
pub(super) fn render(
    path_arg: &str,
    path: &Path,
    body: &Body,
    page: Page,
    ctx: &ToolCtx<'_>,
) -> Result<ToolOutput, String> {
    match body {
        Body::Image(bytes) => image(path_arg, bytes, ctx),
        Body::Text(text) => number_lines(text, page).map(ToolOutput::ok),
        Body::Large { skip, size } => {
            let fail = |err: std::io::Error| format!("cannot read {path_arg}: {err}");
            let mut file = File::open(path).map_err(fail)?;
            file.seek(SeekFrom::Start(*skip)).map_err(fail)?;
            let keep = keep_chars(page.column);
            let show = |n: u64, line: &str, whole: bool| show_line(n, line, page.column, whole);
            let stream = (page.offset, page.limit, page.room, keep);
            read_stream::prefix_lines(BufReader::new(file), stream, *size, &show)
                .map(ToolOutput::ok)
        }
    }
}

/// Characters of a line [`show_line`] needs for `column` (plus one to
/// see that more follow).
fn keep_chars(column: u64) -> usize {
    match column {
        0 => MAX_LINE_CHARS + 1,
        c => (c as usize).saturating_add(COLUMN_CHARS),
    }
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

/// Line `n` numbered: whole, cut at [`MAX_LINE_CHARS`] with its length and
/// how to read on, or from `column` on. `whole` is false when the line was
/// read only in part (its length is then unknown).
pub(super) fn show_line(n: u64, line: &str, column: u64, whole: bool) -> String {
    let total = line.chars().count();
    let more = |shown: usize| match whole {
        true => format!("{shown} of {total} chars"),
        false => format!("{shown} chars, more follow"),
    };
    if column == 0 {
        return match line.char_indices().nth(MAX_LINE_CHARS) {
            None => format!("{n}: {line}\n"),
            Some((end, _)) => format!(
                "{n}: {}... [line cut at {}; read on with offset {n}, limit 1, column {}]\n",
                &line[..end],
                more(MAX_LINE_CHARS),
                MAX_LINE_CHARS + 1
            ),
        };
    }
    let first = column as usize - 1;
    let Some((start, _)) = line.char_indices().nth(first) else {
        return format!("{n}: [the line has {total} chars; column {column} is past its end]\n");
    };
    let rest = &line[start..];
    let mut end = 0;
    for (count, (index, ch)) in rest.char_indices().enumerate() {
        if count == COLUMN_CHARS || index + ch.len_utf8() > COLUMN_BYTES {
            break;
        }
        end = index + ch.len_utf8();
    }
    let last = first + rest[..end].chars().count();
    match end < rest.len() {
        true => format!(
            "{n}: {} [columns {column}-{last} of {}; read on with column {}]\n",
            &rest[..end],
            if whole {
                total.to_string()
            } else {
                "more".to_string()
            },
            last + 1
        ),
        false => format!("{n}: {rest} [columns {column}-{last} of {total}]\n"),
    }
}

/// The end of a head's range note (PLAN.md D82).
const HEAD_HINT: &str = "; first grep it, then read the lines you need with offset and limit";

/// Whole lines of `page` (at least one), then `[lines a to b of n;
/// continue with offset b+1]` when lines remain; a head adds how to read
/// less.
pub(super) fn number_lines(text: &str, page: Page) -> Result<String, String> {
    let lines: Vec<&str> = text.lines().collect();
    let (offset, total) = (page.offset, lines.len() as u64);
    if total == 0 {
        return Ok("(empty file)".to_string());
    }
    if offset > total {
        return Err(format!(
            "offset {offset} is past the end of the file ({total} lines)"
        ));
    }
    let start = offset - 1;
    let last = total.min(start.saturating_add(page.limit));
    let mut out = String::new();
    let mut end = start;
    for line in &lines[start as usize..last as usize] {
        let entry = show_line(end + 1, line, page.column, true);
        if end > start && out.len() + entry.len() > page.room {
            break;
        }
        out.push_str(&entry);
        end += 1;
    }
    if end < total {
        out.push_str(&format!(
            "[lines {offset} to {end} of {total}; continue with offset {}{}]\n",
            end + 1,
            if page.head { HEAD_HINT } else { "" }
        ));
    }
    Ok(out)
}
