//! The system prompt: a template (PLAN.md D31, built-in default D18) with its
//! placeholders filled, then the AGENTS.md files of the session's project
//! (from the repository root down to the working directory, never above it).

use std::path::{Path, PathBuf};

use super::prompt_store::{self, Source};
use crate::session;
use crate::trace;

/// Largest AGENTS.md text put into the prompt, all files together.
pub const MAX_AGENTS_BYTES: usize = 32 * 1024;

/// The built-in prompt template (PLAN.md D18), used without a prompt file (D31).
pub const DEFAULT_TEMPLATE: &str =
    "You are a coding agent in Fast Harness, working in {cwd} on {os}. \
Today is {date} (UTC).\nCall find_tools first to load the tools you need; \
independent calls in one step run in parallel. Be brief.\n";

/// The placeholders a template may use; any other text stays as written.
pub const PLACEHOLDERS: [&str; 4] = ["{cwd}", "{os}", "{date}", "{model}"];

/// A rendered system prompt and where its template came from.
#[derive(Clone, Debug)]
pub struct Rendered {
    pub text: String,
    pub source: Source,
    /// Why a prompt file was skipped (unreadable), for the front end.
    pub notes: Vec<String>,
}

/// The system prompt for an agent working in `cwd` with `model`: the
/// template of [`prompt_store::template`] filled, then the AGENTS.md section.
pub fn render(home: &Path, cwd: &Path, model: &str) -> Rendered {
    let template = prompt_store::template(home, cwd);
    Rendered {
        text: preview(&template.text, cwd, model),
        source: template.source,
        notes: template.notes,
    }
}

/// `template` filled for `cwd` and `model`, with the AGENTS.md section after it.
pub fn preview(template: &str, cwd: &Path, model: &str) -> String {
    compose(fill(template, cwd, model), &agents_files(cwd))
}

/// A filled template with the AGENTS.md section of `files` after it.
pub fn compose(out: String, files: &[(PathBuf, String)]) -> String {
    compose_with(out, "", files, MAX_AGENTS_BYTES)
}

/// [`compose`] with the skills line (D43) after the template and the
/// AGENTS.md section cut at `agents_bytes` (`agentsBytes`).
pub fn compose_with(
    mut out: String,
    skills: &str,
    files: &[(PathBuf, String)],
    agents_bytes: usize,
) -> String {
    if !skills.is_empty() {
        if !out.ends_with('\n') {
            out.push('\n');
        }
        out.push_str(skills);
        out.push('\n');
    }
    let agents = cap(agents_section(files), agents_bytes);
    if !agents.is_empty() {
        if !out.ends_with('\n') {
            out.push('\n');
        }
        out.push_str("\n# Project instructions (AGENTS.md)\n\n");
        out.push_str(&agents);
    }
    out
}

/// The built-in prompt for `cwd` without a model name (no prompt file read).
pub fn system_prompt(cwd: &Path) -> String {
    preview(DEFAULT_TEMPLATE, cwd, "")
}

/// `template` with the [`PLACEHOLDERS`] replaced for `cwd` and `model`.
pub fn fill(template: &str, cwd: &Path, model: &str) -> String {
    let values = [
        cwd.display().to_string(),
        os_name().to_string(),
        today(),
        model.to_string(),
    ];
    fill_with(template, &values)
}

/// One pass over `template`: each placeholder becomes `values[i]` (same order
/// as [`PLACEHOLDERS`]); text inside the values is never replaced again.
pub fn fill_with(template: &str, values: &[String; 4]) -> String {
    let mut out = String::with_capacity(template.len());
    let mut rest = template;
    while let Some(at) = rest.find('{') {
        out.push_str(&rest[..at]);
        let tail = &rest[at..];
        let hit = PLACEHOLDERS.iter().position(|key| tail.starts_with(key));
        match hit {
            Some(index) => {
                out.push_str(&values[index]);
                rest = &tail[PLACEHOLDERS[index].len()..];
            }
            None => {
                out.push('{');
                rest = &tail[1..];
            }
        }
    }
    out.push_str(rest);
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

fn agents_section(files: &[(PathBuf, String)]) -> String {
    let mut out = String::new();
    for (path, text) in files {
        out.push_str(&format!("## {}\n\n{}\n\n", path.display(), text.trim_end()));
    }
    out
}

pub(crate) fn cap(mut text: String, max: usize) -> String {
    if text.len() <= max {
        return text;
    }
    let mut end = max;
    while !text.is_char_boundary(end) {
        end -= 1;
    }
    text.truncate(end);
    let size = match max % 1024 {
        0 => format!("{} KB", max / 1024),
        _ => format!("{max} bytes"),
    };
    text.push_str(&format!("\n[AGENTS.md text cut at {size}]\n"));
    text
}

pub(crate) fn os_name() -> &'static str {
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
pub(crate) fn today() -> String {
    let stamp = session::utc_stamp(trace::now_ms() / 1000);
    format!("{}-{}-{}", &stamp[0..4], &stamp[4..6], &stamp[6..8])
}
