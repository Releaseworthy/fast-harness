//! The output diet (PLAN.md D37): every tool result fits one byte budget
//! before it is first sent. A longer result keeps a cut of its text and
//! sends the whole text to `<session>.out/<callId>.txt`, named by a note at
//! the end of the result.

use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, MutexGuard};
use std::time::SystemTime;

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
}

/// A returned read: path, offset, limit, file size and modified time.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct ReadKey {
    pub path: PathBuf,
    pub offset: u64,
    pub limit: u64,
    pub size: u64,
    pub modified: Option<SystemTime>,
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
    /// file's size and modified time unchanged.
    pub fn read_by(&self, key: &ReadKey) -> Option<String> {
        let reads = self.reads.lock().unwrap_or_else(|e| e.into_inner());
        let wanted_end = key.offset.saturating_add(key.limit.saturating_sub(1));
        let covers = |span: &&ReadSpan| {
            let same_file = (&span.key.path, span.key.size, span.key.modified)
                == (&key.path, key.size, key.modified);
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
    if full.len() <= ctx.budget {
        return output;
    }
    let lines = full.lines().count();
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
        "shell" | "job_output" => head_tail(&full, room),
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

/// A quarter of `room` from the start and the rest from the end, with a
/// marker naming the bytes left out.
fn head_tail(text: &str, room: usize) -> String {
    let marker_room = 48;
    let usable = room.saturating_sub(marker_room);
    let first = head(text, usable / SHELL_HEAD_SHARE);
    let last = tail(text, usable - usable / SHELL_HEAD_SHARE);
    let cut = text.len().saturating_sub(first.len() + last.len());
    let mut out = first.to_string();
    if !out.ends_with('\n') {
        out.push('\n');
    }
    out.push_str(&format!("[... {cut} bytes cut ...]\n"));
    out.push_str(last);
    out
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
