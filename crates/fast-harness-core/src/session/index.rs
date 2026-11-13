//! The session index (PLAN.md D60, gate G10): `sessions/<slug>/index.json`
//! caches the summary of every log under its size and modified time, so a
//! list parses only the logs that changed since the last list. It is a
//! cache: a missing, damaged, older or unwritable index costs parses, never
//! an error or a wrong answer.

use super::{coord, dir, summary_of, SessionSummary, Unfinished};
use crate::store;
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::io;
use std::path::{Path, PathBuf};
use std::time::{Duration, UNIX_EPOCH};

/// The index file in each sessions folder.
pub const INDEX_FILE: &str = "index.json";
/// Format version of the index; another version is rebuilt.
/// 2: titles of first prompts with images use the user's words (D70).
pub const VERSION: u32 = 2;
/// How long a list waits for another process writing the index; after that
/// it answers without writing.
const WAIT: Duration = Duration::from_millis(500);

#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
struct Index {
    version: u32,
    /// Per log file name (`<id>.jsonl`).
    logs: BTreeMap<String, Entry>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
struct Entry {
    size: u64,
    /// Modified time, nanoseconds since 1970.
    modified: u64,
    /// `None` for a log that cannot be read (left out of the list).
    summary: Option<Cached>,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct Cached {
    id: String,
    created_at: u64,
    title: String,
    turns: u64,
    last_time: u64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    unfinished: Option<String>,
}

impl Cached {
    fn new(s: &SessionSummary) -> Cached {
        Cached {
            id: s.id.clone(),
            created_at: s.created_at,
            title: s.title.clone(),
            turns: s.turns,
            last_time: s.last_time,
            unfinished: s.unfinished.map(|u| u.word().to_string()),
        }
    }

    fn summary(&self) -> SessionSummary {
        SessionSummary {
            id: self.id.clone(),
            created_at: self.created_at,
            title: self.title.clone(),
            turns: self.turns,
            last_time: self.last_time,
            unfinished: self.unfinished.as_deref().and_then(Unfinished::from_word),
        }
    }
}

/// What one [`list_counted`] did.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Listing {
    /// Newest first.
    pub sessions: Vec<SessionSummary>,
    /// Logs parsed: no entry, or their size or modified time differ.
    pub parsed: usize,
    /// Logs answered from the index without reading them.
    pub cached: usize,
    /// The index was written back.
    pub written: bool,
}

/// Sessions of `cwd`, newest first. A missing folder is an empty list; a log
/// that cannot be read is left out. Runs on any thread (no shared state
/// besides the index file, written through `store`).
pub fn list(app_home: &Path, cwd: &Path) -> io::Result<Vec<SessionSummary>> {
    Ok(list_counted(app_home, cwd)?.sessions)
}

/// [`list`] with the count of logs parsed and answered from the index.
pub fn list_counted(app_home: &Path, cwd: &Path) -> io::Result<Listing> {
    crate::ui_thread::forbid("session::list");
    let folder = dir(app_home, cwd);
    let Some(logs) = logs(&folder)? else {
        return Ok(Listing::default());
    };
    let path = folder.join(INDEX_FILE);
    let old = read_index(&path);
    let mut new = Index {
        version: VERSION,
        logs: BTreeMap::new(),
    };
    let mut listing = Listing::default();
    for (name, log) in logs {
        let Some(entry) = entry(&old, &name, &log, &mut listing) else {
            continue;
        };
        listing
            .sessions
            .extend(entry.summary.as_ref().map(Cached::summary));
        new.logs.insert(name, entry);
    }
    (listing.sessions).sort_by(|a, b| (b.created_at, &b.id).cmp(&(a.created_at, &a.id)));
    if new != old {
        listing.written = write_index(&path, &folder, new).is_ok();
    }
    Ok(listing)
}

/// The cached entry of `name` when its stamp matches, else a fresh one. The
/// stamp is taken before the read: a log that grows meanwhile is stale on
/// the next list (one more parse), never cached as current.
fn entry(old: &Index, name: &str, log: &Path, listing: &mut Listing) -> Option<Entry> {
    let (size, modified) = stamp(log)?;
    match old.logs.get(name) {
        Some(hit) if hit.size == size && hit.modified == modified => {
            listing.cached += 1;
            Some(hit.clone())
        }
        _ => {
            listing.parsed += 1;
            let summary = summary_of(log).map(|s| Cached::new(&s));
            Some(Entry {
                size,
                modified,
                summary,
            })
        }
    }
}

/// The newest session of `cwd`: the highest id among the log names (ids
/// start with their UTC second, so name order is time order), answered from
/// the index or by parsing that one log. A folder without id-named logs
/// falls back to [`list`]. The index is not written.
pub fn latest(app_home: &Path, cwd: &Path) -> io::Result<Option<SessionSummary>> {
    crate::ui_thread::forbid("session::latest");
    let folder = dir(app_home, cwd);
    let Some(logs) = logs(&folder)? else {
        return Ok(None);
    };
    let mut names: Vec<&str> = (logs.iter().map(|(name, _)| name.as_str()))
        .filter(|name| is_id_name(name))
        .collect();
    names.sort_unstable();
    let index = read_index(&folder.join(INDEX_FILE));
    for name in names.into_iter().rev() {
        let mut listing = Listing::default();
        let found = entry(&index, name, &folder.join(name), &mut listing);
        if let Some(summary) = found.and_then(|e| e.summary) {
            return Ok(Some(summary.summary()));
        }
    }
    Ok(list(app_home, cwd)?.into_iter().next())
}

/// `<yyyymmdd-hhmmss-xxxx>.jsonl` (see [`super::new_id`]).
pub(super) fn is_id_name(name: &str) -> bool {
    let Some(stem) = name.strip_suffix(".jsonl") else {
        return false;
    };
    let b = stem.as_bytes();
    b.len() == 20
        && b[8] == b'-'
        && b[15] == b'-'
        && (b[..8].iter().chain(&b[9..15])).all(u8::is_ascii_digit)
        && b[16..].iter().all(u8::is_ascii_hexdigit)
}

/// `(file name, path)` of every log in `folder`; `None` when it is missing.
pub(super) fn logs(folder: &Path) -> io::Result<Option<Vec<(String, PathBuf)>>> {
    let entries = match std::fs::read_dir(folder) {
        Ok(entries) => entries,
        Err(err) if err.kind() == io::ErrorKind::NotFound => return Ok(None),
        Err(err) => return Err(err),
    };
    let mut logs = Vec::new();
    for entry in entries {
        let path = entry?.path();
        let name = path
            .file_name()
            .and_then(|n| n.to_str())
            .map(str::to_string);
        if let Some(name) = name.filter(|_| coord::is_log(&path)) {
            logs.push((name, path));
        }
    }
    Ok(Some(logs))
}

/// The index at `path`; empty when missing, damaged or of another version.
fn read_index(path: &Path) -> Index {
    store::read(path)
        .ok()
        .and_then(|bytes| serde_json::from_slice::<Index>(&bytes).ok())
        .filter(|index| index.version == VERSION)
        .unwrap_or_default()
}

/// Writes `new` under the index lock, keeping the entries another process
/// wrote meanwhile for logs this list did not see that still exist.
fn write_index(path: &Path, folder: &Path, mut new: Index) -> io::Result<()> {
    store::update(path, WAIT, |current| {
        let current = current
            .and_then(|bytes| serde_json::from_slice::<Index>(&bytes).ok())
            .filter(|index| index.version == VERSION);
        for (name, entry) in current.map(|index| index.logs).unwrap_or_default() {
            if !new.logs.contains_key(&name) && folder.join(&name).is_file() {
                new.logs.insert(name, entry);
            }
        }
        Ok((serde_json::to_vec(&new)?, ()))
    })
}

/// Size and modified time (nanoseconds since 1970) of a regular file. On
/// Windows `fs::metadata` asks the open file, so a log being written is
/// current (a directory entry can lag behind it).
fn stamp(path: &Path) -> Option<(u64, u64)> {
    let meta = std::fs::metadata(path).ok().filter(|m| m.is_file())?;
    let modified = meta.modified().ok()?.duration_since(UNIX_EPOCH).ok()?;
    Some((meta.len(), modified.as_nanos() as u64))
}
