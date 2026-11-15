//! `read` of a range the session already returned whose file changed since
//! (someone else's change; the agent's own edits move its ranges along):
//! only the changed lines come back, numbered, with a note.

use super::budget::{ReadKey, ToolSession};
use super::file_state;
use super::line_diff::{self, Hunk};
use super::read_text::show_line;
use super::ToolOutput;

/// The changed lines of `key`'s range in `text` (now hashed `key.hash`)
/// against the version with line hashes `old` (hashed `old_hash`); `None`
/// when no earlier read returned the whole range or they do not fit `room`.
pub(super) fn changed(
    session: &ToolSession,
    key: &ReadKey,
    (old, text, old_hash): (&[u64], &str, u64),
    room: usize,
    call_id: &str,
) -> Option<ToolOutput> {
    let lines: Vec<&str> = text.lines().collect();
    let total = lines.len() as u64;
    if key.offset > total {
        return None;
    }
    let hunks = line_diff::hunks(old, &line_diff::line_hashes(text));
    let end = total.min(key.offset.saturating_add(key.limit - 1));
    let old_start = line_diff::new_to_old(&hunks, key.offset);
    let old_end = line_diff::new_to_old(&hunks, end + 1)
        .saturating_sub(1)
        .max(old_start);
    let old_key = ReadKey {
        offset: old_start,
        limit: old_end - old_start + 1,
        hash: old_hash,
        ..key.clone()
    };
    let calls = session.read_by(&old_key)?;
    let (body, shown) = render(&lines, &hunks, (key.offset, end))?;
    let text = note(&calls, (key.offset, end), old_start, &body);
    if text.len() > room {
        return None;
    }
    file_state::move_spans(
        session,
        &key.path,
        (old_hash, key.hash),
        &hunks,
        Some(&shown),
        call_id,
    );
    session.remember_span(
        ReadKey {
            limit: end - key.offset + 1,
            ..key.clone()
        },
        end,
        call_id,
    );
    Some(ToolOutput::ok(text))
}

/// The result: a note naming `calls` and the range `from`-`end` (that was
/// at `old_start`), then `body` (the changed lines; none: unchanged).
fn note(calls: &str, (from, end): (u64, u64), old_start: u64, body: &str) -> String {
    let header = match body.is_empty() {
        true => format!("[unchanged since call {calls}"),
        false => format!(
            "[changed since call {calls}: only the changed lines of {from}-{end} follow, the \
             others are as that call returned"
        ),
    };
    let moved = match from as i64 - old_start as i64 {
        0 => String::new(),
        d => format!("; they moved by {d:+} lines"),
    };
    match body.is_empty() {
        true => format!("{header}{moved}]"),
        false => format!("{header}{moved}]\n{body}"),
    }
}

/// The lines of the hunks in `range` (1-based, inclusive) and the hunks
/// shown whole; `None` when a hunk is too long to list.
fn render(lines: &[&str], hunks: &[Hunk], (from, to): (u64, u64)) -> Option<(String, Vec<Hunk>)> {
    let (mut out, mut shown) = (String::new(), Vec::new());
    for hunk in hunks {
        let first = hunk.new as u64 + 1;
        let last = (hunk.new + hunk.new_len) as u64;
        // A deletion sits between lines `first - 1` and `first`.
        let inside = first <= to.saturating_add(1) && last.max(first) >= from;
        if !inside {
            continue;
        }
        if first >= from && last <= to {
            shown.push(*hunk);
        }
        for n in first.max(from)..=last.min(to) {
            out.push_str(&show_line(n, lines.get(n as usize - 1)?, 0, true));
        }
        if hunk.old_len > hunk.new_len || (hunk.new_len == 0 && hunk.old_len > 0) {
            let removed = hunk.old_len - hunk.new_len;
            let at = last.max(first - 1);
            out.push_str(&format!("[{removed} line(s) removed after line {at}]\n"));
        }
    }
    Some((out, shown))
}
