//! `write`: create or replace a UTF-8 text file.

use serde_json::{json, Value};

use super::args::{object_schema, req_str, resolve};
use super::{Tool, ToolCtx, ToolOutput};
use crate::llm::ToolSpec;

pub(crate) struct WriteTool;

impl Tool for WriteTool {
    fn spec(&self) -> ToolSpec {
        ToolSpec {
            name: "write".into(),
            description: "Create or fully replace a UTF-8 text file with `content`. Missing \
                parent folders are created. Reports the bytes and lines written and whether \
                the file was new."
                .into(),
            schema: object_schema(
                json!({
                    "path": {"type": "string", "description": "File path, absolute or relative to the working directory."},
                    "content": {"type": "string", "description": "The full new file content."}
                }),
                &["path", "content"],
            ),
        }
    }

    fn run(&self, args: &Value, ctx: &ToolCtx<'_>) -> ToolOutput {
        match write(args, ctx) {
            Ok(text) => ToolOutput::ok(text),
            Err(message) => ToolOutput::err(message),
        }
    }
}

fn write(args: &Value, ctx: &ToolCtx<'_>) -> Result<String, String> {
    let path_arg = req_str(args, "path")?;
    let content = req_str(args, "content")?;
    let path = resolve(ctx, path_arg);
    if path.is_dir() {
        return Err(format!("{path_arg} is a directory, not a file"));
    }
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)
            .map_err(|err| format!("cannot create folders for {path_arg}: {err}"))?;
    }
    // A file that cannot be read counts as new.
    let old = std::fs::read(&path).ok();
    std::fs::write(&path, content.as_bytes())
        .map_err(|err| format!("cannot write {path_arg}: {err}"))?;
    let lines = line_count(content);
    let bytes = content.len();
    Ok(match old {
        Some(old) => format!(
            "Wrote {bytes} bytes ({lines}) to {path_arg} (replaced {} bytes, {})",
            old.len(),
            line_count(&String::from_utf8_lossy(&old))
        ),
        None => format!("Wrote {bytes} bytes ({lines}) to {path_arg} (new file)"),
    })
}

/// "1 line" or "N lines".
fn line_count(text: &str) -> String {
    match text.lines().count() {
        1 => "1 line".to_string(),
        n => format!("{n} lines"),
    }
}
