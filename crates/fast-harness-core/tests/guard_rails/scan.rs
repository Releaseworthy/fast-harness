//! The tree and the tables: every `.rs` file of `crates/*/src`,
//! `crates/*/tests`, `crates/*/examples`, `crates/*/build.rs` and `templates/`,
//! read once, plus the layering scan over the core.

use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::OnceLock;

use regex::Regex;

use crate::text::{self, Func};
use crate::{EXCEPTIONS, FILE_LIMIT, FUNCTION_LIMIT, LAYERS};

pub(crate) struct Source {
    /// Relative to the workspace root, `/` only, so Linux and Windows agree.
    pub(crate) path: String,
    pub(crate) text: String,
}

pub(crate) fn workspace_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .ancestors()
        .nth(2)
        .expect("workspace root above the crate")
        .to_path_buf()
}

pub(crate) fn read(rel: &str) -> String {
    fs::read_to_string(workspace_root().join(rel)).unwrap_or_default()
}

fn sources() -> &'static [Source] {
    static SOURCES: OnceLock<Vec<Source>> = OnceLock::new();
    SOURCES.get_or_init(collect)
}

fn collect() -> Vec<Source> {
    let root = workspace_root();
    let mut crates: Vec<PathBuf> = Vec::new();
    for entry in fs::read_dir(root.join("crates")).expect("crates folder") {
        let dir = entry.expect("crate folder").path();
        if dir.is_dir() {
            crates.push(dir);
        }
    }
    crates.sort();
    let mut out = Vec::new();
    for dir in &crates {
        for sub in ["src", "tests", "examples"] {
            walk(&dir.join(sub), &root, &mut out);
        }
        let build = dir.join("build.rs");
        if build.is_file() {
            push(&build, &root, &mut out);
        }
    }
    walk(&root.join("templates"), &root, &mut out);
    out.sort_by(|a, b| a.path.cmp(&b.path));
    out
}

fn walk(dir: &Path, root: &Path, out: &mut Vec<Source>) {
    let Ok(entries) = fs::read_dir(dir) else {
        return;
    };
    let mut paths: Vec<PathBuf> = entries
        .map(|entry| entry.expect("folder entry").path())
        .collect();
    paths.sort();
    for path in paths {
        if path.is_dir() {
            if path.file_name() != Some(std::ffi::OsStr::new("target")) {
                walk(&path, root, out);
            }
        } else if path.extension() == Some(std::ffi::OsStr::new("rs")) {
            push(&path, root, out);
        }
    }
}

fn push(path: &Path, root: &Path, out: &mut Vec<Source>) {
    let rel = path.strip_prefix(root).unwrap_or(path);
    out.push(Source {
        path: rel.to_string_lossy().replace('\\', "/"),
        text: fs::read_to_string(path).unwrap_or_default(),
    });
}

fn all_functions() -> &'static [Func] {
    static FUNCS: OnceLock<Vec<Func>> = OnceLock::new();
    FUNCS.get_or_init(|| {
        let mut out = Vec::new();
        for src in sources() {
            out.extend(text::functions(&src.path, &src.text));
        }
        out
    })
}

/// Test code: a file under a `tests` folder, a `tests.rs` or a `*_tests.rs`,
/// and (in [`text::functions`]) a `#[cfg(test)]` module.
fn is_test_path(path: &str) -> bool {
    let name = path.rsplit('/').next().unwrap_or("");
    path.contains("/tests/") || name == "tests.rs" || name.ends_with("_tests.rs")
}

fn is_core_source(path: &str) -> bool {
    path.starts_with("crates/fast-harness-core/src/")
}

fn module_of(path: &str) -> Option<&str> {
    let rest = path.strip_prefix("crates/fast-harness-core/src/")?;
    let head = rest.split('/').next().unwrap_or("");
    Some(head.strip_suffix(".rs").unwrap_or(head))
}

pub(crate) fn layer_of(name: &str) -> Option<usize> {
    LAYERS.iter().position(|group| group.contains(&name))
}

pub(crate) fn declared_modules(lib: &str) -> Vec<String> {
    mod_re()
        .captures_iter(&text::without_tests(lib))
        .map(|hit| hit[1].to_string())
        .collect()
}

pub(crate) fn module_exists(name: &str) -> bool {
    let path = workspace_root()
        .join("crates/fast-harness-core/src")
        .join(name);
    path.with_extension("rs").is_file() || path.join("mod.rs").is_file()
}

/// The upward imports of the core: (from, to, file) with their line numbers.
pub(crate) fn upward_imports() -> BTreeMap<(String, String, String), BTreeSet<usize>> {
    let mut out: BTreeMap<(String, String, String), BTreeSet<usize>> = BTreeMap::new();
    for src in sources() {
        if !is_core_source(&src.path) || is_test_path(&src.path) {
            continue;
        }
        let Some(from) = module_of(&src.path) else {
            continue;
        };
        let Some(low) = layer_of(from) else {
            continue;
        };
        for (line, to) in text::imports(&src.text) {
            let Some(high) = layer_of(&to) else {
                continue;
            };
            if high > low {
                out.entry((from.to_string(), to, src.path.clone()))
                    .or_default()
                    .insert(line);
            }
        }
    }
    out
}

/// The names of the layer line of `lib.rs`, in order.
pub(crate) fn lib_layer_names(lib: &str) -> Vec<String> {
    let mut started = false;
    let mut text = String::new();
    for line in lib.lines() {
        if line.contains("Layering (a module never imports a higher one)") {
            started = true;
        }
        if !started {
            continue;
        }
        text.push_str(line.trim_start_matches("//!").trim());
        text.push(' ');
        if line.contains("`app`.") {
            break;
        }
    }
    backtick_re()
        .captures_iter(&text)
        .map(|hit| hit[1].to_string())
        .collect()
}

pub(crate) fn decision_links(plan: &str) -> BTreeSet<String> {
    decision_re()
        .captures_iter(plan)
        .map(|hit| format!("{}.md", &hit[1]))
        .collect()
}

/// The name a file line of `EXCEPTIONS` carries instead of a function name.
const FILE_NAME: &str = "<file>";

pub(crate) fn file_violations() -> Vec<(String, String, usize)> {
    sources()
        .iter()
        .map(|src| {
            (
                src.path.clone(),
                FILE_NAME.to_string(),
                src.text.lines().count(),
            )
        })
        .filter(|(_, _, lines)| *lines >= FILE_LIMIT)
        .collect()
}

pub(crate) fn function_violations() -> Vec<(String, String, usize)> {
    all_functions()
        .iter()
        .filter(|func| func.lines > FUNCTION_LIMIT && !is_test_path(&func.path))
        .map(|func| (func.path.clone(), func.name.clone(), func.lines))
        .collect()
}

/// What today's violations mean against `EXCEPTIONS`: the lines to fail with.
/// `files` picks the file lines of the table (name `<file>`) or the function
/// ones, so a listed function is no news to the file rule and the other way.
pub(crate) fn table_problems(violations: &[(String, String, usize)], files: bool) -> Vec<String> {
    let today: BTreeMap<(&str, &str), usize> = violations
        .iter()
        .map(|(path, name, lines)| ((path.as_str(), name.as_str()), *lines))
        .collect();
    let listed: BTreeMap<(&str, &str), usize> = EXCEPTIONS
        .iter()
        .filter(|(_, name, _)| (*name == FILE_NAME) == files)
        .map(|(path, name, lines)| ((*path, *name), *lines))
        .collect();
    let mut problems = Vec::new();
    for ((path, name), lines) in &today {
        match listed.get(&(*path, *name)) {
            None => problems.push(format!("new: {path} {name}, {lines} lines")),
            Some(before) if lines > before => {
                problems.push(format!(
                    "grew: {path} {name}, {lines} lines, listed at {before}"
                ));
            }
            Some(_) => {}
        }
    }
    for (path, name) in listed.keys() {
        if !today.contains_key(&(*path, *name)) {
            problems.push(format!(
                "gone or within the limit: {path} {name}, remove its line"
            ));
        }
    }
    problems
}

/// Today's table, sorted by size, ready to paste into `EXCEPTIONS`.
pub(crate) fn paste_table(violations: &[(String, String, usize)]) -> String {
    let mut sorted = violations.to_vec();
    sorted.sort_by(|a, b| {
        b.2.cmp(&a.2)
            .then_with(|| a.0.cmp(&b.0))
            .then_with(|| a.1.cmp(&b.1))
    });
    let mut out = String::from("today, sorted by size:\n");
    for (path, name, lines) in sorted {
        out.push_str(&format!("    (\"{path}\", \"{name}\", {lines}),\n"));
    }
    out
}

fn mod_re() -> &'static Regex {
    static RE: OnceLock<Regex> = OnceLock::new();
    RE.get_or_init(|| Regex::new(r"(?m)^(?:pub )?mod ([a-z_][a-z0-9_]*)\s*;").expect("mod regex"))
}

fn backtick_re() -> &'static Regex {
    static RE: OnceLock<Regex> = OnceLock::new();
    RE.get_or_init(|| Regex::new(r"`([a-z_][a-z0-9_]*)`").expect("backtick regex"))
}

/// Lines of non-test source outside `trace/` that read a trace event's data
/// by string key (PLAN.md D61), as `path:line`.
pub(crate) fn trace_key_reads() -> Vec<String> {
    let mut out = Vec::new();
    for src in sources() {
        let inside = src.path.starts_with("crates/fast-harness-core/src/trace/");
        if inside || is_test_path(&src.path) || !src.path.contains("/src/") {
            continue;
        }
        let lines: Vec<&str> = src.text.lines().map(str::trim_start).collect();
        for (index, code) in lines.iter().enumerate() {
            // A file's unit tests sit at its end, in a `#[cfg(test)]` module.
            let next = lines.get(index + 1).copied().unwrap_or("");
            if code.starts_with("#[cfg(test)]") && next.starts_with("mod ") {
                break;
            }
            if !code.starts_with("//") && trace_key_re().is_match(code) {
                out.push(format!("{}:{}", src.path, index + 1));
            }
        }
    }
    out
}

fn trace_key_re() -> &'static Regex {
    static RE: OnceLock<Regex> = OnceLock::new();
    RE.get_or_init(|| {
        Regex::new(r#"event\.data\b|event\["data"\]|\.data\[|\.data\.get\("#).expect("data regex")
    })
}

fn decision_re() -> &'static Regex {
    static RE: OnceLock<Regex> = OnceLock::new();
    RE.get_or_init(|| Regex::new(r"docs/decisions/(D\d+)\.md").expect("decision regex"))
}
