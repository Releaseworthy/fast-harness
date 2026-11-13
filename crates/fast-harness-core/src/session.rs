//! Session ids, paths, listing and resume by replaying the log.
//! Layout (docs/formats.md "Files on disk"):
//! `<app_home>/sessions/<cwd slug>/<id>.jsonl` and `<id>.wire/`.

mod children;
pub mod compacted;
pub mod coord;
pub mod index;
pub mod lock;
mod unfinished;

pub use children::{children, Child};
pub use index::{latest, list, list_counted, Listing};
pub use unfinished::{unfinished, Unfinished};

use crate::trace::{self, Record, TraceEvent};
use crate::types::{Block, Message, Role};
use std::io;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::OnceLock;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SessionSummary {
    pub id: String,
    /// Unix milliseconds.
    pub created_at: u64,
    /// First user message text, at most 60 characters.
    pub title: String,
    /// `turn/start` events.
    pub turns: u64,
    /// Time of the last event, or `created_at` when there is none.
    pub last_time: u64,
    /// The last turn can be resumed: paused, cancelled or interrupted (D39).
    pub unfinished: Option<Unfinished>,
    /// The session that started this one as a subagent (its header's parent).
    pub parent: Option<String>,
}

/// The sessions a person started: subagent sessions (a parent in the
/// header) leave the sidebar lists and stay reachable from their parent
/// (PLAN.md D79).
pub fn top_level(sessions: Vec<SessionSummary>) -> Vec<SessionSummary> {
    sessions
        .into_iter()
        .filter(|s| s.parent.is_none())
        .collect()
}

const TITLE_CHARS: usize = 60;

/// `<yyyymmdd-hhmmss>-<4 hex>` from the UTC clock and a process-wide counter,
/// so ids sort by creation time. The counter starts at a per-process value
/// below 0x8000 so two processes rarely pick the same id in the same second.
pub fn new_id() -> String {
    static START: OnceLock<u32> = OnceLock::new();
    static COUNTER: AtomicU32 = AtomicU32::new(0);
    let start = *START.get_or_init(|| {
        let nanos = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.subsec_nanos())
            .unwrap_or(0);
        (nanos ^ std::process::id().rotate_left(7)) & 0x7fff
    });
    let count = start.wrapping_add(COUNTER.fetch_add(1, Ordering::Relaxed)) & 0xffff;
    format!("{}-{count:04x}", utc_stamp(trace::now_ms() / 1000))
}

/// `yyyymmdd-hhmmss` of a Unix time in seconds, in UTC.
pub fn utc_stamp(unix_secs: u64) -> String {
    let days = (unix_secs / 86_400) as i64;
    let secs = unix_secs % 86_400;
    let (year, month, day) = civil_from_days(days);
    let (hour, minute, second) = (secs / 3600, secs % 3600 / 60, secs % 60);
    format!("{year:04}{month:02}{day:02}-{hour:02}{minute:02}{second:02}")
}

/// Proleptic Gregorian date of a day count since 1970-01-01
/// (Howard Hinnant's `civil_from_days`).
fn civil_from_days(days: i64) -> (i64, u32, u32) {
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z.rem_euclid(146_097);
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let day = (doy - (153 * mp + 2) / 5 + 1) as u32;
    let month = if mp < 10 { mp + 3 } else { mp - 9 } as u32;
    let year = yoe + era * 400 + i64::from(month <= 2);
    (year, month, day)
}

/// Folder name for a working directory: every run of `:`, `\` and `/`
/// becomes one `-`, wrapped in `--` (`F:\work\x` gives `--F-work-x--`).
pub fn slug(cwd: &Path) -> String {
    let mut out = String::from("--");
    let mut in_separator = false;
    for c in cwd.to_string_lossy().chars() {
        if matches!(c, ':' | '\\' | '/') {
            if !in_separator {
                out.push('-');
            }
            in_separator = true;
        } else {
            out.push(c);
            in_separator = false;
        }
    }
    out.push_str("--");
    out
}

pub fn dir(app_home: &Path, cwd: &Path) -> PathBuf {
    app_home.join("sessions").join(slug(cwd))
}

/// A session id is one file name stem: no separator and no dot, so it cannot
/// name a path outside its sessions folder.
pub fn valid_id(id: &str) -> bool {
    !id.is_empty() && !id.contains(['/', '\\', '.'])
}

pub fn log_path(app_home: &Path, cwd: &Path, id: &str) -> PathBuf {
    dir(app_home, cwd).join(format!("{id}.jsonl"))
}

pub fn wire_dir(app_home: &Path, cwd: &Path, id: &str) -> PathBuf {
    dir(app_home, cwd).join(format!("{id}.wire"))
}

/// Spill files of the session's cut tool results (PLAN.md D37).
pub fn out_dir(app_home: &Path, cwd: &Path, id: &str) -> PathBuf {
    dir(app_home, cwd).join(format!("{id}.out"))
}

/// The summary of the log at `path` from one full parse (what the index of
/// [`list`] caches); `None` when it cannot be read.
pub fn summary_of(path: &Path) -> Option<SessionSummary> {
    let (header, events) = trace::read(path).ok()?;
    Some(summarize(header, &events))
}

fn summarize(header: trace::SessionHeader, events: &[TraceEvent]) -> SessionSummary {
    let title = events
        .iter()
        .find_map(|e| match e.record() {
            Record::UserMessage(user) => Some(user.message),
            _ => None,
        })
        .map(|m| {
            title_words(&m)
                .chars()
                .take(TITLE_CHARS)
                .collect::<String>()
        })
        .unwrap_or_default();
    let turns = events
        .iter()
        .filter(|e| e.kind == trace::kind::TURN_START)
        .count() as u64;
    let last_time = events
        .iter()
        .map(|e| e.time)
        .max()
        .unwrap_or(header.created_at);
    SessionSummary {
        id: header.id,
        created_at: header.created_at,
        title: title.lines().next().unwrap_or_default().to_string(),
        turns,
        last_time,
        unfinished: unfinished(events),
        parent: header.parent,
    }
}

/// The words a title starts from: with images (D70) the user's own text,
/// the last block of `media::user_message`, not the `[image N: ...]` labels
/// before it; the labels only when that text is blank.
fn title_words(message: &Message) -> String {
    if message.images().next().is_some() {
        if let Some(Block::Text { text }) = message.content.last() {
            if !text.trim().is_empty() {
                return text.trim_start().to_string();
            }
        }
    }
    message.text()
}

/// True for the event types that carry a message (PLAN.md D61).
fn carries_message(kind: &str) -> bool {
    matches!(
        kind,
        trace::kind::USER_MESSAGE | trace::kind::ASSISTANT_MESSAGE | trace::kind::TOOL_RESULT
    )
}

/// The error of a line that must carry a message but does not have the shape
/// this build reads.
fn no_message(event: &TraceEvent) -> io::Error {
    let text = format!(
        "seq {}: {} has no valid message: its data is not the shape this build reads",
        event.seq, event.kind
    );
    io::Error::new(io::ErrorKind::InvalidData, text)
}

/// Rebuilds the conversation from `user/message`, `assistant/message` and
/// `tool/result` events in seq order, starting from the history of the last
/// `compaction` (PLAN.md D38). A turn without `turn/end` (a crash or a
/// cancel) still counts.
pub fn replay(path: &Path) -> io::Result<Vec<Message>> {
    replay_events(&trace::read(path)?.1)
}

/// [`replay`] of events already read.
pub fn replay_events(events: &[TraceEvent]) -> io::Result<Vec<Message>> {
    let mut sorted: Vec<&TraceEvent> = events.iter().collect();
    sorted.sort_by_key(|e| e.seq);
    let mut messages = Vec::new();
    for event in sorted {
        let (role, message) = match event.record() {
            Record::UserMessage(user) => (Role::User, user.message),
            Record::AssistantMessage(answer) => (Role::Assistant, answer.message),
            Record::ToolResult(result) => (Role::Tool, result.message),
            Record::Compaction(compaction) => {
                messages = compacted::from_event(&compaction);
                continue;
            }
            Record::Raw { .. } if carries_message(&event.kind) => return Err(no_message(event)),
            _ => continue,
        };
        if message.role != role {
            let text = format!(
                "seq {}: {} holds a {:?} message",
                event.seq, event.kind, message.role
            );
            return Err(io::Error::new(io::ErrorKind::InvalidData, text));
        }
        unfinished::push(&mut messages, message, event);
    }
    Ok(messages)
}

/// A working directory that has session logs under `<app_home>/sessions/`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct KnownCwd {
    /// The `cwd` in the header of the folder's newest log.
    pub cwd: PathBuf,
    /// `created_at` of the oldest and the newest log (unix ms).
    pub first: u64,
    pub last: u64,
}

/// Every session folder with at least one readable log, one entry per folder.
/// Only the header line of each log is read; a log without a valid header is
/// left out. A missing `sessions` folder is an empty list.
pub fn known_cwds(app_home: &Path) -> io::Result<Vec<KnownCwd>> {
    let entries = match std::fs::read_dir(app_home.join("sessions")) {
        Ok(entries) => entries,
        Err(err) if err.kind() == io::ErrorKind::NotFound => return Ok(Vec::new()),
        Err(err) => return Err(err),
    };
    let mut known = Vec::new();
    for entry in entries {
        let path = entry?.path();
        if path.is_dir() {
            known.extend(folder_cwd(&path)?);
        }
    }
    Ok(known)
}

fn folder_cwd(folder: &Path) -> io::Result<Option<KnownCwd>> {
    let mut found: Option<KnownCwd> = None;
    for entry in std::fs::read_dir(folder)? {
        let path = entry?.path();
        if !coord::is_log(&path) {
            continue;
        }
        let Some(header) = read_header(&path) else {
            continue;
        };
        let at = header.created_at;
        match &mut found {
            Some(k) => {
                k.first = k.first.min(at);
                if at >= k.last {
                    k.last = at;
                    k.cwd = PathBuf::from(header.cwd);
                }
            }
            None => {
                found = Some(KnownCwd {
                    cwd: header.cwd.into(),
                    first: at,
                    last: at,
                })
            }
        }
    }
    Ok(found.filter(|k| !k.cwd.as_os_str().is_empty()))
}

/// The header line of the log at `path` (nothing else is read).
pub fn read_header(path: &Path) -> Option<trace::SessionHeader> {
    use std::io::BufRead;
    let file = std::fs::File::open(path).ok()?;
    let mut line = String::new();
    io::BufReader::new(file).read_line(&mut line).ok()?;
    serde_json::from_str(line.trim()).ok()
}
