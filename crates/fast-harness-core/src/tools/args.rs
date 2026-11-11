//! Argument helpers shared by the built-in tools. Each returns a message that
//! tells the model what to fix.

use std::path::{Path, PathBuf};

use serde_json::Value;

use super::ToolCtx;

/// A required string argument.
pub(crate) fn req_str<'a>(args: &'a Value, name: &str) -> Result<&'a str, String> {
    match args.get(name) {
        Some(Value::String(text)) => Ok(text),
        Some(Value::Null) | None => Err(format!("missing required argument `{name}`")),
        Some(_) => Err(format!("argument `{name}` must be a string")),
    }
}

/// An optional string argument (`null` counts as absent).
pub(crate) fn opt_str<'a>(args: &'a Value, name: &str) -> Result<Option<&'a str>, String> {
    match args.get(name) {
        Some(Value::String(text)) => Ok(Some(text)),
        Some(Value::Null) | None => Ok(None),
        Some(_) => Err(format!("argument `{name}` must be a string")),
    }
}

/// An optional boolean argument, `default` when absent.
pub(crate) fn opt_bool(args: &Value, name: &str, default: bool) -> Result<bool, String> {
    match args.get(name) {
        Some(Value::Bool(flag)) => Ok(*flag),
        Some(Value::Null) | None => Ok(default),
        Some(_) => Err(format!("argument `{name}` must be true or false")),
    }
}

/// An optional positive integer argument, `default` when absent.
pub(crate) fn opt_positive(args: &Value, name: &str, default: u64) -> Result<u64, String> {
    match args.get(name) {
        Some(Value::Null) | None => Ok(default),
        Some(value) => match value.as_u64() {
            Some(number) if number >= 1 => Ok(number),
            _ => Err(format!("argument `{name}` must be a positive integer")),
        },
    }
}

/// Resolves a path argument against the context's working directory.
pub(crate) fn resolve(ctx: &ToolCtx<'_>, path: &str) -> PathBuf {
    let candidate = Path::new(path);
    let joined = if candidate.is_absolute() {
        candidate.to_path_buf()
    } else {
        ctx.cwd.join(candidate)
    };
    // Rebuilding from components drops `.` segments ("dir/./x" -> "dir/x"),
    // so paths shown relative to the working directory stay clean.
    joined.components().collect()
}

/// `path` relative to `cwd` when it lies under it, else as given; always with
/// `/` separators so the model sees one style.
pub(crate) fn display_rel(cwd: &Path, path: &Path) -> String {
    let shown = path.strip_prefix(cwd).unwrap_or(path);
    let text = shown.to_string_lossy();
    if cfg!(windows) {
        text.replace('\\', "/")
    } else {
        text.into_owned()
    }
}

/// True when the first 8 KB hold a NUL byte.
pub(crate) fn looks_binary(bytes: &[u8]) -> bool {
    bytes.iter().take(8 * 1024).any(|byte| *byte == 0)
}

/// Cuts `text` to `max` characters, marking the cut.
pub(crate) fn cut_chars(text: &str, max: usize) -> String {
    match text.char_indices().nth(max) {
        Some((end, _)) => format!("{}... [line cut]", &text[..end]),
        None => text.to_string(),
    }
}

/// The JSON Schema of an object with `properties` and `required` fields.
pub(crate) fn object_schema(properties: Value, required: &[&str]) -> Value {
    serde_json::json!({
        "type": "object",
        "properties": properties,
        "required": required,
        "additionalProperties": false,
    })
}
