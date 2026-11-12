//! Cards of the tools that run something: shell, subagent, find and others.

use super::tool_detail::{count, value_text, Call, Tone, ToolDetail};
use super::tool_summary;
use crate::llm::find::call_label;

pub(crate) fn shell(detail: &mut ToolDetail, call: &Call<'_>) {
    let command = call.str("command").unwrap_or_default();
    detail.target = first_line(command);
    if command.trim().lines().count() > 1 {
        detail.field("command", command);
    }
    if let Some(ms) = call.text("timeout_ms") {
        detail.field("timeout", format!("{ms} ms"));
    }
    let Some(out) = call.output else { return };
    let mut lines: Vec<&str> = out.lines().collect();
    let mut notes = Vec::new();
    while let Some(note) = lines.last().and_then(|line| shell_note(line)) {
        notes.push(note);
        lines.pop();
    }
    let text = lines.join("\n");
    if text.trim() == "(no output)" {
        detail.chip("no output", Tone::Plain);
    } else {
        detail.text = text;
    }
    for (text, tone) in notes.into_iter().rev() {
        detail.chip(text, tone);
    }
}

/// A trailing bracket line of shell's output as a chip.
fn shell_note(line: &str) -> Option<(String, Tone)> {
    let inner = line.trim().strip_prefix('[')?.strip_suffix(']')?;
    let (text, tone) = if let Some(code) = inner.strip_prefix("exit code: ") {
        match code {
            "none" => ("no exit code".to_string(), Tone::Warning),
            "0" => ("exit 0".to_string(), Tone::Success),
            _ => (format!("exit {code}"), Tone::Error),
        }
    } else if inner.starts_with("timed out") {
        ("timed out".to_string(), Tone::Error)
    } else if inner.starts_with("cancelled;") {
        ("cancelled".to_string(), Tone::Warning)
    } else {
        let bytes = inner.strip_suffix(" more bytes of output dropped")?;
        bytes.parse::<u64>().ok()?;
        ("output cut".to_string(), Tone::Warning)
    };
    Some((text, tone))
}

pub(crate) fn subagent(detail: &mut ToolDetail, call: &Call<'_>) {
    let prompt = call.str("prompt").unwrap_or_default();
    let description = call.str("description").filter(|d| !d.trim().is_empty());
    detail.target = match description {
        Some(text) => text.trim().to_string(),
        None => first_line(prompt),
    };
    if let Some(text) = description {
        detail.field("description", text);
    }
    detail.field("prompt", prompt);
    let Some(out) = call.output else { return };
    let mut kept = Vec::new();
    let (mut session, mut ended) = (None, None);
    for line in out.lines() {
        let trimmed = line.trim();
        if let Some(inner) = trimmed.strip_prefix("[subagent session ") {
            session = inner.strip_suffix(']');
        } else if let Some(rest) = trimmed.strip_prefix("subagent ended ") {
            ended = rest.split(':').next().map(str::trim);
        } else {
            kept.push(line);
        }
    }
    let mut parts = session.unwrap_or_default().split(" · ");
    if let Some(id) = parts.next().filter(|id| !id.is_empty()) {
        detail.chip(format!("session {id}"), Tone::Plain);
    }
    for part in parts {
        let tone = if part.starts_with("model ") {
            Tone::Info
        } else {
            Tone::Plain
        };
        detail.chip(part, tone);
    }
    if let Some(outcome) = ended {
        detail.chip(format!("ended {outcome}"), Tone::Error);
    }
    detail.text = kept.join("\n").trim().to_string();
}

pub(crate) fn find(detail: &mut ToolDetail, call: &Call<'_>, arguments: &str) {
    let label = call_label(arguments).or_else(|| call.text("query"));
    detail.target = label.unwrap_or_default();
    let Some(out) = call.result() else { return };
    if let Some(names) = out.lines().find_map(|line| line.strip_prefix("Loaded: ")) {
        let n = names
            .split(',')
            .filter(|name| !name.trim().is_empty())
            .count();
        detail.chip(format!("loaded {}", count(n, "tool", "tools")), Tone::Info);
    } else if out.starts_with("No tool matched.") {
        detail.chip("no match", Tone::Warning);
    }
    detail.text = out.to_string();
}

pub(crate) fn other(detail: &mut ToolDetail, call: &Call<'_>, name: &str, arguments: &str) {
    detail.target = tool_summary(name, arguments);
    for (key, value) in call.args {
        detail.field(key, value_text(value));
    }
    detail.text = call.output.unwrap_or_default().to_string();
}

/// The first non-empty line, trimmed.
fn first_line(text: &str) -> String {
    let line = text.lines().map(str::trim).find(|line| !line.is_empty());
    line.unwrap_or_default().to_string()
}
