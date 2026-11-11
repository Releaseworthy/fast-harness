//! The workspace list: folders the user works in (PLAN.md D28, "Files on disk").
//! `<app_home>/workspaces.json` =
//! `{"version":1,"workspaces":[{"path":"F:\\work\\x","addedAt":<unix ms>,"lastUsedAt":<unix ms>}]}`.
//! A missing file is seeded from the session folders; removing a workspace never touches files.

use crate::session;
use serde::{Deserialize, Serialize};
use std::fmt;
use std::fs;
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
}

impl Workspaces {
    /// Reads `<app_home>/workspaces.json`. A missing file gives the list seeded from
    /// every session folder (not saved yet); a corrupt file or another version is an error.
    pub fn load(app_home: &Path) -> Result<Workspaces, WorkspacesError> {
        let path = app_home.join(FILE_NAME);
        let text = match fs::read_to_string(&path) {
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
        };
        for entry in data.workspaces {
            list.insert(entry);
        }
        Ok(list)
    }

    fn seed(app_home: &Path, path: PathBuf) -> io::Result<Workspaces> {
        let mut list = Workspaces {
            path,
            entries: Vec::new(),
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

    /// Writes the list: a temporary file in the same folder, renamed over the old one.
    pub fn save(&self) -> Result<(), WorkspacesError> {
        let data = FileData {
            version: VERSION,
            workspaces: self.list(),
        };
        let mut text = serde_json::to_string_pretty(&data).map_err(WorkspacesError::Json)?;
        text.push('\n');
        if let Some(dir) = self.path.parent().filter(|d| !d.as_os_str().is_empty()) {
            fs::create_dir_all(dir)?;
        }
        let mut name = self.path.file_name().unwrap_or_default().to_os_string();
        name.push(format!(".{}.tmp", std::process::id()));
        let tmp = self.path.with_file_name(name);
        if let Err(err) = fs::write(&tmp, text).and_then(|()| fs::rename(&tmp, &self.path)) {
            let _ = fs::remove_file(&tmp);
            return Err(err.into());
        }
        Ok(())
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
