//! A small unified diff applier for reflect proposals (PLAN.md D50). Paths
//! under `memory/` land in the workspace memory folder, every other path is
//! relative to the workspace; absolute paths and `..` are refused. Hunks
//! are placed by their old lines (context and removals), so line numbers
//! that drifted still apply. Every file is checked before any is written.

use std::fs;
use std::path::{Component, Path, PathBuf};

/// The prefix of diff paths that name the memory folder.
pub const MEMORY_PREFIX: &str = "memory/";

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Hunk {
    /// 1-based first old line (`@@ -l,n`), used when `old` is empty.
    pub start: usize,
    pub old: Vec<String>,
    pub new: Vec<String>,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct FilePatch {
    /// The path as the diff names it (`a/` and `b/` removed).
    pub path: String,
    pub hunks: Vec<Hunk>,
}

impl FilePatch {
    /// True for a file of the memory folder.
    pub fn is_memory(&self) -> bool {
        self.path.starts_with(MEMORY_PREFIX)
    }
}

fn header_path(line: &str) -> String {
    let path = line.split('\t').next().unwrap_or("").trim();
    let path = path.strip_prefix("a/").or_else(|| path.strip_prefix("b/"));
    path.unwrap_or(line.split('\t').next().unwrap_or("").trim())
        .to_string()
}

/// Parses `diff` into file patches; `Err` names the first bad line.
pub fn parse(diff: &str) -> Result<Vec<FilePatch>, String> {
    let mut files: Vec<FilePatch> = Vec::new();
    let mut old_path = String::new();
    for line in diff.lines() {
        if let Some(rest) = line.strip_prefix("--- ") {
            old_path = header_path(rest);
        } else if let Some(rest) = line.strip_prefix("+++ ") {
            let mut path = header_path(rest);
            if path == "/dev/null" {
                path = old_path.clone();
            }
            files.push(FilePatch {
                path,
                hunks: Vec::new(),
            });
        } else if let Some(rest) = line.strip_prefix("@@ -") {
            let file = files
                .last_mut()
                .ok_or(format!("hunk before a file header: {line}"))?;
            let start = rest.split([',', ' ']).next().unwrap_or("1");
            let start = start
                .parse()
                .map_err(|_| format!("bad hunk header: {line}"))?;
            file.hunks.push(Hunk {
                start,
                ..Hunk::default()
            });
        } else if let Some(hunk) = files.last_mut().and_then(|f| f.hunks.last_mut()) {
            body_line(hunk, line);
        }
    }
    Ok(files)
}

fn body_line(hunk: &mut Hunk, line: &str) {
    if let Some(text) = line.strip_prefix('+') {
        hunk.new.push(text.to_string());
    } else if let Some(text) = line.strip_prefix('-') {
        hunk.old.push(text.to_string());
    } else if let Some(text) = line.strip_prefix(' ') {
        hunk.old.push(text.to_string());
        hunk.new.push(text.to_string());
    } else if line.is_empty() {
        hunk.old.push(String::new());
        hunk.new.push(String::new());
    }
}

/// Where `path` of a diff lands, or why it may not.
pub fn target(path: &str, cwd: &Path, memory_dir: &Path) -> Result<PathBuf, String> {
    let (root, rel) = match path.strip_prefix(MEMORY_PREFIX) {
        Some(rel) => (memory_dir, rel),
        None => (cwd, path),
    };
    let rel_path = Path::new(rel);
    let plain = rel_path
        .components()
        .all(|c| matches!(c, Component::Normal(_)));
    if rel.is_empty() || !plain {
        return Err(format!("{path}: only plain relative paths are applied"));
    }
    Ok(root.join(rel_path))
}

/// `text` with `patch`'s hunks applied.
pub fn apply_text(text: &str, patch: &FilePatch) -> Result<String, String> {
    let mut lines: Vec<String> = text.lines().map(str::to_string).collect();
    let mut cursor = 0;
    for hunk in &patch.hunks {
        let at = if hunk.old.is_empty() {
            hunk.start.min(lines.len())
        } else {
            find(&lines, &hunk.old, cursor)
                .or_else(|| find(&lines, &hunk.old, 0))
                .ok_or_else(|| format!("{}: a hunk does not match the file", patch.path))?
        };
        lines.splice(at..at + hunk.old.len(), hunk.new.iter().cloned());
        cursor = at + hunk.new.len();
    }
    let mut out = lines.join("\n");
    if !out.is_empty() {
        out.push('\n');
    }
    Ok(out)
}

fn find(lines: &[String], old: &[String], from: usize) -> Option<usize> {
    if old.len() > lines.len() {
        return None;
    }
    (from..=lines.len() - old.len()).find(|&i| lines[i..i + old.len()] == *old)
}

/// Applies the patches of `diff` that `keep` accepts; returns the paths
/// written. Nothing is written when one of them fails.
pub fn apply(
    diff: &str,
    cwd: &Path,
    memory_dir: &Path,
    keep: impl Fn(&FilePatch) -> bool,
) -> Result<Vec<PathBuf>, String> {
    let mut planned = Vec::new();
    for patch in parse(diff)?.iter().filter(|p| keep(p)) {
        let path = target(&patch.path, cwd, memory_dir)?;
        let text = fs::read_to_string(&path).unwrap_or_default();
        planned.push((path, apply_text(&text, patch)?));
    }
    for (path, text) in &planned {
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent).map_err(|err| format!("{}: {err}", parent.display()))?;
        }
        fs::write(path, text).map_err(|err| format!("{}: {err}", path.display()))?;
    }
    Ok(planned.into_iter().map(|(path, _)| path).collect())
}
