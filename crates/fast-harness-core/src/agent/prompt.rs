//! The system prompt: one identity line, one tool line and the AGENTS.md
//! files of the session's project (from the repository root down to the
//! working directory, never above it).

use std::path::{Path, PathBuf};

use crate::session;
use crate::trace;

/// Largest AGENTS.md text put into the prompt, all files together.
pub const MAX_AGENTS_BYTES: usize = 32 * 1024;

const TOOL_LINE: &str = "Use the tools to read, search and change files and to run commands; \
independent calls in one step run in parallel. Be brief.";

/// The full system prompt for an agent working in `cwd`.
pub fn system_prompt(cwd: &Path) -> String {
    let mut out = format!(
        "You are a coding agent in Fast Harness, working in {} on {}. Today is {} (UTC).\n{TOOL_LINE}\n",
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

/// The directories whose AGENTS.md belongs to the session, outermost first:
/// from the nearest ancestor of `cwd` (itself included) holding a `.git`
/// entry (directory or file) down to `cwd`; only `cwd` without one.
pub fn project_dirs(cwd: &Path) -> Vec<&Path> {
    let mut dirs = Vec::new();
    for dir in cwd.ancestors() {
        dirs.push(dir);
        if std::fs::symlink_metadata(dir.join(".git")).is_ok() {
            dirs.reverse();
            return dirs;
        }
    }
    vec![cwd]
}

/// `(path, text)` of every readable `AGENTS.md` of [`project_dirs`], outermost first.
pub fn agents_files(cwd: &Path) -> Vec<(PathBuf, String)> {
    let dirs = project_dirs(cwd);
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
