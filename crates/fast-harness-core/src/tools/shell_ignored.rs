//! Which folders `.gitignore` leaves out among the paths a recursive
//! listing printed (D89): the shell hint that points at `glob`. A path is
//! judged in its own repository, so a listing of another folder (`cd` or
//! `-Path` elsewhere) counts too (D91).

use std::collections::HashMap;
use std::path::{Component, Path, PathBuf};

use ignore::gitignore::Gitignore;

/// Output lines looked at; a listing longer than this is judged by its start.
const MAX_LINES: usize = 20_000;
/// Ignored folders named in the hint.
const MAX_NAMED: usize = 3;

/// The `.gitignore` files met so far, by folder, the folders judged and
/// the repository root of each folder asked about.
#[derive(Default)]
struct Ignores {
    files: HashMap<PathBuf, Option<Gitignore>>,
    judged: HashMap<PathBuf, bool>,
    tops: HashMap<PathBuf, Option<PathBuf>>,
}

impl Ignores {
    /// The nearest folder at or above `dir` holding `.git`.
    fn repo_of(&mut self, dir: &Path) -> Option<PathBuf> {
        if let Some(known) = self.tops.get(dir) {
            return known.clone();
        }
        let top = (dir.ancestors())
            .find(|dir| dir.join(".git").exists())
            .map(Path::to_path_buf);
        self.tops.insert(dir.to_path_buf(), top.clone());
        top
    }

    /// Does a `.gitignore` in a folder from `top` down to the parent of
    /// `dir` leave folder `dir` out?
    fn ignored(&mut self, top: &Path, dir: &Path) -> bool {
        if let Some(known) = self.judged.get(dir) {
            return *known;
        }
        let parents: Vec<PathBuf> = dir
            .ancestors()
            .skip(1)
            .take_while(|parent| parent.starts_with(top))
            .map(Path::to_path_buf)
            .collect();
        let mut hit = false;
        for parent in parents {
            let matcher = self.files.entry(parent.clone()).or_insert_with(|| {
                let file = parent.join(".gitignore");
                file.is_file().then(|| Gitignore::new(file).0)
            });
            if let Some(matcher) = matcher {
                hit |= matcher.matched(dir, true).is_ignore();
            }
        }
        self.judged.insert(dir.to_path_buf(), hit);
        hit
    }
}

/// Up to [`MAX_NAMED`] folders that `.gitignore` leaves out and that the
/// listing `output` went into: relative to `cwd` under it, else whole.
pub(super) fn ignored_dirs(output: &str, cwd: &Path) -> Vec<String> {
    let mut ignores = Ignores::default();
    let mut named: Vec<String> = Vec::new();
    for line in output.lines().take(MAX_LINES) {
        let Some((path, is_dir)) = listed_path(line) else {
            continue;
        };
        let Some(dir) = first_ignored(&mut ignores, cwd, &cwd.join(path), is_dir) else {
            continue;
        };
        let name = dir.strip_prefix(cwd).unwrap_or(&dir).display().to_string();
        if !named.contains(&name) {
            named.push(name);
        }
        if named.len() == MAX_NAMED {
            break;
        }
    }
    named
}

/// The path a listing line names: a `Directory: <path>` header (a
/// folder), or a whole line with a separator and no table columns.
pub(super) fn listed_path(line: &str) -> Option<(&str, bool)> {
    let line = line.trim();
    if let Some(dir) = line.strip_prefix("Directory: ") {
        return Some((dir.trim(), true));
    }
    let path_like = line.contains(['\\', '/']) && !line.contains("  ");
    path_like.then_some((line, false))
}

/// The outermost folder of `path` that is ignored, judged from its
/// repository root (from `cwd` when it lies under `cwd` outside any
/// repository); `path` itself counts only when it is a folder.
fn first_ignored(ignores: &mut Ignores, cwd: &Path, path: &Path, is_dir: bool) -> Option<PathBuf> {
    let parent = if is_dir { path } else { path.parent()? };
    let top = match ignores.repo_of(parent) {
        Some(top) => top,
        None if path.starts_with(cwd) => cwd.to_path_buf(),
        None => return None,
    };
    let rel = path.strip_prefix(&top).ok()?;
    let parts: Vec<Component> = rel.components().collect();
    let dirs = if is_dir {
        parts.len()
    } else {
        parts.len().saturating_sub(1)
    };
    let mut dir = top.clone();
    for part in &parts[..dirs] {
        let Component::Normal(name) = part else {
            return None;
        };
        dir.push(name);
        if ignores.ignored(&top, &dir) {
            return Some(dir);
        }
    }
    None
}
