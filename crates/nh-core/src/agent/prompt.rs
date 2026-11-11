//! The system prompt: identity, date, tool guidance and every AGENTS.md from
//! the filesystem root down to the working directory.

use std::path::{Path, PathBuf};

use crate::session;
use crate::trace;

/// Largest AGENTS.md text put into the prompt, all files together.
pub const MAX_AGENTS_BYTES: usize = 32 * 1024;

const TOOL_GUIDE: &str = "\
Tools:
- read, grep and glob to look before you change anything; read only the ranges you need.
- edit for targeted changes to an existing file, write for new files or full rewrites.
- shell for builds, tests and git; commands run in the working directory.
- subagent for a self-contained task that would fill this conversation with detail.
Independent tool calls in one step run in parallel.
Keep answers short; report only what you observed.";

/// The full system prompt for an agent working in `cwd`.
pub fn system_prompt(cwd: &Path) -> String {
    let mut out = format!(
        "You are nh, a coding agent working in {} on {}.\nToday is {} (UTC).\n\n{TOOL_GUIDE}\n",
        cwd.display(),
        os_name(),
        today()
    );
    let agents = agents_section(cwd);
    if !agents.is_empty() {
        out.push_str("\n# Project instructions (AGENTS.md)\n\n");
        out.push_str(&agents);
    }
    out
}

/// `(path, text)` of every readable `AGENTS.md` from the root down to `cwd`.
pub fn agents_files(cwd: &Path) -> Vec<(PathBuf, String)> {
    let mut dirs: Vec<&Path> = cwd.ancestors().collect();
    dirs.reverse();
    dirs.into_iter()
        .filter_map(|dir| {
            let path = dir.join("AGENTS.md");
            let text = std::fs::read_to_string(&path).ok()?;
            Some((path, text))
        })
        .collect()
}

fn agents_section(cwd: &Path) -> String {
    let mut out = String::new();
    for (path, text) in agents_files(cwd) {
        out.push_str(&format!("## {}\n\n{}\n\n", path.display(), text.trim_end()));
    }
    cap(out, MAX_AGENTS_BYTES)
}

fn cap(mut text: String, max: usize) -> String {
    if text.len() <= max {
        return text;
    }
    let mut end = max;
    while !text.is_char_boundary(end) {
        end -= 1;
    }
    text.truncate(end);
    text.push_str("\n[AGENTS.md text cut at 32 KB]\n");
    text
}

fn os_name() -> &'static str {
    match std::env::consts::OS {
        "windows" => "Windows",
        "macos" => "macOS",
        "linux" => "Linux",
        "android" => "Android",
        "ios" => "iOS",
        other => other,
    }
}

/// `yyyy-mm-dd` in UTC.
fn today() -> String {
    let stamp = session::utc_stamp(trace::now_ms() / 1000);
    format!("{}-{}-{}", &stamp[0..4], &stamp[4..6], &stamp[6..8])
}
