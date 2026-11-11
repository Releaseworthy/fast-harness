//! The file walk shared by grep and glob: `.gitignore` aware (inside a git
//! repo or not), hidden files skipped, optional glob filter.
//!
//! The glob is applied as a filter after the ignore rules, not as an
//! `ignore` override, because an override whitelist would beat `.gitignore`.

use std::path::Path;

use ignore::overrides::{Override, OverrideBuilder};
use ignore::{DirEntry, WalkBuilder};

/// Files under `root` (sorted by name per directory), keeping only those
/// matching `glob` when given (gitignore glob syntax, relative to `root`).
pub(crate) fn files(
    root: &Path,
    glob: Option<&str>,
) -> Result<impl Iterator<Item = DirEntry>, String> {
    // A single file named as the root is searched whatever the glob says.
    let filter = match glob {
        Some(glob) if !root.is_file() => Some(build_filter(root, glob)?),
        _ => None,
    };
    let mut builder = WalkBuilder::new(root);
    builder
        .hidden(true)
        .git_ignore(true)
        .git_exclude(true)
        .git_global(false)
        .require_git(false)
        .sort_by_file_name(|a, b| a.cmp(b));
    let walk = builder.build().flatten().filter(move |entry| {
        let is_file = entry.file_type().is_some_and(|kind| kind.is_file());
        is_file
            && filter
                .as_ref()
                .is_none_or(|filter| filter.matched(entry.path(), false).is_whitelist())
    });
    Ok(walk)
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
