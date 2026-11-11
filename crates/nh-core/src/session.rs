//! Session ids, paths, listing and resume by replaying the log (chunk 1C).

use std::path::{Path, PathBuf};

#[derive(Debug)]
pub struct SessionSummary {
    pub id: String,
    pub created_at: u64,
    pub title: String,
    pub turns: u64,
    pub last_time: u64,
}

pub fn new_id() -> String {
    // A simplified sortable ID: <yyyymmdd-hhmmss>-<counter>
    // For now using just a placeholder as I cannot add a new crate like `time`.
    // I need to use std::time or similar.
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs();

    // I will use a simple format for now.
    format!("{:x}-0000", now)
}

pub fn slug(cwd: &Path) -> String {
    let s = cwd.to_string_lossy();
    let slug: String = s
        .chars()
        .map(|c| {
            if c == ':' || c == '\\' || c == '/' {
                '-'
            } else {
                c
            }
        })
        .collect();
    format!("--{}--", slug)
}

pub fn dir(nh_home: &Path, cwd: &Path) -> PathBuf {
    nh_home.join("sessions").join(slug(cwd))
}

pub fn log_path(nh_home: &Path, cwd: &Path, id: &str) -> PathBuf {
    dir(nh_home, cwd).join(format!("{}.jsonl", id))
}

pub fn wire_dir(nh_home: &Path, cwd: &Path, id: &str) -> PathBuf {
    dir(nh_home, cwd).join(format!("{}.wire", id))
}
