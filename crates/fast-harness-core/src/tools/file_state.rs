//! What this session's agent last saw of each file: the content hash and
//! line hashes of its last `read`, `edit` or `write`. `edit` and `write`
//! refuse a file the agent never read or that changed since (by someone
//! else); `read` returns only the lines that changed since. Each
//! conversation has its own [`super::ToolSession`], so a subagent starts
//! with nothing read: it reads a file before it changes it.

use std::collections::HashMap;
use std::hash::{DefaultHasher, Hasher};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};

use super::budget::ReadKey;
use super::line_diff::{self, Hunk};
use super::{ToolCtx, ToolSession};

/// A file larger than this is identified by its size and modified time:
/// hashing it would read all of it (`read` streams only what it shows).
pub(crate) const HASHED_MAX: u64 = 4 * 1024 * 1024;

/// The version of a file the agent last saw.
#[derive(Clone, Debug)]
pub(crate) struct Snapshot {
    pub hash: u64,
    /// Line hashes of the decoded text; `None` for an image or a large file.
    pub lines: Option<Arc<Vec<u64>>>,
    /// The call that saw it.
    pub call: String,
    /// The version (hash, line hashes) the session's reads returned before
    /// its own edits since: a re-read shows the lines they changed (D87).
    pub base: Option<(u64, Arc<Vec<u64>>)>,
}

/// Snapshots by canonical path.
#[derive(Debug, Default)]
pub struct FileStates {
    map: Mutex<HashMap<PathBuf, Snapshot>>,
    /// `<session>.files.json`, where the snapshots outlive a restart
    /// (PLAN.md D88); `None` without a session folder.
    saved: Option<PathBuf>,
}

impl FileStates {
    /// The snapshots kept in `saved` (none when it is absent or unreadable),
    /// written back there on every change.
    pub(crate) fn open(saved: Option<PathBuf>) -> FileStates {
        let map = saved.as_deref().map(saved_file::load).unwrap_or_default();
        FileStates {
            map: Mutex::new(map),
            saved,
        }
    }

    pub(crate) fn get(&self, path: &Path) -> Option<Snapshot> {
        let map = self.map.lock().unwrap_or_else(|e| e.into_inner());
        map.get(path).cloned()
    }

    pub(crate) fn record(&self, path: PathBuf, snapshot: Snapshot) {
        let mut map = self.map.lock().unwrap_or_else(|e| e.into_inner());
        map.insert(path, snapshot);
        // Under the lock, so the file never goes back to an older map.
        if let Some(saved) = &self.saved {
            saved_file::save(saved, &map);
        }
    }
}

/// `<session>.files.json` (version 1): `{"version":1,"files":[{path, hash,
/// call}]}`. Line hashes are not kept: after a reopen the first `read` of a
/// file returns it whole.
mod saved_file {
    use super::*;
    use serde::{Deserialize, Serialize};

    const VERSION: u32 = 1;

    #[derive(Serialize, Deserialize)]
    struct Saved {
        version: u32,
        files: Vec<SavedFile>,
    }

    #[derive(Serialize, Deserialize)]
    struct SavedFile {
        path: PathBuf,
        hash: u64,
        call: String,
    }

    pub(super) fn load(path: &Path) -> HashMap<PathBuf, Snapshot> {
        let bytes = crate::store::read(path).unwrap_or_default();
        let Ok(saved) = serde_json::from_slice::<Saved>(&bytes) else {
            return HashMap::new();
        };
        if saved.version != VERSION {
            return HashMap::new();
        }
        let snapshot = |f: SavedFile| {
            let (hash, call) = (f.hash, f.call);
            (
                f.path,
                Snapshot {
                    hash,
                    lines: None,
                    call,
                },
            )
        };
        saved.files.into_iter().map(snapshot).collect()
    }

    /// Best effort: a snapshot that is not saved only makes `edit` ask for
    /// a read after a restart.
    pub(super) fn save(path: &Path, map: &HashMap<PathBuf, Snapshot>) {
        let mut files: Vec<SavedFile> = (map.iter())
            .map(|(path, s)| SavedFile {
                path: path.clone(),
                hash: s.hash,
                call: s.call.clone(),
            })
            .collect();
        files.sort_by(|a, b| a.path.cmp(&b.path));
        let saved = Saved {
            version: VERSION,
            files,
        };
        if let Ok(bytes) = serde_json::to_vec(&saved) {
            let _ = crate::store::write_atomic(path, &bytes);
        }
    }
}

/// The key a file is known by: its canonical path when it has one.
pub(crate) fn file_key(path: &Path) -> PathBuf {
    std::fs::canonicalize(path).unwrap_or_else(|_| path.to_path_buf())
}

/// The content hash of `path` holding `bytes`: of the bytes up to
/// [`HASHED_MAX`], else of the size and modified time.
pub(crate) fn hash_of(path: &Path, bytes: &[u8]) -> u64 {
    if bytes.len() as u64 > HASHED_MAX {
        return meta_hash(path);
    }
    let mut hasher = DefaultHasher::new();
    hasher.write(bytes);
    hasher.finish()
}

/// The hash of a large file: of its size and modified time.
pub(crate) fn meta_hash(path: &Path) -> u64 {
    let mut hasher = DefaultHasher::new();
    let meta = std::fs::metadata(path).ok();
    hasher.write_u64(meta.as_ref().map_or(0, |m| m.len()));
    let modified = meta.and_then(|m| m.modified().ok());
    let since = modified.and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok());
    hasher.write_u128(since.map_or(0, |d| d.as_nanos()));
    hasher.finish()
}

/// Refuses to change `path` (holding `bytes`) unless this session read or
/// wrote that very content; nothing is checked outside a session.
pub(crate) fn check_seen(
    ctx: &ToolCtx<'_>,
    path: &Path,
    path_arg: &str,
    bytes: &[u8],
) -> Result<(), String> {
    let Some(session) = ctx.session else {
        return Ok(());
    };
    match session.files().get(&file_key(path)) {
        None => Err(format!(
            "{path_arg} was not read in this session; read it first, then change it"
        )),
        Some(seen) if seen.hash != hash_of(path, bytes) => Err(format!(
            "{path_arg} changed since this session last read or wrote it (call {}); read it \
             again, then change it",
            seen.call
        )),
        Some(_) => Ok(()),
    }
}

/// Records what `read` returned of `path` (`text` when it is text).
pub(crate) fn record_read(
    session: &ToolSession,
    path: PathBuf,
    hash: u64,
    text: Option<&str>,
    call: &str,
) {
    let lines = text.map(|text| Arc::new(line_diff::line_hashes(text)));
    let before = session.files().get(&path).and_then(|s| s.base);
    if let (Some((base, old)), Some(new)) = (before, &lines) {
        // Ranges the diff did not show keep only the lines no edit touched.
        let hunks = line_diff::hunks(&old, new);
        move_spans(session, &path, (base, hash), &hunks, Some(&[]), call);
    }
    let call = call.to_string();
    let base = None;
    session.files().record(
        path,
        Snapshot {
            hash,
            lines,
            call,
            base,
        },
    );
}

/// After the agent's own `edit` or `write` turned `old` into `new` text
/// (now `bytes` on disk): the new version is the one it knows, and read
/// ranges move with it. `whole` (a `write`) makes the whole file known.
pub(crate) fn record_change(
    ctx: &ToolCtx<'_>,
    path: &Path,
    (old, new): (&str, &str),
    bytes: &[u8],
    whole: bool,
) {
    let Some(session) = ctx.session else {
        return;
    };
    let key = file_key(path);
    let hash = hash_of(path, bytes);
    let before = session.files().get(&key);
    let lines = Arc::new(line_diff::line_hashes(new));
    let call = ctx.call_id.to_string();
    let mut base = None;
    if whole {
        session.forget_reads(&key);
        let read = ReadKey {
            path: key.clone(),
            offset: 1,
            limit: u64::MAX,
            hash,
        };
        session.remember_span(read, u64::MAX, &call);
    } else if let Some(before) = before {
        // The read ranges stay at the version they were read at: the next
        // read diffs against it and returns the lines the edits changed.
        let old_lines = || Arc::new(line_diff::line_hashes(old));
        base = Some(before.base.unwrap_or_else(|| (before.hash, old_lines())));
    }
    session.files().record(
        key,
        Snapshot {
            hash,
            lines: Some(lines),
            call,
            base,
        },
    );
}

/// Moves the read ranges of `key` at content `hashes.0` to `hashes.1`
/// through `hunks`. A range a hunk not in `shown` touches is dropped (its
/// lines changed unseen); `None` shows every hunk. A touched range names
/// `call` after its own.
pub(crate) fn move_spans(
    session: &ToolSession,
    key: &Path,
    hashes: (u64, u64),
    hunks: &[Hunk],
    shown: Option<&[Hunk]>,
    call: &str,
) {
    let touches = |h: &Hunk, start: u64, end: u64| {
        let (first, last) = (h.old as u64 + 1, (h.old + h.old_len) as u64);
        first <= end.saturating_add(1) && last.saturating_add(1) >= start
    };
    let map = |start: u64, end: u64| -> Option<(u64, u64, bool)> {
        let touched: Vec<&Hunk> = hunks.iter().filter(|h| touches(h, start, end)).collect();
        if touched
            .iter()
            .any(|h| shown.is_some_and(|s| !s.contains(h)))
        {
            return None;
        }
        let new_start = line_diff::old_to_new(hunks, start);
        let new_end = match end {
            u64::MAX => u64::MAX,
            end => line_diff::old_to_new(hunks, end + 1)
                .saturating_sub(1)
                .max(new_start),
        };
        Some((new_start, new_end, !touched.is_empty()))
    };
    session.move_reads(key, hashes, &map, call);
}
