//! `grep`: regex search over file contents.

use std::path::Path;
use std::sync::atomic::Ordering;

use regex::{Regex, RegexBuilder};
use serde_json::{json, Value};

use super::args::{
    cut_chars, display_rel, looks_binary, object_schema, opt_bool, opt_positive, opt_str, req_str,
    resolve,
};
use super::walk::files;
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
                line by line. Respects .gitignore and skips hidden and binary files. Output: \
                matches grouped by file, a path line (relative to the working directory) then \
                \"  line: text\" lines; at most max_results matches, then a count of the rest."
                .into(),
            schema: object_schema(
                json!({
                    "pattern": {"type": "string", "description": "Regular expression to search for."},
                    "path": {"type": "string", "description": "File or directory to search. Default: the working directory."},
                    "glob": {"type": "string", "description": "Only search files matching this glob, e.g. \"*.rs\" or \"src/**/*.ts\"."},
                    "ignore_case": {"type": "boolean", "description": "Case-insensitive match. Default false."},
                    "max_results": {"type": "integer", "minimum": 1, "description": "Maximum matching lines shown. Default 50."}
                }),
                &["pattern"],
            ),
        }
    }

    fn run(&self, args: &Value, ctx: &ToolCtx<'_>) -> ToolOutput {
        match grep(args, ctx) {
            Ok(text) => ToolOutput::ok(text),
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
    };
    for entry in files(&root, glob)? {
        if ctx.cancel.load(Ordering::Relaxed) {
            return Err("grep cancelled".to_string());
        }
        search_file(&mut search, ctx.cwd, entry.path());
    }
    Ok(render(&search))
}

/// Adds the matches of one file: shown up to `max`, counted after it.
fn search_file(search: &mut Search, cwd: &Path, path: &Path) {
    let Ok(bytes) = std::fs::read(path) else {
        return;
    };
    if looks_binary(&bytes) {
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
        let line = cut_chars(line, MAX_LINE_CHARS);
        shown.push(format!("  {}: {line}", index + 1));
    }
    if hidden > 0 {
        search.more += hidden;
        search.more_files += 1;
    }
    if !shown.is_empty() {
        search.groups.push((display_rel(cwd, path), shown));
    }
}

fn render(search: &Search) -> String {
    if search.groups.is_empty() {
        return "No matches.".to_string();
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
    out
}
