//! `glob`: find files by path pattern, newest first.

use std::time::SystemTime;

use serde_json::{json, Value};

use super::args::{display_rel, object_schema, opt_str, req_str, resolve, search_base};
use super::grep::include;
use super::search_seen::dedupe;
use super::walk::walk;
use super::{Effect, Tool, ToolCtx, ToolOutput};
use crate::llm::ToolSpec;

const MAX_FILES: usize = 100;

pub(crate) struct GlobTool;

impl Tool for GlobTool {
    fn effect(&self) -> Effect {
        Effect::Read
    }

    fn spec(&self) -> ToolSpec {
        ToolSpec {
            name: "glob".into(),
            description: "Find files whose path matches a glob pattern (gitignore syntax: \
                \"*.rs\" matches at any depth, \"src/**/*.ts\" is anchored to `path`). \
                Skips files .gitignore leaves out and hidden files (a last line counts them). Returns at most 100 paths, newest \
                modification first, relative to the working directory, or to `path` when \
                given (a first line names it)."
                .into(),
            schema: object_schema(
                json!({
                    "pattern": {"type": "string", "description": "Glob pattern, e.g. \"**/*.rs\"."},
                    "path": {"type": "string", "description": "Directory to search. Default: the working directory."},
                    "include_ignored": {"type": "boolean", "description": "Also list files .gitignore leaves out. Default false."},
                    "include_hidden": {"type": "boolean", "description": "Also list hidden files and folders. Default false."}
                }),
                &["pattern"],
            ),
        }
    }

    fn run(&self, args: &Value, ctx: &ToolCtx<'_>) -> ToolOutput {
        match glob(args, ctx) {
            Ok(text) => ToolOutput::ok(dedupe("glob", args, text, ctx)),
            Err(message) => ToolOutput::err(message),
        }
    }
}

fn glob(args: &Value, ctx: &ToolCtx<'_>) -> Result<String, String> {
    let pattern = req_str(args, "pattern")?;
    if pattern.trim().is_empty() {
        return Err("pattern must not be empty".to_string());
    }
    let root = resolve(ctx, opt_str(args, "path")?.unwrap_or("."));
    if !root.is_dir() {
        return Err(format!("not a directory: {}", root.display()));
    }
    let (base, header) = search_base(ctx.cwd, &root);
    let mut found: Vec<(SystemTime, String)> = Vec::new();
    let walked = walk(&root, Some(pattern), include(args)?, ctx.cancel);
    let walked = walked.map_err(|err| format!("glob {err}"))?;
    for entry in &walked.files {
        let modified = entry
            .metadata()
            .ok()
            .and_then(|meta| meta.modified().ok())
            .unwrap_or(SystemTime::UNIX_EPOCH);
        found.push((modified, display_rel(base, entry.path())));
    }
    let note = walked.skipped.note(0);
    if found.is_empty() {
        return Ok(match note {
            Some(note) => format!("No files found.\n{note}\n"),
            None => "No files found.".to_string(),
        });
    }
    found.sort_by(|a, b| b.0.cmp(&a.0).then_with(|| a.1.cmp(&b.1)));
    let total = found.len();
    let mut out: Vec<String> = header.into_iter().collect();
    out.extend(found.into_iter().take(MAX_FILES).map(|(_, p)| p));
    if total > MAX_FILES {
        out.push(format!(
            "[{} more files not shown; narrow the pattern or path]",
            total - MAX_FILES
        ));
    }
    out.extend(note);
    Ok(out.join("\n") + "\n")
}
