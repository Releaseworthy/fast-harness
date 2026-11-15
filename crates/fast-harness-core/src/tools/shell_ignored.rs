//! Which folders `.gitignore` leaves out among the paths a recursive
//! listing printed (D89): the shell hint that points at `glob`.

use std::collections::HashMap;
use std::path::{Component, Path, PathBuf};

use ignore::gitignore::Gitignore;

/// Output lines looked at; a listing longer than this is judged by its start.
const MAX_LINES: usize = 20_000;
/// Ignored folders named in the hint.
const MAX_NAMED: usize = 3;

/// The `.gitignore` files met so far, by folder, and the folders judged.
struct Ignores {
    /// The repository root (the nearest folder above `cwd` with `.git`),
    /// else `cwd`: no `.gitignore` above it counts.
    top: PathBuf,
    files: HashMap<PathBuf, Option<Gitignore>>,
    judged: HashMap<PathBuf, bool>,
}

impl Ignores {
    fn new(cwd: &Path) -> Ignores {
        let top = cwd
            .ancestors()
            .find(|dir| dir.join(".git").exists())
            .unwrap_or(cwd)
            .to_path_buf();
        Ignores {
            top,
            files: HashMap::new(),
            judged: HashMap::new(),
        }
    }

    /// Does a `.gitignore` in a folder from `top` down to the parent of
    /// `dir` leave folder `dir` out?
    fn ignored(&mut self, dir: &Path) -> bool {
        if let Some(known) = self.judged.get(dir) {
            return *known;
        }
        let parents: Vec<PathBuf> = dir
            .ancestors()
            .skip(1)
            .take_while(|parent| parent.starts_with(&self.top))
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

/// Up to [`MAX_NAMED`] folders (relative to `cwd`) that `.gitignore`
/// leaves out and that the listing `output` went into.
pub(super) fn ignored_dirs(output: &str, cwd: &Path) -> Vec<String> {
    let mut ignores = Ignores::new(cwd);
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
fn listed_path(line: &str) -> Option<(&str, bool)> {
    let line = line.trim();
    if let Some(dir) = line.strip_prefix("Directory: ") {
        return Some((dir.trim(), true));
    }
    let path_like = line.contains(['\\', '/']) && !line.contains("  ");
    path_like.then_some((line, false))
}

/// The outermost folder of `path` under `cwd` that is ignored; `path`
/// itself counts only when it is a folder.
fn first_ignored(ignores: &mut Ignores, cwd: &Path, path: &Path, is_dir: bool) -> Option<PathBuf> {
    let rel = path.strip_prefix(cwd).ok()?;
    let parts: Vec<Component> = rel.components().collect();
    let dirs = if is_dir {
        parts.len()
    } else {
        parts.len().saturating_sub(1)
    };
    let mut dir = cwd.to_path_buf();
    for part in &parts[..dirs] {
        let Component::Normal(name) = part else {
            return None;
        };
        dir.push(name);
        if ignores.ignored(&dir) {
            return Some(dir);
        }
    }
    None
}
