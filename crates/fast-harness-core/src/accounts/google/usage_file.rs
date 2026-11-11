//! `<app_home>/google-usage.json` (version 1): the last quota read of each
//! Google account, so the Accounts page shows it after a restart. Written
//! temp file then rename; a missing, corrupt or other-version file reads as empty.

use super::quota::{GoogleQuota, QuotaGroup, QuotaWindow};
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, HashMap};
use std::fs;
use std::io;
use std::path::{Path, PathBuf};

pub const USAGE_FILE: &str = "google-usage.json";
pub const USAGE_VERSION: u64 = 1;

/// One quota read.
#[derive(Clone, Debug, PartialEq)]
pub struct QuotaRead {
    pub quota: GoogleQuota,
    /// Unix milliseconds of the read.
    pub read_at: u64,
}

#[derive(Serialize, Deserialize)]
struct FileJson {
    version: u64,
    accounts: BTreeMap<String, ReadJson>,
}

#[derive(Serialize, Deserialize)]
struct ReadJson {
    #[serde(rename = "readAt")]
    read_at: u64,
    groups: Vec<GroupJson>,
}

#[derive(Serialize, Deserialize)]
struct GroupJson {
    name: String,
    #[serde(rename = "fiveHour", default, skip_serializing_if = "Option::is_none")]
    five_hour: Option<WindowJson>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    weekly: Option<WindowJson>,
}

#[derive(Serialize, Deserialize)]
struct WindowJson {
    used: f64,
    #[serde(rename = "resetsAt", default, skip_serializing_if = "Option::is_none")]
    resets_at: Option<u64>,
    #[serde(default)]
    disabled: bool,
}

/// The file path under `app_home`.
pub fn usage_path(app_home: &Path) -> PathBuf {
    app_home.join(USAGE_FILE)
}

fn window_json(w: &QuotaWindow) -> WindowJson {
    WindowJson {
        used: w.used,
        resets_at: w.resets_at,
        disabled: w.disabled,
    }
}

fn window_of(w: WindowJson) -> QuotaWindow {
    let used = if w.used.is_finite() {
        w.used.clamp(0.0, 1.0)
    } else {
        1.0
    };
    QuotaWindow {
        used,
        resets_at: w.resets_at,
        disabled: w.disabled,
    }
}

fn read_json(read: &QuotaRead) -> ReadJson {
    let groups = read.quota.groups.iter().map(|g| GroupJson {
        name: g.name.clone(),
        five_hour: g.five_hour.as_ref().map(window_json),
        weekly: g.weekly.as_ref().map(window_json),
    });
    ReadJson {
        read_at: read.read_at,
        groups: groups.collect(),
    }
}

fn read_of(json: ReadJson) -> QuotaRead {
    let groups = json.groups.into_iter().map(|g| QuotaGroup {
        name: g.name,
        five_hour: g.five_hour.map(window_of),
        weekly: g.weekly.map(window_of),
    });
    QuotaRead {
        quota: GoogleQuota {
            groups: groups.collect(),
        },
        read_at: json.read_at,
    }
}

/// The reads in `path`; empty when the file is missing, corrupt or of another version.
pub fn load_usage(path: &Path) -> HashMap<String, QuotaRead> {
    let Ok(text) = fs::read_to_string(path) else {
        return HashMap::new();
    };
    match serde_json::from_str::<FileJson>(&text) {
        Ok(file) if file.version == USAGE_VERSION => file
            .accounts
            .into_iter()
            .map(|(id, read)| (id, read_of(read)))
            .collect(),
        _ => HashMap::new(),
    }
}

/// Writes `reads` to `path` (temp file, then rename).
pub fn save_usage(path: &Path, reads: &HashMap<String, QuotaRead>) -> io::Result<()> {
    let file = FileJson {
        version: USAGE_VERSION,
        accounts: reads
            .iter()
            .map(|(id, r)| (id.clone(), read_json(r)))
            .collect(),
    };
    let text = serde_json::to_string_pretty(&file).map_err(io::Error::other)?;
    if let Some(dir) = path.parent() {
        fs::create_dir_all(dir)?;
    }
    let temp = path.with_extension("json.tmp");
    fs::write(&temp, text)?;
    fs::rename(&temp, path)
}
