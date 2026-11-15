//! `grep`: regex search over file contents.

use std::path::Path;
use std::sync::atomic::Ordering;

use regex::{Regex, RegexBuilder};
use serde_json::{json, Value};

use super::args::{
    cut_chars, display_rel, looks_binary, object_schema, opt_bool, opt_positive, opt_str, req_str,
    resolve, search_base,
};
use super::search_seen::dedupe;
use super::walk::{walk, Include, Skipped};
use super::{Effect, Tool, ToolCtx, ToolOutput};
use crate::llm::ToolSpec;

const DEFAULT_MAX_RESULTS: u64 = 50;
const MAX_LINE_CHARS: usize = 300;

pub(crate) struct GrepTool;

impl Tool for GrepTool {
    fn effect(&self) -> Effect {
        Effect::Read
    }

    fn spec(&self) -> ToolSpec {
        ToolSpec {
            name: "grep".into(),
            description: "Search file contents with a regular expression (Rust regex syntax), \
                line by line. Skips files .gitignore leaves out, hidden and binary files \
                (a last line counts them). Output: \
                matches grouped by file, a path line (relative to the working directory, or \
                to a folder `path`, named once by a first line) then \
                \"  line: text\" lines; at most max_results matches, then a count of the rest."
                .into(),
            schema: object_schema(
                json!({
                    "pattern": {"type": "string", "description": "Regular expression to search for."},
                    "path": {"type": "string", "description": "File or directory to search. Default: the working directory."},
                    "glob": {"type": "string", "description": "Only search files matching this glob, e.g. \"*.rs\" or \"src/**/*.ts\"."},
                    "ignore_case": {"type": "boolean", "description": "Case-insensitive match. Default false."},
                    "max_results": {"type": "integer", "minimum": 1, "description": "Maximum matching lines shown. Default 50."},
                    "include_ignored": {"type": "boolean", "description": "Also search files .gitignore leaves out. Default false."},
                    "include_hidden": {"type": "boolean", "description": "Also search hidden files and folders. Default false."}
                }),
                &["pattern"],
            ),
        }
    }

    fn run(&self, args: &Value, ctx: &ToolCtx<'_>) -> ToolOutput {
        match grep(args, ctx) {
            Ok(text) => ToolOutput::ok(dedupe("grep", args, text, ctx)),
            Err(message) => ToolOutput::err(message),
        }
    }
}

struct Search {
    regex: Regex,
    max: usize,
    /// Shown matches grouped by file, in walk order.
    groups: Vec<(String, Vec<String>)>,
    shown: usize,
    more: usize,
    more_files: usize,
    /// Binary files passed over.
    binary: usize,
}

fn grep(args: &Value, ctx: &ToolCtx<'_>) -> Result<String, String> {
    let pattern = req_str(args, "pattern")?;
    let root = resolve(ctx, opt_str(args, "path")?.unwrap_or("."));
    let glob = opt_str(args, "glob")?;
    let ignore_case = opt_bool(args, "ignore_case", false)?;
    let max = opt_positive(args, "max_results", DEFAULT_MAX_RESULTS)?;
    let regex = RegexBuilder::new(pattern)
        .case_insensitive(ignore_case)
        .build()
        .map_err(|err| format!("invalid regex: {err}"))?;
    if !root.exists() {
        return Err(format!("path not found: {}", root.display()));
    }
    let mut search = Search {
        regex,
        max: usize::try_from(max).unwrap_or(usize::MAX),
        groups: Vec::new(),
        shown: 0,
        more: 0,
        more_files: 0,
        binary: 0,
    };
    let (base, header) = search_base(ctx.cwd, &root);
    let found = walk(&root, glob, include(args)?, ctx.cancel).map_err(|e| format!("grep {e}"))?;
    for entry in &found.files {
        if ctx.cancel.load(Ordering::Relaxed) {
            return Err("grep cancelled".to_string());
        }
        search_file(&mut search, base, entry.path());
    }
    match (header, render(&search, &found.skipped)) {
        (Some(line), out) if !search.groups.is_empty() => Ok(format!("{line}\n{out}")),
        (_, out) => Ok(out),
    }
}

/// Adds the matches of one file: shown up to `max`, counted after it; its
/// path is shown relative to `base`.
fn search_file(search: &mut Search, base: &Path, path: &Path) {
    let Ok(bytes) = std::fs::read(path) else {
        return;
    };
    if looks_binary(&bytes) {
        search.binary += 1;
        return;
    }
    let text = String::from_utf8_lossy(&bytes);
    let mut shown = Vec::new();
    let mut hidden = 0;
    for (index, line) in text.lines().enumerate() {
        if !search.regex.is_match(line) {
            continue;
        }
        if search.shown >= search.max {
            hidden += 1;
            continue;
        }
        search.shown += 1;
        let line = clip(&search.regex, line);
        shown.push(format!("  {}: {line}", index + 1));
    }
    if hidden > 0 {
        search.more += hidden;
        search.more_files += 1;
    }
    if !shown.is_empty() {
        search.groups.push((display_rel(base, path), shown));
    }
}

/// Characters kept before a match that starts past the first
/// [`MAX_LINE_CHARS`] of its line.
const LEAD_CHARS: usize = MAX_LINE_CHARS / 4;

/// `line` within [`MAX_LINE_CHARS`] characters: its start when the first
/// match ends inside it, else a window from a little before that match,
/// `...` on each cut side and a note with the window's columns and the
/// matches left outside it.
fn clip(regex: &Regex, line: &str) -> String {
    let total = line.chars().count();
    let Some(first) = regex.find(line).filter(|_| total > MAX_LINE_CHARS) else {
        return cut_chars(line, MAX_LINE_CHARS);
    };
    let start = line[..first.start()].chars().count();
    let end = start + first.as_str().chars().count();
    if end <= MAX_LINE_CHARS {
        return cut_chars(line, MAX_LINE_CHARS);
    }
    let from = start.saturating_sub(LEAD_CHARS).min(total - MAX_LINE_CHARS);
    let to = from + MAX_LINE_CHARS;
    let byte = |column: usize| {
        line.char_indices()
            .nth(column)
            .map_or(line.len(), |(i, _)| i)
    };
    let (a, b) = (byte(from), byte(to));
    let outside = regex
        .find_iter(line)
        .filter(|m| m.start() < a || m.start() >= b)
        .count();
    let mark = |cut: bool| if cut { "..." } else { "" };
    let (left, right) = (mark(from > 0), mark(to < total));
    let shown = &line[a..b];
    let mut out = format!(
        "{left}{shown}{right} [line cut: columns {}-{to} of {total}",
        from + 1
    );
    if outside > 0 {
        out.push_str(&format!("; {outside} more matches outside"));
    }
    out.push(']');
    out
}
/// The include_ignored and include_hidden arguments.
pub(super) fn include(args: &Value) -> Result<Include, String> {
    Ok(Include {
        ignored: opt_bool(args, "include_ignored", false)?,
        hidden: opt_bool(args, "include_hidden", false)?,
    })
}

fn render(search: &Search, skipped: &Skipped) -> String {
    let note = skipped.note(search.binary);
    if search.groups.is_empty() {
        return match note {
            Some(note) => format!("No matches.\n{note}\n"),
            None => "No matches.".to_string(),
        };
    }
    let mut out = String::new();
    for (path, lines) in &search.groups {
        out.push_str(path);
        out.push('\n');
        for line in lines {
            out.push_str(line);
            out.push('\n');
        }
    }
    if search.more > 0 {
        let files = match search.more_files {
            1 => "1 file".to_string(),
            n => format!("{n} files"),
        };
        out.push_str(&format!(
            "[{} more matches in {files}; narrow the pattern, path or glob, or raise max_results]\n",
            search.more
        ));
    }
    if let Some(note) = note {
        out.push_str(&note);
        out.push('\n');
    }
    out
}
