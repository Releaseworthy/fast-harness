//! `approvals` and `allow` (PLAN.md D64): whether a `Write` or `Run` tool
//! call waits for the user's yes, and the allow list that skips the
//! question. Matching fails closed: anything it cannot prove covered asks.

use std::path::{Component, Path, PathBuf};

use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::tools::Effect;

/// `approvals`: `off` (the default, nothing asks) or `ask`.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Deserialize, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum Approvals {
    #[default]
    Off,
    Ask,
}

/// `allow`: what runs without a question under `ask`. Only the user layer
/// sets it ([`super::layers`] drops a project's).
#[derive(Clone, Debug, Default, PartialEq, Eq, Deserialize, Serialize)]
#[serde(default)]
pub struct Allow {
    /// Command prefixes of the `shell` tool, matched word by word.
    pub commands: Vec<String>,
    /// Folder or file prefixes of `write` and `edit`, absolute or relative
    /// to the session's working directory.
    pub paths: Vec<String>,
    /// Tool names allowed whatever their arguments (an MCP or plugin tool).
    pub tools: Vec<String>,
}

/// The shell tool's name.
pub const SHELL: &str = "shell";
/// The built-in tools whose `path` argument is the file they change.
pub const PATH_TOOLS: [&str; 2] = ["write", "edit"];

/// True when `command` is one simple command: no character that pwsh reads
/// as a separator, pipe, redirection, call, subexpression, script block or
/// escape (`; & | ( ) { } < > ` and backtick), no line break, and nothing
/// outside printable ASCII (pwsh also reads some Unicode dashes and quotes
/// as operators).
pub fn simple_command(command: &str) -> bool {
    const COMPOUND: &[char] = &[';', '&', '|', '(', ')', '{', '}', '<', '>', '`'];
    !command.is_empty()
        && command
            .chars()
            .all(|c| (' '..='~').contains(&c) && !COMPOUND.contains(&c))
}

/// True when `command` is simple and its first words equal the words of
/// one prefix (ASCII case ignored, as pwsh resolves commands).
pub fn command_allowed(command: &str, prefixes: &[String]) -> bool {
    if !simple_command(command) {
        return false;
    }
    let words: Vec<&str> = command.split_whitespace().collect();
    prefixes.iter().any(|prefix| {
        let want: Vec<&str> = prefix.split_whitespace().collect();
        !want.is_empty()
            && want.len() <= words.len()
            && want
                .iter()
                .zip(&words)
                .all(|(a, b)| a.eq_ignore_ascii_case(b))
    })
}

/// `path` made absolute against `cwd`, `.` and `..` resolved by name (a
/// `..` above the root gives `None`), then the longest existing ancestor
/// resolved through links and junctions; lower case on Windows.
pub fn normal_path(path: &str, cwd: &Path) -> Option<PathBuf> {
    let joined = cwd.join(path);
    let mut out = PathBuf::new();
    for part in joined.components() {
        match part {
            Component::CurDir => {}
            Component::ParentDir => {
                if !out.pop() || out.parent().is_none() {
                    return None;
                }
            }
            other => out.push(other),
        }
    }
    if !out.is_absolute() {
        return None;
    }
    Some(fold_case(resolve_existing(&out)))
}

/// The longest existing ancestor of `path` canonicalised, the rest added.
fn resolve_existing(path: &Path) -> PathBuf {
    for base in path.ancestors() {
        if std::fs::symlink_metadata(base).is_ok() {
            let rest = path.strip_prefix(base).unwrap_or(Path::new(""));
            let real = super::trust::canonical(base);
            return match rest.as_os_str().is_empty() {
                true => real,
                false => real.join(rest),
            };
        }
    }
    path.to_path_buf()
}

fn fold_case(path: PathBuf) -> PathBuf {
    match cfg!(windows) {
        true => PathBuf::from(path.to_string_lossy().to_lowercase()),
        false => path,
    }
}

/// True when `path` lies at or below one allowed prefix.
pub fn path_allowed(path: &str, cwd: &Path, prefixes: &[String]) -> bool {
    let Some(target) = normal_path(path, cwd) else {
        return false;
    };
    prefixes.iter().any(|prefix| {
        !prefix.trim().is_empty()
            && normal_path(prefix.trim(), cwd).is_some_and(|base| target.starts_with(base))
    })
}

/// What one call is, for matching, the session's grants and the prompt.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Subject {
    /// The command of a `shell` call, whitespace collapsed; the normal path
    /// of a `write` or `edit` call; empty for any other tool.
    pub key: String,
    /// One line for the person who answers.
    pub summary: String,
}

/// The subject of a call of `tool` with arguments `raw`.
pub fn subject(tool: &str, raw: &str, cwd: &Path) -> Subject {
    let args: Value = serde_json::from_str(raw).unwrap_or(Value::Null);
    let text = |key: &str| args.get(key).and_then(Value::as_str).map(str::to_string);
    if tool == SHELL {
        if let Some(command) = text("command") {
            let key = command.split_whitespace().collect::<Vec<_>>().join(" ");
            return Subject {
                summary: clip(&format!("run: {command}")),
                key,
            };
        }
    }
    if PATH_TOOLS.contains(&tool) {
        if let Some(path) = text("path") {
            let full = normal_path(&path, cwd);
            let shown = full
                .as_deref()
                .map_or(path.clone(), |p| p.display().to_string());
            return Subject {
                key: full.map_or_else(|| format!("?{path}"), |p| p.display().to_string()),
                summary: clip(&format!("{tool} file: {shown}")),
            };
        }
    }
    Subject {
        key: raw.trim().to_string(),
        summary: clip(&format!("{tool} {raw}")),
    }
}

/// At most 300 characters on one line.
fn clip(text: &str) -> String {
    let flat = text.replace(['\r', '\n'], " ");
    match flat.char_indices().nth(300) {
        Some((cut, _)) => format!("{}...", &flat[..cut]),
        None => flat,
    }
}

/// True when the allow list covers a call: `Read` always, a tool named in
/// `tools`, a simple `shell` command with an allowed prefix, a `write` or
/// `edit` path under an allowed prefix. `effect` is `None` for an unknown
/// tool, which asks.
pub fn covered(allow: &Allow, tool: &str, effect: Option<Effect>, raw: &str, cwd: &Path) -> bool {
    match effect {
        Some(Effect::Read) => return true,
        None => return false,
        Some(_) => {}
    }
    if allow.tools.iter().any(|name| name == tool) {
        return true;
    }
    let args: Value = serde_json::from_str(raw).unwrap_or(Value::Null);
    let text = |key: &str| args.get(key).and_then(Value::as_str);
    match (tool, effect) {
        (SHELL, Some(Effect::Run)) => {
            text("command").is_some_and(|c| command_allowed(c, &allow.commands))
        }
        (name, Some(Effect::Write)) if PATH_TOOLS.contains(&name) => {
            text("path").is_some_and(|p| path_allowed(p, cwd, &allow.paths))
        }
        _ => false,
    }
}
