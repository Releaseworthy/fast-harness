//! One short line describing a tool call.

use serde_json::Value;

use crate::llm::find::{self, FIND_TOOLS};

/// Longest summary, in characters.
const MAX_CHARS: usize = 80;

/// A one-line summary of a tool call: the path for `read`, `write` and
/// `edit`, the first command line for `shell`, pattern and path for `grep`,
/// the pattern for `glob`, the description or the start of the prompt for
/// `subagent`. Any other tool, or arguments that are not the expected JSON,
/// give the tool name and the start of the raw arguments.
pub fn tool_summary(name: &str, arguments: &str) -> String {
    let args: Value = serde_json::from_str(arguments).unwrap_or(Value::Null);
    match known_summary(name, &args) {
        Some(summary) => clip(&summary),
        None => {
            let raw = clip(&one_line(arguments));
            format!("{name} {raw}").trim_end().to_string()
        }
    }
}

fn known_summary(name: &str, args: &Value) -> Option<String> {
    let field = |key: &str| {
        args.get(key)
            .and_then(Value::as_str)
            .map(str::trim)
            .filter(|text| !text.is_empty())
    };
    match name {
        "read" | "write" | "edit" => field("path").map(str::to_string),
        "shell" => field("command").and_then(|command| {
            command
                .lines()
                .map(str::trim)
                .find(|line| !line.is_empty())
                .map(str::to_string)
        }),
        "grep" => field("pattern").map(|pattern| match field("path") {
            Some(path) => format!("{pattern} in {path}"),
            None => pattern.to_string(),
        }),
        "glob" => field("pattern").map(str::to_string),
        // `find_tools`: the names it asks for, else its quoted query.
        FIND_TOOLS => find::call_label(&args.to_string()),
        "subagent" => field("description")
            .or_else(|| field("prompt"))
            .map(one_line),
        _ => None,
    }
}

/// Collapses every run of whitespace, newlines included, into one space.
fn one_line(text: &str) -> String {
    text.split_whitespace().collect::<Vec<_>>().join(" ")
}

fn clip(text: &str) -> String {
    text.chars().take(MAX_CHARS).collect()
}
