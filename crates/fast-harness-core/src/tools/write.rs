//! `write`: create or replace a text file. Replacing needs a read of the
//! current content first; a binary file and emptying a file need a flag.
//! A replaced file keeps its encoding, BOM and CRLF line ends.

use std::path::Path;

use serde_json::{json, Value};

use super::args::{object_schema, opt_bool, req_str, resolve};
use super::text_codec;
use super::{file_state, Effect, Tool, ToolCtx, ToolOutput};
use crate::llm::ToolSpec;

pub(crate) struct WriteTool;

impl Tool for WriteTool {
    fn effect(&self) -> Effect {
        Effect::Write
    }

    fn spec(&self) -> ToolSpec {
        ToolSpec {
            name: "write".into(),
            description: "Create or fully replace a text file with `content`. Missing \
                parent folders are created. Replacing a file needs a read of it first; it \
                keeps its encoding (UTF-16, BOM) and CRLF line ends. Reports the bytes and \
                lines written and whether the file was new."
                .into(),
            schema: object_schema(
                json!({
                    "path": {"type": "string", "description": "File path, absolute or relative to the working directory."},
                    "content": {"type": "string", "description": "The full new file content."},
                    "overwrite_binary": {"type": "boolean", "description": "Allow replacing a binary file. Default false."},
                    "allow_empty": {"type": "boolean", "description": "Allow empty content over a non-empty file. Default false."}
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
    let flags = (
        opt_bool(args, "overwrite_binary", false)?,
        opt_bool(args, "allow_empty", false)?,
    );
    let path = resolve(ctx, path_arg);
    if path.is_dir() {
        return Err(format!("{path_arg} is a directory, not a file"));
    }
    // A file that cannot be read counts as new.
    let old = std::fs::read(&path).ok();
    let (bytes, kept, old_text) = match &old {
        Some(old) => replacing(ctx, (&path, path_arg), old, content, flags)?,
        None => (content.as_bytes().to_vec(), String::new(), String::new()),
    };
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)
            .map_err(|err| format!("cannot create folders for {path_arg}: {err}"))?;
    }
    super::undo::tracked_replace(ctx, &path, &bytes)
        .map_err(|err| format!("cannot write {path_arg}: {err}"))?;
    file_state::record_change(ctx, &path, (&old_text, content), &bytes, true);
    let lines = line_count(content);
    let size = bytes.len();
    Ok(match old {
        Some(old) => format!(
            "Wrote {size} bytes ({lines}) to {path_arg} (replaced {} bytes, {}{kept})",
            old.len(),
            line_count(&old_text)
        ),
        None => format!("Wrote {size} bytes ({lines}) to {path_arg} (new file)"),
    })
}

/// The bytes replacing `old` (in its encoding and line ends), a note on
/// what was kept and the old text; refuses an unread or changed file, a
/// binary file without `overwrite_binary` and empty content without
/// `allow_empty`.
fn replacing(
    ctx: &ToolCtx<'_>,
    (path, path_arg): (&Path, &str),
    old: &[u8],
    content: &str,
    (overwrite_binary, allow_empty): (bool, bool),
) -> Result<(Vec<u8>, String, String), String> {
    let Some(encoding) = text_codec::detect(old) else {
        if !overwrite_binary {
            return Err(format!(
                "{path_arg} is a binary file ({} bytes); write refused. To replace it, call \
                 write again with overwrite_binary: true",
                old.len()
            ));
        }
        return Ok((content.as_bytes().to_vec(), String::new(), String::new()));
    };
    let old_text = text_codec::decode_lossy(old, encoding);
    if content.is_empty() && !old.is_empty() && !allow_empty {
        return Err(format!(
            "write of empty content over {path_arg} ({} bytes) refused. To empty it, call \
             write again with allow_empty: true",
            old.len()
        ));
    }
    if !old.is_empty() {
        file_state::check_seen(ctx, path, path_arg, old)?;
    }
    let crlf = text_codec::keep_line_ends(&old_text, content);
    let text = crlf.as_deref().unwrap_or(content);
    let mut kept: Vec<&str> = encoding.label().into_iter().collect();
    if crlf.is_some() {
        kept.push("CRLF");
    }
    let kept = match kept.is_empty() {
        true => String::new(),
        false => format!("; kept {}", kept.join(", ")),
    };
    Ok((text_codec::encode(text, encoding), kept, old_text))
}

/// "1 line" or "N lines".
fn line_count(text: &str) -> String {
    match text.lines().count() {
        1 => "1 line".to_string(),
        n => format!("{n} lines"),
    }
}
