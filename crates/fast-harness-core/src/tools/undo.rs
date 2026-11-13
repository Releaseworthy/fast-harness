//! Undo of a turn's file changes (PLAN.md D69). Before `write` or `edit`
//! changes a file during a turn, its prior state is kept once per file per
//! turn (the bytes, or "did not exist") with the hash of what the turn left,
//! in `<session>.undo/turn-<n>/` (a `manifest.json` plus one `<i>.bin` per
//! file that existed), written through `store::write_atomic`. Changes made
//! by `shell`, background jobs or subagents are not tracked.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::sync::{Mutex, MutexGuard};

use serde::{Deserialize, Serialize};

use crate::store;

/// Prior bytes kept for one turn before it is marked not undoable.
pub const TURN_CAP: u64 = 16 * 1024 * 1024;
/// Prior bytes kept for one session (undoable turns not yet undone).
pub const SESSION_CAP: u64 = 64 * 1024 * 1024;
/// What every undo result says about the changes it cannot see.
pub const UNTRACKED: &str = "Files changed through shell commands, background jobs or \
subagents are not tracked and were not undone.";
const MANIFEST: &str = "manifest.json";
const FORMAT: u32 = 1;

/// The size caps of an [`UndoLog`].
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Caps {
    pub turn: u64,
    pub session: u64,
}

impl Default for Caps {
    fn default() -> Caps {
        Caps {
            turn: TURN_CAP,
            session: SESSION_CAP,
        }
    }
}

/// One file a turn changed.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct Tracked {
    pub path: PathBuf,
    /// The file existed before the turn (false: the turn created it).
    #[serde(default)]
    pub existed: bool,
    /// The blob holding the prior bytes (gone after an undo or over a cap).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub before: Option<String>,
    /// [`hash`] of the content the turn left.
    #[serde(default)]
    pub after: String,
}

/// `turn-<n>/manifest.json`.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct Manifest {
    pub version: u32,
    pub turn: u32,
    pub files: Vec<Tracked>,
    /// Prior bytes kept on disk for this turn.
    pub bytes: u64,
    /// Why this turn cannot be undone, in words.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub not_undoable: Option<String>,
    #[serde(default)]
    pub undone: bool,
    /// The model was told about the undo (the note went into a user message).
    #[serde(default)]
    pub noted: bool,
}

/// What a view shows about the next undo: the turn and its file count.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct UndoInfo {
    pub turn: u32,
    pub files: usize,
    /// Why it cannot be undone (over a cap); `None` when it can.
    pub not_undoable: Option<String>,
}

/// The undo store of one session, shared by its tools and its service session.
#[derive(Debug, Default)]
pub struct UndoLog {
    /// `<session>.undo`; `None` tracks nothing.
    dir: Option<PathBuf>,
    inner: Mutex<Inner>,
}

#[derive(Debug, Default)]
pub(crate) struct Inner {
    loaded: bool,
    pub(crate) turns: BTreeMap<u32, Manifest>,
    current: u32,
    caps: Caps,
}

impl UndoLog {
    pub fn new(dir: Option<PathBuf>) -> UndoLog {
        UndoLog {
            dir,
            inner: Mutex::default(),
        }
    }

    /// Replaces the size caps (tests, settings).
    pub fn set_caps(&self, caps: Caps) {
        self.lock().caps = caps;
    }

    /// Changes from now on belong to `turn`.
    pub fn begin_turn(&self, turn: u32) {
        self.lock().current = turn;
    }

    pub(crate) fn dir(&self) -> Option<&Path> {
        self.dir.as_deref()
    }

    /// The state, read from disk on first use.
    pub(crate) fn lock(&self) -> MutexGuard<'_, Inner> {
        let mut inner = self.inner.lock().unwrap_or_else(|p| p.into_inner());
        if !inner.loaded {
            inner.loaded = true;
            if let Some(dir) = &self.dir {
                inner.turns = load(dir);
            }
        }
        inner
    }

    /// The most recent turn with tracked changes that is not undone.
    pub fn last(&self) -> Option<UndoInfo> {
        let inner = self.lock();
        let manifest = last_open(&inner.turns)?;
        Some(UndoInfo {
            turn: manifest.turn,
            files: manifest.files.len(),
            not_undoable: manifest.not_undoable.clone(),
        })
    }

    /// Keeps the prior state of `path` before the current turn changes it
    /// (once per file per turn); nothing outside a turn or a session.
    pub(crate) fn before_change(&self, path: &Path) {
        let Some(dir) = self.dir.clone() else { return };
        let mut inner = self.lock();
        let (turn, caps) = (inner.current, inner.caps);
        if turn == 0 {
            return;
        }
        let used = session_bytes(&inner.turns);
        let manifest = inner.turns.entry(turn).or_insert_with(|| Manifest {
            version: FORMAT,
            turn,
            ..Manifest::default()
        });
        if manifest.files.iter().any(|f| same_file(&f.path, path)) {
            return;
        }
        let index = manifest.files.len();
        let mut tracked = Tracked {
            path: path.to_path_buf(),
            ..Tracked::default()
        };
        if manifest.not_undoable.is_none() {
            let room = Room { caps, used };
            keep(&dir, manifest, &mut tracked, index, room);
        } else {
            tracked.existed = path.exists();
        }
        manifest.files.push(tracked);
        save(&dir, manifest);
    }

    /// Records what the current turn left in `path`.
    pub(crate) fn after_change(&self, path: &Path, bytes: &[u8]) {
        let Some(dir) = self.dir.clone() else { return };
        let mut inner = self.lock();
        let turn = inner.current;
        let Some(manifest) = inner.turns.get_mut(&turn) else {
            return;
        };
        if let Some(file) = manifest.files.iter_mut().find(|f| same_file(&f.path, path)) {
            file.after = hash(bytes);
            save(&dir, manifest);
        }
    }
}

/// Whether `a` and `b` name one file (`sub/../a.txt` and `a.txt`, a
/// drive letter or a name in another case on Windows, a link): `..`
/// resolved by name, links of the existing part followed, case folded.
fn same_file(a: &Path, b: &Path) -> bool {
    let key = |path: &Path| {
        let text = path.to_string_lossy();
        crate::config::approvals::normal_path(&text, Path::new(""))
            .unwrap_or_else(|| path.to_path_buf())
    };
    a == b || key(a) == key(b)
}

/// The caps and the bytes the session already keeps.
struct Room {
    caps: Caps,
    used: u64,
}

/// Stores the prior bytes of `tracked` as blob `index`, or marks the turn
/// not undoable when a cap or an error stops it.
fn keep(dir: &Path, manifest: &mut Manifest, tracked: &mut Tracked, index: usize, room: Room) {
    let prior = match std::fs::read(&tracked.path) {
        Ok(bytes) => bytes,
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => return,
        Err(err) => {
            tracked.existed = true;
            let path = tracked.path.display();
            return give_up(
                dir,
                manifest,
                format!("its earlier content of {path} could not be read ({err})"),
            );
        }
    };
    tracked.existed = true;
    let size = prior.len() as u64;
    if manifest.bytes + size > room.caps.turn {
        let cap = mib(room.caps.turn);
        return give_up(
            dir,
            manifest,
            format!("its earlier file content passed the {cap} cap per turn"),
        );
    }
    if room.used + size > room.caps.session {
        let cap = mib(room.caps.session);
        return give_up(
            dir,
            manifest,
            format!("the session's kept file content passed its {cap} cap"),
        );
    }
    let name = format!("{index}.bin");
    let path = turn_dir(dir, manifest.turn).join(&name);
    match store::write_atomic(&path, &prior) {
        Ok(()) => {
            manifest.bytes += size;
            tracked.before = Some(name);
        }
        Err(err) => give_up(
            dir,
            manifest,
            format!("its earlier file content could not be kept ({err})"),
        ),
    }
}

/// Marks `manifest` not undoable for `reason` and drops its blobs.
fn give_up(dir: &Path, manifest: &mut Manifest, reason: String) {
    drop_blobs(dir, manifest);
    manifest.not_undoable = Some(reason);
}

/// Removes the blobs of `manifest` (after an undo or over a cap).
pub(crate) fn drop_blobs(dir: &Path, manifest: &mut Manifest) {
    let folder = turn_dir(dir, manifest.turn);
    for file in &mut manifest.files {
        if let Some(blob) = file.before.take() {
            let _ = std::fs::remove_file(folder.join(blob));
        }
    }
    manifest.bytes = 0;
}

fn mib(bytes: u64) -> String {
    match bytes >= 1024 * 1024 {
        true => format!("{} MiB", bytes / (1024 * 1024)),
        false => format!("{bytes} bytes"),
    }
}

pub(crate) fn turn_dir(dir: &Path, turn: u32) -> PathBuf {
    dir.join(format!("turn-{turn}"))
}

/// Writes `manifest`; a failure leaves the earlier one (best effort).
pub(crate) fn save(dir: &Path, manifest: &Manifest) {
    if let Ok(text) = serde_json::to_vec_pretty(manifest) {
        let _ = store::write_atomic(&turn_dir(dir, manifest.turn).join(MANIFEST), &text);
    }
}

/// The manifests under `dir`; unreadable ones are skipped.
fn load(dir: &Path) -> BTreeMap<u32, Manifest> {
    let mut turns = BTreeMap::new();
    let Ok(entries) = std::fs::read_dir(dir) else {
        return turns;
    };
    for entry in entries.flatten() {
        let path = entry.path().join(MANIFEST);
        let parsed = store::read(&path)
            .ok()
            .and_then(|bytes| serde_json::from_slice::<Manifest>(&bytes).ok());
        if let Some(manifest) = parsed {
            turns.insert(manifest.turn, manifest);
        }
    }
    turns
}

/// The newest turn with changes not undone.
pub(crate) fn last_open(turns: &BTreeMap<u32, Manifest>) -> Option<&Manifest> {
    turns
        .values()
        .rev()
        .find(|m| !m.files.is_empty() && !m.undone)
}

fn session_bytes(turns: &BTreeMap<u32, Manifest>) -> u64 {
    turns.values().map(|m| m.bytes).sum()
}

/// `atomic::replace` for `write` and `edit`: the session's undo store keeps
/// the prior state first and the hash of `bytes` after.
pub(crate) fn tracked_replace(
    ctx: &super::ToolCtx<'_>,
    path: &Path,
    bytes: &[u8],
) -> std::io::Result<()> {
    let undo = ctx.session.map(|session| session.undo());
    if let Some(undo) = undo {
        undo.before_change(path);
    }
    super::atomic::replace(path, bytes)?;
    if let Some(undo) = undo {
        undo.after_change(path, bytes);
    }
    Ok(())
}

/// FNV-1a 64 of `bytes` with their length: enough to see that a file
/// changed since the turn (not a defence against a crafted collision).
pub(crate) fn hash(bytes: &[u8]) -> String {
    let mut value: u64 = 0xcbf2_9ce4_8422_2325;
    for byte in bytes {
        value ^= u64::from(*byte);
        value = value.wrapping_mul(0x0100_0000_01b3);
    }
    format!("fnv1a64:{value:016x}:{}", bytes.len())
}
