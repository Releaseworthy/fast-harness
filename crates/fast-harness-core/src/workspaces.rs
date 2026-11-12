//! The workspace list: folders the user works in (PLAN.md D28, docs/formats.md "Files on disk").
//! `<app_home>/workspaces.json` =
//! `{"version":1,"workspaces":[{"path":"F:\\work\\x","addedAt":<unix ms>,"lastUsedAt":<unix ms>}]}`.
//! A missing file is seeded from the session folders; removing a workspace never touches files.

use crate::session;
use crate::store;
use serde::{Deserialize, Serialize};
use std::fmt;
use std::io;
use std::path::{Path, PathBuf};

pub const VERSION: u32 = 1;
pub const FILE_NAME: &str = "workspaces.json";

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Workspace {
    pub path: PathBuf,
    /// Unix milliseconds.
    pub added_at: u64,
    /// Unix milliseconds.
    pub last_used_at: u64,
}

#[derive(Debug)]
pub enum WorkspacesError {
    Io(io::Error),
    Json(serde_json::Error),
    Version(u32),
}

impl fmt::Display for WorkspacesError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            WorkspacesError::Io(err) => write!(f, "workspaces.json: {err}"),
            WorkspacesError::Json(err) => write!(f, "workspaces.json is not valid: {err}"),
            WorkspacesError::Version(v) => write!(f, "workspaces.json has unknown version {v}"),
        }
    }
}

impl std::error::Error for WorkspacesError {}

impl From<io::Error> for WorkspacesError {
    fn from(err: io::Error) -> Self {
        WorkspacesError::Io(err)
    }
}

#[derive(Serialize, Deserialize)]
struct FileData {
    version: u32,
    workspaces: Vec<Workspace>,
}

/// The list and the file it lives in.
#[derive(Clone, Debug)]
pub struct Workspaces {
    path: PathBuf,
    entries: Vec<Workspace>,
    /// The session slugs the file held at the load: what [Workspaces::save]
    /// compares against to find this process's additions and removals.
    base: Vec<String>,
}

impl Workspaces {
    /// Reads `<app_home>/workspaces.json`. A missing file gives the list seeded from
    /// every session folder (not saved yet); a corrupt file or another version is an error.
    pub fn load(app_home: &Path) -> Result<Workspaces, WorkspacesError> {
        let path = app_home.join(FILE_NAME);
        let text = match crate::store::read_to_string(&path) {
            Ok(text) => text,
            Err(err) if err.kind() == io::ErrorKind::NotFound => {
                return Ok(Workspaces::seed(app_home, path)?);
            }
            Err(err) => return Err(err.into()),
        };
        let data: FileData = serde_json::from_str(&text).map_err(WorkspacesError::Json)?;
        if data.version != VERSION {
            return Err(WorkspacesError::Version(data.version));
        }
        let mut list = Workspaces {
            path,
            entries: Vec::new(),
            base: Vec::new(),
        };
        for entry in data.workspaces {
            list.insert(entry);
        }
        list.base = list
            .entries
            .iter()
            .map(|w| session::slug(&w.path))
            .collect();
        Ok(list)
    }

    /// An empty list for `<app_home>/workspaces.json` (used when the file cannot be read).
    pub fn empty(app_home: &Path) -> Workspaces {
        Workspaces {
            path: app_home.join(FILE_NAME),
            entries: Vec::new(),
            base: Vec::new(),
        }
    }

    fn seed(app_home: &Path, path: PathBuf) -> io::Result<Workspaces> {
        let mut list = Workspaces {
            path,
            entries: Vec::new(),
            base: Vec::new(),
        };
        for known in session::known_cwds(app_home)? {
            list.insert(Workspace {
                path: known.cwd,
                added_at: known.first,
                last_used_at: known.last,
            });
        }
        Ok(list)
    }

    /// Adds `entry` unless a workspace with the same session slug exists; then the
    /// existing one is kept, its `lastUsedAt` raised to the newer of the two.
    fn insert(&mut self, entry: Workspace) {
        let slug = session::slug(&entry.path);
        match self
            .entries
            .iter_mut()
            .find(|w| session::slug(&w.path) == slug)
        {
            Some(old) => old.last_used_at = old.last_used_at.max(entry.last_used_at),
            None => self.entries.push(entry),
        }
    }

    /// Writes this process's changes into the file's current content, under
    /// its lock (`store::update`, PLAN.md D55): a folder added since the load
    /// is added, a removed one removed, `lastUsedAt` takes the newer value,
    /// and folders another process added or removed stay so. A corrupt file
    /// or another version is an error and stays untouched.
    pub fn save(&self) -> Result<(), WorkspacesError> {
        let mut failure = None;
        let saved = store::update(&self.path, store::WAIT, |current| {
            let merged = self.merged(current).map_err(|err| {
                failure = Some(err);
                io::Error::other("workspaces.json cannot be merged")
            })?;
            let data = FileData {
                version: VERSION,
                workspaces: merged.list(),
            };
            let mut text = serde_json::to_string_pretty(&data).map_err(io::Error::other)?;
            text.push('\n');
            Ok((text.into_bytes(), ()))
        });
        match (saved, failure) {
            (_, Some(err)) => Err(err),
            (saved, None) => Ok(saved?),
        }
    }

    /// The file's list (`current`, `None` when missing) with this process's
    /// changes since the load applied.
    fn merged(&self, current: Option<Vec<u8>>) -> Result<Workspaces, WorkspacesError> {
        let mut merged = Workspaces {
            path: self.path.clone(),
            entries: Vec::new(),
            base: Vec::new(),
        };
        if let Some(bytes) = current {
            let data: FileData = serde_json::from_slice(&bytes).map_err(WorkspacesError::Json)?;
            if data.version != VERSION {
                return Err(WorkspacesError::Version(data.version));
            }
            data.workspaces.into_iter().for_each(|w| merged.insert(w));
        }
        for gone in self.base.iter().filter(|s| !self.has_slug(s)) {
            merged.entries.retain(|w| session::slug(&w.path) != *gone);
        }
        for entry in &self.entries {
            let slug = session::slug(&entry.path);
            if !self.base.contains(&slug) || merged.has_slug(&slug) {
                merged.insert(entry.clone());
            }
        }
        Ok(merged)
    }

    fn has_slug(&self, slug: &str) -> bool {
        self.entries.iter().any(|w| session::slug(&w.path) == slug)
    }

    /// Adds `path` at time `now`. Returns false when it was already listed (kept as is).
    pub fn add(&mut self, path: &Path, now: u64) -> bool {
        if self.find(path).is_some() {
            return false;
        }
        self.insert(Workspace {
            path: path.to_path_buf(),
            added_at: now,
            last_used_at: now,
        });
        true
    }

    /// Removes `path` from the list only. Returns whether it was listed.
    pub fn remove(&mut self, path: &Path) -> bool {
        let slug = session::slug(path);
        let before = self.entries.len();
        self.entries.retain(|w| session::slug(&w.path) != slug);
        self.entries.len() != before
    }

    /// Sets `lastUsedAt` of `path` to `now`. Returns whether it was listed.
    pub fn touch(&mut self, path: &Path, now: u64) -> bool {
        let slug = session::slug(path);
        match self
            .entries
            .iter_mut()
            .find(|w| session::slug(&w.path) == slug)
        {
            Some(entry) => {
                entry.last_used_at = now;
                true
            }
            None => false,
        }
    }

    /// The entry for `path` (same session slug), if listed.
    pub fn find(&self, path: &Path) -> Option<&Workspace> {
        let slug = session::slug(path);
        self.entries.iter().find(|w| session::slug(&w.path) == slug)
    }

    /// Every workspace in `addedAt` order (ties by path).
    pub fn list(&self) -> Vec<Workspace> {
        let mut list = self.entries.clone();
        list.sort_by(|a, b| (a.added_at, &a.path).cmp(&(b.added_at, &b.path)));
        list
    }

    /// The most recently used workspace.
    pub fn most_recent(&self) -> Option<&Workspace> {
        self.entries.iter().max_by_key(|w| w.last_used_at)
    }
}
