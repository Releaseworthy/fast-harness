//! The file walk shared by grep and glob: `.gitignore` aware (inside a git
//! repo or not), hidden files skipped unless asked for, optional glob
//! filter, and a count of what the walk left out so an empty result does
//! not read as "nothing there".
//!
//! The glob is applied as a filter after the ignore rules, not as an
//! `ignore` override, because an override whitelist would beat `.gitignore`.

use std::collections::HashSet;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};

use ignore::overrides::{Override, OverrideBuilder};
use ignore::{DirEntry, WalkBuilder};

/// What a walk takes in beyond the default.
#[derive(Clone, Copy, Debug, Default)]
pub(crate) struct Include {
    /// Hidden files and folders (a name starting with `.`).
    pub hidden: bool,
    /// Files and folders `.gitignore` (or `.ignore`) leaves out.
    pub ignored: bool,
}

/// What the walk left out, one level deep: a left-out folder counts once.
#[derive(Debug, Default)]
pub(crate) struct Skipped {
    pub ignored_files: usize,
    pub ignored_dirs: usize,
    pub hidden: usize,
}

pub(crate) struct Walk {
    /// The files kept, sorted by name per directory.
    pub files: Vec<DirEntry>,
    pub skipped: Skipped,
}

/// Files under `root`, keeping only those matching `glob` when given
/// (gitignore glob syntax, relative to `root`).
pub(crate) fn walk(
    root: &Path,
    glob: Option<&str>,
    include: Include,
    cancel: &AtomicBool,
) -> Result<Walk, String> {
    // A single file named as the root is searched whatever the glob says.
    let filter = match glob {
        Some(glob) if !root.is_file() => Some(build_filter(root, glob)?),
        _ => None,
    };
    let mut builder = WalkBuilder::new(root);
    builder
        .hidden(!include.hidden)
        .ignore(!include.ignored)
        .git_ignore(!include.ignored)
        .git_exclude(!include.ignored)
        .git_global(false)
        .require_git(false)
        .sort_by_file_name(|a, b| a.cmp(b));
    let (mut files, mut dirs, mut seen) = (Vec::new(), Vec::new(), HashSet::new());
    for entry in builder.build().flatten() {
        if cancel.load(Ordering::Relaxed) {
            return Err("cancelled".to_string());
        }
        seen.insert(entry.path().to_path_buf());
        let is_file = entry.file_type().is_some_and(|kind| kind.is_file());
        if entry.file_type().is_some_and(|kind| kind.is_dir()) {
            dirs.push(entry.path().to_path_buf());
        }
        if is_file && matches(filter.as_ref(), entry.path()) {
            files.push(entry);
        }
    }
    let skipped = skipped(&dirs, &seen, filter.as_ref());
    Ok(Walk { files, skipped })
}

fn matches(filter: Option<&Override>, path: &Path) -> bool {
    filter.is_none_or(|filter| filter.matched(path, false).is_whitelist())
}

/// The children of the walked folders the walk did not visit, `.git`
/// aside; a left-out file counts only when it matches the glob.
fn skipped(dirs: &[PathBuf], seen: &HashSet<PathBuf>, filter: Option<&Override>) -> Skipped {
    let mut out = Skipped::default();
    for dir in dirs {
        let Ok(children) = std::fs::read_dir(dir) else {
            continue;
        };
        for child in children.flatten() {
            let path = child.path();
            let name = child.file_name();
            if seen.contains(&path) || name == ".git" {
                continue;
            }
            let is_dir = child.file_type().is_ok_and(|kind| kind.is_dir());
            if !is_dir && !matches(filter, &path) {
                continue;
            }
            match (name.to_string_lossy().starts_with('.'), is_dir) {
                (true, _) => out.hidden += 1,
                (false, true) => out.ignored_dirs += 1,
                (false, false) => out.ignored_files += 1,
            }
        }
    }
    out
}

impl Skipped {
    /// A bracketed note of what was left out and how to include it, with
    /// `binary` files grep passed over; `None` when nothing was.
    pub(crate) fn note(&self, binary: usize) -> Option<String> {
        let mut parts = Vec::new();
        let mut ignored = Vec::new();
        if self.ignored_files > 0 {
            ignored.push(count(self.ignored_files, "file", "files"));
        }
        if self.ignored_dirs > 0 {
            ignored.push(count(self.ignored_dirs, "folder", "folders"));
        }
        if !ignored.is_empty() {
            let what = ignored.join(" and ");
            parts.push(format!(
                "{what} skipped by .gitignore (include_ignored: true)"
            ));
        }
        if self.hidden > 0 {
            let what = count(self.hidden, "hidden entry", "hidden entries");
            parts.push(format!("{what} skipped (include_hidden: true)"));
        }
        if binary > 0 {
            let what = count(binary, "binary file", "binary files");
            parts.push(format!("{what} not searched"));
        }
        (!parts.is_empty()).then(|| format!("[{}]", parts.join("; ")))
    }
}

fn count(n: usize, one: &str, many: &str) -> String {
    match n {
        1 => format!("1 {one}"),
        n => format!("{n} {many}"),
    }
}

fn build_filter(root: &Path, glob: &str) -> Result<Override, String> {
    let mut builder = OverrideBuilder::new(root);
    builder
        .add(glob)
        .map_err(|err| format!("invalid glob `{glob}`: {err}"))?;
    builder
        .build()
        .map_err(|err| format!("invalid glob `{glob}`: {err}"))
}
