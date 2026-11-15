//! The output diet (PLAN.md D37): every tool result fits one byte budget
//! before it is first sent. A longer result keeps a cut of its text and
//! sends the whole text to `<session>.out/<callId>.txt`, named by a note at
//! the end of the result.

use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, MutexGuard};

use super::file_state::FileStates;
use super::{Jobs, ToolCtx, ToolOutput, UndoLog};
use crate::types::ImageRef;

/// A poisoned lock still holds a usable list.
fn lock<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    mutex.lock().unwrap_or_else(|e| e.into_inner())
}

/// Start of the note a spilled result ends with.
pub const SPILL_MARK: &str = "[full output: ";
/// The shell keeps a quarter of the room as head (2 KB of 8 KB), the rest
/// as tail: errors are at the end.
const SHELL_HEAD_SHARE: usize = 4;
/// Lines the shell keeps from the start of a cut result; the whole text is
/// in the spill file.
pub const SHELL_HEAD_LINES: usize = 20;
/// Lines the shell keeps from the end of a cut result.
pub const SHELL_TAIL_LINES: usize = 30;

/// What the tools of one session share: where spill files go and the read
/// ranges already returned. A resumed session starts with no ranges.
#[derive(Debug, Default)]
#[non_exhaustive]
pub struct ToolSession {
    /// `<session>.out`; `None` keeps nothing on disk.
    pub out_dir: Option<PathBuf>,
    reads: Mutex<Vec<ReadSpan>>,
    /// Background shell jobs (PLAN.md D68), shared with the service session.
    jobs: Arc<Jobs>,
    /// Prior states of the files `write` and `edit` change (PLAN.md D69).
    undo: Arc<UndoLog>,
    /// `<session>.media`: images kept by reference (PLAN.md D70).
    media_dir: Option<PathBuf>,
    /// Images attached for the next user message (PLAN.md D70).
    attached: Mutex<Vec<ImageRef>>,
    /// What the agent last saw of each file (`file_state`).
    files: FileStates,
    /// Results of `grep` and `glob` already returned (`search_seen`).
    searches: super::search_seen::Searches,
}

/// A returned read: path, offset, limit and the file's content hash
/// (`file_state::hash_of`: a touched but unchanged file stays the same).
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct ReadKey {
    pub path: PathBuf,
    pub offset: u64,
    pub limit: u64,
    pub hash: u64,
}

impl ToolSession {
    pub fn new(out_dir: Option<PathBuf>) -> ToolSession {
        // `<id>.out` -> `<id>.undo`, beside it.
        let undo_dir = out_dir.as_ref().map(|dir| dir.with_extension("undo"));
        let media_dir = out_dir.as_ref().map(|dir| dir.with_extension("media"));
        ToolSession {
            out_dir,
            media_dir,
            attached: Mutex::default(),
            files: FileStates::default(),
            searches: Default::default(),
            reads: Mutex::new(Vec::new()),
            jobs: Arc::default(),
            undo: Arc::new(UndoLog::new(undo_dir)),
        }
    }

    /// `<session>.media`, where images are kept (PLAN.md D70); `None`
    /// without a session folder.
    pub fn media_dir(&self) -> Option<&Path> {
        self.media_dir.as_deref()
    }

    /// Queues an image for the next user message.
    pub fn attach(&self, image: ImageRef) {
        lock(&self.attached).push(image);
    }

    /// The images queued for the next user message, in order.
    pub fn attached(&self) -> Vec<ImageRef> {
        lock(&self.attached).clone()
    }

    /// Takes the queued images (the next user message carries them).
    pub fn take_attached(&self) -> Vec<ImageRef> {
        std::mem::take(&mut *lock(&self.attached))
    }

    /// The session's undo store (PLAN.md D69).
    pub fn undo(&self) -> &Arc<UndoLog> {
        &self.undo
    }

    /// Uses `undo` from now on (a conversation reopened after a panic).
    pub fn share_undo(&mut self, undo: Arc<UndoLog>) {
        self.undo = undo;
    }

    /// The session's background jobs (PLAN.md D68).
    pub fn jobs(&self) -> &Arc<Jobs> {
        &self.jobs
    }

    /// Uses `jobs` from now on: a conversation reopened after a panic keeps
    /// its session's jobs.
    pub fn share_jobs(&mut self, jobs: Arc<Jobs>) {
        self.jobs = jobs;
    }

    /// The call that already returned `key`, if any: the same request, or
    /// lines inside a range an earlier read returned (PLAN.md D82), the
    /// file's content unchanged.
    pub fn read_by(&self, key: &ReadKey) -> Option<String> {
        let reads = self.reads.lock().unwrap_or_else(|e| e.into_inner());
        let wanted_end = key.offset.saturating_add(key.limit.saturating_sub(1));
        let covers = |span: &&ReadSpan| {
            let same_file = same_file(&span.key, key);
            let inside = span.key.offset <= key.offset && wanted_end <= span.end;
            same_file && (span.key == *key || inside)
        };
        reads.iter().find(covers).map(|span| span.call.clone())
    }

    /// Forgets every returned range: after a compaction the model no longer
    /// sees the earlier results (PLAN.md D38).
    pub fn clear_reads(&self) {
        let mut reads = self.reads.lock().unwrap_or_else(|e| e.into_inner());
        reads.clear();
        self.searches.clear();
    }

    /// The `grep` and `glob` results already returned.
    pub(crate) fn searches(&self) -> &super::search_seen::Searches {
        &self.searches
    }

    /// Records that `call_id` returned all of `key`'s lines.
    pub fn remember_read(&self, key: ReadKey, call_id: &str) {
        let end = key.offset.saturating_add(key.limit.saturating_sub(1));
        self.remember_span(key, end, call_id);
    }

    /// Records that `call_id`, asked for `key`, returned lines `key.offset`
    /// to `end` (a cut result ends early). The first call keeps a request.
    pub fn remember_span(&self, key: ReadKey, end: u64, call_id: &str) {
        let mut reads = self.reads.lock().unwrap_or_else(|e| e.into_inner());
        if reads.iter().any(|span| span.key == key) {
            return;
        }
        let call = call_id.to_string();
        reads.push(ReadSpan { key, end, call });
    }

    /// The part of `key`'s lines no earlier read of the unchanged file
    /// returned: earlier ranges that hold its first or its last lines are
    /// cut off (one in the middle is not). See [`Seen`].
    pub fn read_seen(&self, key: &ReadKey) -> Seen {
        let reads = lock(&self.reads);
        let spans: Vec<&ReadSpan> = reads.iter().filter(|s| same_file(&s.key, key)).collect();
        let end = key.offset.saturating_add(key.limit.saturating_sub(1));
        let mut seen = Seen {
            offset: key.offset,
            end,
            ..Seen::default()
        };
        while let Some(span) = spans
            .iter()
            .find(|s| s.key.offset <= seen.offset && seen.offset <= s.end)
        {
            push_new(&mut seen.before, &span.call);
            if span.end >= seen.end {
                seen.covered = true;
                return seen;
            }
            seen.offset = span.end + 1;
        }
        // A span holding `end` starts after `offset`, else the loop above
        // would have gone on from it: `end` stays at or past `offset`.
        while let Some(span) = spans
            .iter()
            .find(|s| s.key.offset <= seen.end && seen.end <= s.end)
        {
            push_new(&mut seen.after, &span.call);
            seen.to_end |= span.end == u64::MAX;
            seen.end = span.key.offset - 1;
        }
        seen
    }
}

/// Lines of a read request earlier reads returned (the file unchanged):
/// those before `offset` (by the calls `before`) and after `end` (by the
/// calls `after`, `to_end` when they reached the end of the file);
/// `covered` when every line was.
#[derive(Debug, Default, PartialEq, Eq)]
#[non_exhaustive]
pub struct Seen {
    pub offset: u64,
    pub end: u64,
    pub before: Vec<String>,
    pub after: Vec<String>,
    pub covered: bool,
    pub to_end: bool,
}

/// Same path and content hash: the file did not change between.
fn same_file(a: &ReadKey, b: &ReadKey) -> bool {
    (&a.path, a.hash) == (&b.path, b.hash)
}

/// A new line range for an old one (`None` drops it) and whether a change
/// touched it.
pub(crate) type MoveSpan<'a> = dyn Fn(u64, u64) -> Option<(u64, u64, bool)> + 'a;

/// File state (`file_state`): kept apart from the ranges above.
impl ToolSession {
    /// What the agent last saw of each file.
    pub(crate) fn files(&self) -> &FileStates {
        &self.files
    }

    /// Forgets the read ranges of `path` (the agent wrote all of it).
    pub(crate) fn forget_reads(&self, path: &Path) {
        lock(&self.reads).retain(|span| span.key.path != path);
    }

    /// Moves the ranges of `path` read at content `from` to content `to`
    /// through `map`; a range a change touched also names `call`.
    pub(crate) fn move_reads(
        &self,
        path: &Path,
        (from, to): (u64, u64),
        map: &MoveSpan<'_>,
        call: &str,
    ) {
        let mut reads = lock(&self.reads);
        let old = std::mem::take(&mut *reads);
        for span in old {
            if span.key.path != path || span.key.hash != from {
                reads.push(span);
                continue;
            }
            let Some((offset, end, touched)) = map(span.key.offset, span.end) else {
                continue;
            };
            let limit = match end {
                u64::MAX => span.key.limit,
                end => end - offset + 1,
            };
            let key = ReadKey {
                offset,
                limit,
                hash: to,
                ..span.key
            };
            let call = match touched && !span.call.split(", ").any(|c| c == call) {
                true => format!("{}, {call}", span.call),
                false => span.call,
            };
            reads.push(ReadSpan { key, end, call });
        }
    }
}

fn push_new(calls: &mut Vec<String>, call: &str) {
    if !calls.iter().any(|c| c == call) {
        calls.push(call.to_string());
    }
}

/// A read this session returned: the request, the last line it held and
/// the call id.
#[derive(Debug)]
struct ReadSpan {
    key: ReadKey,
    end: u64,
    call: String,
}
/// Fits `output` of tool `name` into `ctx.budget`: sets `bytes` to the full
/// size and, when cut, writes the spill file and sets `spilled`.
pub fn fit(name: &str, mut output: ToolOutput, ctx: &ToolCtx<'_>) -> ToolOutput {
    let full = output.text().into_owned();
    output.set_size(full.len(), None);
    let lines = full.lines().count();
    let head_tail_kept = SHELL_HEAD_LINES + SHELL_TAIL_LINES;
    let too_many_lines = keeps_head_tail(name) && lines > head_tail_kept;
    if full.len() <= ctx.budget && !too_many_lines {
        return output;
    }
    let spilled = spill(ctx, &full);
    let note = match &spilled {
        Some(path) => format!(
            "\n{SPILL_MARK}{}, {} bytes, {lines} lines; read it with offset and limit or grep it]",
            path.display(),
            full.len()
        ),
        None => format!(
            "\n[output cut: {} bytes, {lines} lines in full]",
            full.len()
        ),
    };
    let room = ctx.budget.saturating_sub(note.len());
    let body = match name {
        _ if keeps_head_tail(name) => head_tail(&full, room),
        "subagent" => head_and_last_line(&full, room),
        _ => head(&full, room).to_string(),
    };
    output.set_text(body + &note);
    output.set_size(full.len(), spilled.map(|path| path.display().to_string()));
    output
}

/// Writes `<out_dir>/<callId>.txt`; `None` outside a session or on failure.
fn spill(ctx: &ToolCtx<'_>, full: &str) -> Option<PathBuf> {
    let dir = ctx.session?.out_dir.as_ref()?;
    let name: String = ctx
        .call_id
        .chars()
        .map(
            |c| match c.is_ascii_alphanumeric() || c == '-' || c == '_' {
                true => c,
                false => '_',
            },
        )
        .collect();
    let name = if name.is_empty() { "call".into() } else { name };
    std::fs::create_dir_all(dir).ok()?;
    let path = dir.join(format!("{name}.txt"));
    std::fs::write(&path, full).ok()?;
    Some(path)
}

/// The longest start of `text` within `room` bytes, ending after a newline
/// when one is in reach, else on a character boundary.
pub fn head(text: &str, room: usize) -> &str {
    if text.len() <= room {
        return text;
    }
    let mut end = room;
    while !text.is_char_boundary(end) {
        end -= 1;
    }
    match text[..end].rfind('\n') {
        Some(newline) if newline > 0 => &text[..=newline],
        _ => &text[..end],
    }
}

/// The longest end of `text` within `room` bytes, starting on a line when
/// one is in reach.
fn tail(text: &str, room: usize) -> &str {
    if text.len() <= room {
        return text;
    }
    let mut start = text.len() - room;
    while !text.is_char_boundary(start) {
        start += 1;
    }
    match text[start..].find('\n') {
        Some(newline) if start + newline + 1 < text.len() => &text[start + newline + 1..],
        _ => &text[start..],
    }
}

/// Does tool `name` keep the head and the tail of a cut result (errors are
/// at the end of a command's output)?
fn keeps_head_tail(name: &str) -> bool {
    matches!(name, "shell" | "job_output")
}

/// A quarter of `room` from the start and the rest from the end, at most
/// [`SHELL_HEAD_LINES`] and [`SHELL_TAIL_LINES`] lines, with a marker
/// naming the lines and bytes left out.
fn head_tail(text: &str, room: usize) -> String {
    let marker_room = 64;
    let usable = room.saturating_sub(marker_room);
    let first = first_lines(head(text, usable / SHELL_HEAD_SHARE), SHELL_HEAD_LINES);
    let last = last_lines(
        tail(text, usable - usable / SHELL_HEAD_SHARE),
        SHELL_TAIL_LINES,
    );
    let cut = text.len().saturating_sub(first.len() + last.len());
    let total = text.lines().count();
    let kept = first.lines().count() + last.lines().count();
    let cut_lines = total.saturating_sub(kept);
    let mut out = first.to_string();
    if !out.ends_with('\n') {
        out.push('\n');
    }
    out.push_str(&format!("[... {cut_lines} lines, {cut} bytes cut ...]\n"));
    out.push_str(last);
    out
}

/// The first `count` lines of `text` (with their newlines).
fn first_lines(text: &str, count: usize) -> &str {
    match text.match_indices('\n').nth(count.saturating_sub(1)) {
        Some((index, _)) if count > 0 => &text[..=index],
        _ if count == 0 => "",
        _ => text,
    }
}

/// The last `count` lines of `text`; a final newline does not start a line.
fn last_lines(text: &str, count: usize) -> &str {
    let body = text.strip_suffix('\n').unwrap_or(text);
    match body.rmatch_indices('\n').nth(count.saturating_sub(1)) {
        Some((index, _)) if count > 0 => &text[index + 1..],
        _ if count == 0 => "",
        _ => text,
    }
}

/// The answer cut to fit with its last line (the subagent stats) kept.
fn head_and_last_line(text: &str, room: usize) -> String {
    let trimmed = text.trim_end();
    let (body, last) = trimmed.rsplit_once('\n').unwrap_or(("", trimmed));
    let keep = room.saturating_sub(last.len() + 2);
    let mut out = head(body, keep).trim_end().to_string();
    out.push_str("\n\n");
    out.push_str(last);
    out
}

/// The spill path named in a result's note, if it has one.
pub fn spill_path(content: &str) -> Option<&str> {
    let start = content.rfind(SPILL_MARK)? + SPILL_MARK.len();
    let rest = &content[start..];
    let end = rest.rfind(" bytes, ")?;
    rest[..end].rsplit_once(", ").map(|(path, _)| path)
}
