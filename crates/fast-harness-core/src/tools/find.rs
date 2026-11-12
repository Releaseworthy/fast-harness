//! `find_tools` (PLAN.md D30): the only tool loaded at the start. It loads
//! other tools by exact name or by a ranked word search over names,
//! descriptions and parameter names, at most [`MAX_RESULTS`] per call.

use serde_json::{json, Value};

use crate::llm::find::{loaded_line, FIND_TOOLS};
use crate::llm::ToolSpec;
use crate::tools::skills::{self, Skill, MAX_SKILLS};
use crate::tools::ToolOutput;

/// Tools one call loads unless it passes `limit`.
pub const MAX_RESULTS: usize = 5;
/// The largest `limit` a call may pass.
pub const MAX_LIMIT: usize = 20;

/// The spec, whose description names the `others` without their schemas.
pub fn spec(others: &[ToolSpec]) -> ToolSpec {
    spec_noted(others, "")
}

/// [`spec`] whose description ends with `note` (the agent types, D42).
pub fn spec_noted(others: &[ToolSpec], note: &str) -> ToolSpec {
    let names: Vec<&str> = others.iter().map(|tool| tool.name.as_str()).collect();
    let mut description = format!(
        "Load tools: only loaded tools can be called. Tools: {}. Pass `names` \
        (exact tool names) or `query` (words matched against tool names, \
        descriptions and parameter names); at most {MAX_RESULTS} tools per call \
        unless you pass `limit` (up to {MAX_LIMIT}). \
        A loaded tool stays loaded for the session.",
        names.join(", ")
    );
    if !note.is_empty() {
        description.push(' ');
        description.push_str(note);
    }
    ToolSpec {
        name: FIND_TOOLS.into(),
        description,
        schema: json!({
            "type": "object",
            "properties": {
                "query": {"type": "string", "description": "Words describing the tools you need."},
                "names": {"type": "array", "items": {"type": "string"}, "description": "Exact tool names."},
                "limit": {"type": "integer", "minimum": 1, "maximum": MAX_LIMIT, "description": "Most tools to load. Default 5."}
            }
        }),
    }
}

/// Runs a find over `others` (every tool but `find_tools`).
pub fn run(args: &Value, others: &[ToolSpec]) -> ToolOutput {
    run_with(args, others, &[])
}

/// [`run`] that also matches `skills` (PLAN.md D43): a name or the best
/// query match returns the skill body as text after the tool lines.
pub fn run_with(args: &Value, others: &[ToolSpec], skills: &[Skill]) -> ToolOutput {
    let names: Vec<&str> = args
        .get("names")
        .and_then(Value::as_array)
        .map(|list| list.iter().filter_map(Value::as_str).collect())
        .unwrap_or_default();
    let query = args.get("query").and_then(Value::as_str).unwrap_or("");
    if names.is_empty() && query.trim().is_empty() {
        return ToolOutput::err("pass `query` (words) or `names` (exact tool names)");
    }
    let mut picked: Vec<usize> = Vec::new();
    let mut unknown: Vec<&str> = Vec::new();
    let mut skilled: Vec<usize> = Vec::new();
    for name in &names {
        let tool = others
            .iter()
            .position(|t| t.name.eq_ignore_ascii_case(name));
        let skill = skills
            .iter()
            .position(|s| s.name.eq_ignore_ascii_case(name));
        match (tool, skill) {
            (Some(index), _) if !picked.contains(&index) => picked.push(index),
            (Some(_), _) => {}
            (None, Some(index)) if !skilled.contains(&index) => skilled.push(index),
            (None, Some(_)) => {}
            (None, None) => unknown.push(name),
        }
    }
    if let Some(best) = best_skill(query, skills) {
        if !skilled.contains(&best) {
            skilled.push(best);
        }
    }
    skilled.truncate(MAX_SKILLS);
    for index in search(query, others) {
        if !picked.contains(&index) {
            picked.push(index);
        }
    }
    let limit = args
        .get("limit")
        .and_then(Value::as_u64)
        .unwrap_or(MAX_RESULTS as u64);
    picked.truncate((limit as usize).clamp(1, MAX_LIMIT));
    let mut text = answer(&picked, &unknown, others, !skilled.is_empty());
    for index in skilled {
        text.push_str("\n\n");
        text.push_str(&skills::body(&skills[index]));
    }
    ToolOutput::ok(text)
}

/// The skill matching `query` best, when it scores at least 2 (a name hit
/// or two description words).
fn best_skill(query: &str, skills: &[Skill]) -> Option<usize> {
    if query.trim().is_empty() || skills.is_empty() {
        return None;
    }
    let specs: Vec<ToolSpec> = (skills.iter())
        .map(|skill| ToolSpec {
            name: skill.name.clone(),
            description: skill.description.clone(),
            schema: json!({}),
        })
        .collect();
    let lower = query.to_lowercase();
    let words: Vec<&str> = lower
        .split(|c: char| !c.is_alphanumeric())
        .filter(|word| word.chars().count() >= 2)
        .collect();
    let best = search(query, &specs).into_iter().next()?;
    (score(&words, &specs[best]) >= 2).then_some(best)
}

fn answer(picked: &[usize], unknown: &[&str], others: &[ToolSpec], skills: bool) -> String {
    let all: Vec<&str> = others.iter().map(|tool| tool.name.as_str()).collect();
    let mut lines = Vec::new();
    if picked.is_empty() && skills {
        lines.push("Skill instructions below (no tool loaded).".to_string());
    } else if picked.is_empty() {
        lines.push(format!("No tool matched. Tools: {}.", all.join(", ")));
    } else {
        let found: Vec<&str> = picked.iter().map(|&i| others[i].name.as_str()).collect();
        lines.push(loaded_line(&found));
        lines.push("You can call them now.".to_string());
    }
    if !unknown.is_empty() {
        lines.push(format!("Unknown: {}.", unknown.join(", ")));
    }
    lines.join("\n")
}

/// Indexes of the tools matching `query`, best first; ties keep the
/// registration order. Tools matching no word are left out.
pub fn search(query: &str, tools: &[ToolSpec]) -> Vec<usize> {
    let lower = query.to_lowercase();
    let words: Vec<&str> = lower
        .split(|c: char| !c.is_alphanumeric())
        .filter(|word| word.chars().count() >= 2)
        .collect();
    let mut scored: Vec<(u32, usize)> = tools
        .iter()
        .enumerate()
        .map(|(index, tool)| (score(&words, tool), index))
        .filter(|(score, _)| *score > 0)
        .collect();
    scored.sort_by(|a, b| b.0.cmp(&a.0).then(a.1.cmp(&b.1)));
    scored.into_iter().map(|(_, index)| index).collect()
}

fn score(words: &[&str], tool: &ToolSpec) -> u32 {
    let name = tool.name.to_lowercase();
    let description = tool.description.to_lowercase();
    let params: Vec<String> = tool
        .schema
        .get("properties")
        .and_then(Value::as_object)
        .map(|props| props.keys().map(|key| key.to_lowercase()).collect())
        .unwrap_or_default();
    let mut total = 0;
    for word in words {
        total += if name == *word {
            10
        } else if name.contains(word) {
            6
        } else {
            0
        };
        if params.iter().any(|param| param == word) {
            total += 3;
        } else if params.iter().any(|param| param.contains(word)) {
            total += 2;
        }
        if description.contains(word) {
            total += 1;
        }
    }
    total
}
