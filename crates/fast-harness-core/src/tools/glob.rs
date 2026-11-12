//! `glob`: find files by path pattern, newest first.

use std::sync::atomic::Ordering;
use std::time::SystemTime;

use serde_json::{json, Value};

use super::args::{display_rel, object_schema, opt_str, req_str, resolve};
use super::walk::files;
use super::{Tool, ToolCtx, ToolOutput};
use crate::llm::ToolSpec;

const MAX_FILES: usize = 100;

pub(crate) struct GlobTool;

impl Tool for GlobTool {
    fn spec(&self) -> ToolSpec {
        ToolSpec {
            name: "glob".into(),
            description: "Find files whose path matches a glob pattern (gitignore syntax: \
                \"*.rs\" matches at any depth, \"src/**/*.ts\" is anchored to `path`). \
                Respects .gitignore and skips hidden files. Returns at most 100 paths, newest \
                modification first, relative to the working directory."
                .into(),
            schema: object_schema(
                json!({
                    "pattern": {"type": "string", "description": "Glob pattern, e.g. \"**/*.rs\"."},
                    "path": {"type": "string", "description": "Directory to search. Default: the working directory."}
                }),
                &["pattern"],
            ),
        }
    }

    fn run(&self, args: &Value, ctx: &ToolCtx<'_>) -> ToolOutput {
        match glob(args, ctx) {
            Ok(text) => ToolOutput::ok(text),
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
    let mut found: Vec<(SystemTime, String)> = Vec::new();
    for entry in files(&root, Some(pattern))? {
        if ctx.cancel.load(Ordering::Relaxed) {
            return Err("glob cancelled".to_string());
        }
        let modified = entry
            .metadata()
            .ok()
            .and_then(|meta| meta.modified().ok())
            .unwrap_or(SystemTime::UNIX_EPOCH);
        found.push((modified, display_rel(ctx.cwd, entry.path())));
    }
    if found.is_empty() {
        return Ok("No files found.".to_string());
    }
    found.sort_by(|a, b| b.0.cmp(&a.0).then_with(|| a.1.cmp(&b.1)));
    let total = found.len();
    let mut out: Vec<String> = found.into_iter().take(MAX_FILES).map(|(_, p)| p).collect();
    if total > MAX_FILES {
        out.push(format!(
            "[{} more files not shown; narrow the pattern or path]",
            total - MAX_FILES
        ));
    }
    Ok(out.join("\n") + "\n")
}
