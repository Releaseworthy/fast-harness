//! `<nh_home>/usage.json`: the last usage read of each account, so a restart
//! keeps the switch order. Written at most every [`SAVE_GAP_MS`] unless forced.
//!
//! Format: `{"version":1,"accounts":{"<id>":{"usage":{...},"readAt":<unix ms>}}}`.

use crate::accounts::usage::Usage;
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::fs;
use std::io;
use std::path::PathBuf;

/// Shortest gap between two unforced writes (claude-switch `USAGE_SAVE_GAP_MS`).
pub(super) const SAVE_GAP_MS: u64 = 2_000;
/// An entry older than this is not restored (claude-switch `USAGE_SNAPSHOT_MAX_AGE_MS`).
const MAX_AGE_MS: u64 = 7 * 24 * 60 * 60_000;
const VERSION: u64 = 1;

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(super) struct Entry {
    pub usage: Usage,
    /// Unix milliseconds of the read.
    pub read_at: u64,
}

#[derive(Serialize, Deserialize)]
struct FileData {
    version: u64,
    #[serde(default)]
    accounts: BTreeMap<String, Entry>,
}

pub(super) struct Snapshot {
    path: PathBuf,
    pub entries: BTreeMap<String, Entry>,
    last_save: u64,
}

impl Snapshot {
    /// Reads `path`; a missing, unreadable or foreign file starts empty.
    pub fn load(path: PathBuf, now: u64) -> Snapshot {
        let mut entries = fs::read_to_string(&path)
            .ok()
            .and_then(|text| serde_json::from_str::<FileData>(&text).ok())
            .filter(|data| data.version == VERSION)
            .map(|data| data.accounts)
            .unwrap_or_default();
        entries.retain(|_, entry| now.saturating_sub(entry.read_at) < MAX_AGE_MS);
        Snapshot {
            path,
            entries,
            last_save: 0,
        }
    }

    pub fn usage(&self, id: &str) -> Option<&Usage> {
        self.entries.get(id).map(|entry| &entry.usage)
    }

    pub fn read_at(&self, id: &str) -> Option<u64> {
        self.entries.get(id).map(|entry| entry.read_at)
    }

    pub fn set(&mut self, id: &str, usage: Usage, now: u64) {
        let entry = Entry {
            usage,
            read_at: now,
        };
        self.entries.insert(id.to_string(), entry);
    }

    pub fn remove(&mut self, id: &str) {
        self.entries.remove(id);
    }

    /// Writes the file (temporary file, then rename) unless the last write was
    /// less than [`SAVE_GAP_MS`] ago and `force` is false.
    pub fn save(&mut self, now: u64, force: bool) -> io::Result<()> {
        if !force && self.last_save != 0 && now.saturating_sub(self.last_save) < SAVE_GAP_MS {
            return Ok(());
        }
        let data = FileData {
            version: VERSION,
            accounts: self.entries.clone(),
        };
        let text = serde_json::to_string_pretty(&data).map_err(io::Error::other)?;
        if let Some(dir) = self.path.parent().filter(|d| !d.as_os_str().is_empty()) {
            fs::create_dir_all(dir)?;
        }
        let mut tmp = self.path.clone().into_os_string();
        tmp.push(format!(".{}.tmp", std::process::id()));
        let tmp = PathBuf::from(tmp);
        if let Err(err) = fs::write(&tmp, text).and_then(|()| fs::rename(&tmp, &self.path)) {
            let _ = fs::remove_file(&tmp);
            return Err(err);
        }
        self.last_save = now;
        Ok(())
    }
}
