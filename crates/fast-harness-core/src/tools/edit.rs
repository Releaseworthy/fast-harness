//! `edit`: exact string replacement in a text file.

use serde_json::{json, Value};

use super::args::{object_schema, opt_bool, req_str, resolve};
use super::{Tool, ToolCtx, ToolOutput};
use crate::llm::ToolSpec;

pub(crate) struct EditTool;

impl Tool for EditTool {
    fn spec(&self) -> ToolSpec {
        ToolSpec {
            name: "edit".into(),
            description: "Replace exact text in a file. `old_string` must match the file \
                exactly (whitespace included) and, unless replace_all is true, exactly once; \
                add surrounding lines to make it unique. Reports the number of replacements."
                .into(),
            schema: object_schema(
                json!({
                    "path": {"type": "string", "description": "File path, absolute or relative to the working directory."},
                    "old_string": {"type": "string", "description": "Exact text to replace."},
                    "new_string": {"type": "string", "description": "Replacement text."},
                    "replace_all": {"type": "boolean", "description": "Replace every match. Default false."}
                }),
                &["path", "old_string", "new_string"],
            ),
        }
    }

    fn run(&self, args: &Value, ctx: &ToolCtx<'_>) -> ToolOutput {
        match edit(args, ctx) {
            Ok(text) => ToolOutput::ok(text),
            Err(message) => ToolOutput::err(message),
        }
    }
}

fn edit(args: &Value, ctx: &ToolCtx<'_>) -> Result<String, String> {
    let path_arg = req_str(args, "path")?;
    let old = req_str(args, "old_string")?;
    let new = req_str(args, "new_string")?;
    let replace_all = opt_bool(args, "replace_all", false)?;
    if old.is_empty() {
        return Err("old_string must not be empty".to_string());
    }
    if old == new {
        return Err("old_string and new_string are identical; nothing to change".to_string());
    }
    let path = resolve(ctx, path_arg);
    let bytes = std::fs::read(&path).map_err(|err| format!("cannot read {path_arg}: {err}"))?;
    let content = String::from_utf8(bytes)
        .map_err(|_| format!("{path_arg} is not valid UTF-8 text; edit refused"))?;
    let (old, new) = pick_strings(&content, old, new);
    let count = content.matches(old.as_str()).count();
    if count == 0 {
        return Err(format!("old_string not found in {path_arg}"));
    }
    if count > 1 && !replace_all {
        return Err(format!(
            "old_string matches {count} times in {path_arg}; add surrounding context to make \
             it unique, or set replace_all to true"
        ));
    }
    let (updated, replaced) = if replace_all {
        (content.replace(old.as_str(), &new), count)
    } else {
        (content.replacen(old.as_str(), &new, 1), 1)
    };
    std::fs::write(&path, updated.as_bytes())
        .map_err(|err| format!("cannot write {path_arg}: {err}"))?;
    let lines = match_lines(&content, &old, replaced);
    Ok(if replaced == 1 {
        format!("Replaced 1 occurrence in {path_arg} at line {lines}")
    } else {
        format!("Replaced {replaced} occurrences in {path_arg} at lines {lines}")
    })
}

/// The 1-based line numbers of the first `count` matches of `old` in
/// `content`: at most 10, then ", ...".
fn match_lines(content: &str, old: &str, count: usize) -> String {
    let mut numbers = Vec::new();
    let mut line = 1;
    let mut seen = 0;
    for (index, _) in content.match_indices(old).take(count.min(10)) {
        line += content[seen..index].matches('\n').count();
        seen = index;
        numbers.push(line.to_string());
    }
    let mut text = numbers.join(", ");
    if count > 10 {
        text.push_str(", ...");
    }
    text
}

/// The strings to use: as given, or with CRLF line ends when the given ones
/// do not match and the file uses CRLF.
fn pick_strings(content: &str, old: &str, new: &str) -> (String, String) {
    if content.contains(old) || !content.contains("\r\n") || !old.contains('\n') {
        return (old.to_string(), new.to_string());
    }
    (to_crlf(old), to_crlf(new))
}

fn to_crlf(text: &str) -> String {
    text.replace("\r\n", "\n").replace('\n', "\r\n")
}
