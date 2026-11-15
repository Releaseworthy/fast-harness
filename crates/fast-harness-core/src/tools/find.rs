//! `find_tools` (PLAN.md D30): the only tool loaded at the start. It loads
//! other tools by exact name or by a ranked word search over names,
//! descriptions and parameter names, at most [`MAX_RESULTS`] per call.

use serde_json::{json, Value};

use crate::llm::find::{loaded_line, CALL_NOW, FIND_TOOLS};
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
    run_loaded(args, others, skills, &[])
}

/// [`run_with`] where the tools in `loaded` are loaded already: each one
/// matched gets the line `<name>: already loaded` instead of its schema.
pub fn run_loaded(
    args: &Value,
    others: &[ToolSpec],
    skills: &[Skill],
    loaded: &[String],
) -> ToolOutput {
    let names: Vec<&str> = args
        .get("names")
        .and_then(Value::as_array)
        .map(|list| list.iter().filter_map(Value::as_str).collect())
        .unwrap_or_default();
    let query = args.get("query").and_then(Value::as_str).unwrap_or("");
    if names.is_empty() && query.trim().is_empty() {
        return ToolOutput::err("pass `query` (words) or `names` (exact tool names)");
    }
    let (mut picked, mut skilled, unknown) = by_name(&names, others, skills);
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
    let (already, picked): (Vec<usize>, Vec<usize>) =
        (picked.into_iter()).partition(|&i| loaded.iter().any(|name| *name == others[i].name));
    let found = Found {
        picked: &picked,
        already: &already,
        unknown: &unknown,
    };
    let mut text = answer(&found, others, !skilled.is_empty());
    for index in skilled {
        text.push_str("\n\n");
        text.push_str(&skills::body(&skills[index]));
    }
    ToolOutput::ok(text)
}

/// [`run_with`] that also matches `mcp`, the MCP tools past the cap
/// (PLAN.md D47): matched ones are listed after the tool lines, and
/// `mcp_describe` and `mcp_call` load with them.
pub fn run_full(
    args: &Value,
    others: &[ToolSpec],
    skills: &[Skill],
    mcp: &[ToolSpec],
    loaded: &[String],
) -> ToolOutput {
    if mcp.is_empty() {
        return run_loaded(args, others, skills, loaded);
    }
    let names: Vec<&str> = (args.get("names").and_then(Value::as_array))
        .map(|list| list.iter().filter_map(Value::as_str).collect())
        .unwrap_or_default();
    let query = args.get("query").and_then(Value::as_str).unwrap_or("");
    let (mut hits, mut rest): (Vec<usize>, Vec<&str>) = (Vec::new(), Vec::new());
    for name in names {
        match mcp.iter().position(|t| t.name.eq_ignore_ascii_case(name)) {
            Some(index) if !hits.contains(&index) => hits.push(index),
            Some(_) => {}
            None => rest.push(name),
        }
    }
    for index in search(query, mcp) {
        if !hits.contains(&index) {
            hits.push(index);
        }
    }
    hits.truncate(MAX_RESULTS);
    if hits.is_empty() {
        return run_loaded(args, others, skills, loaded);
    }
    let mut names = vec!["mcp_describe", "mcp_call"];
    names.extend(rest);
    let mut args = args.clone();
    args["names"] = json!(names);
    let output = run_loaded(&args, others, skills, loaded);
    let text = output.text().into_owned() + &mcp_lines(&hits, mcp);
    ToolOutput::new(text, output.is_error())
}

/// The matched MCP tools with the first line of each description.
fn mcp_lines(hits: &[usize], mcp: &[ToolSpec]) -> String {
    let mut text = String::from(
        "\n\nMCP tools matched (not in the tool list: mcp_describe shows one's schema, \
        mcp_call runs it with `name` and `arguments`):",
    );
    for &index in hits {
        let tool = &mcp[index];
        let line = tool.description.lines().next().unwrap_or("");
        let short: String = line.chars().take(200).collect();
        text.push_str(&format!("\n- {}: {short}", tool.name));
    }
    text
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

/// The tools and skills `names` name, in order without repeats, and the
/// names matching neither.
fn by_name<'a>(
    names: &[&'a str],
    others: &[ToolSpec],
    skills: &[Skill],
) -> (Vec<usize>, Vec<usize>, Vec<&'a str>) {
    let (mut picked, mut skilled, mut unknown) = (Vec::new(), Vec::new(), Vec::new());
    for &name in names {
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
    (picked, skilled, unknown)
}

/// The tools a find matched: new ones, loaded ones, unknown names.
struct Found<'a> {
    picked: &'a [usize],
    already: &'a [usize],
    unknown: &'a [&'a str],
}

fn answer(found: &Found<'_>, others: &[ToolSpec], skills: bool) -> String {
    let (picked, unknown) = (found.picked, found.unknown);
    let all: Vec<&str> = others.iter().map(|tool| tool.name.as_str()).collect();
    let mut lines = Vec::new();
    if picked.is_empty() && skills {
        lines.push("Skill instructions below (no tool loaded).".to_string());
    } else if picked.is_empty() && !found.already.is_empty() {
        // No `Loaded:` line: nothing new to send a schema for.
    } else if picked.is_empty() && unknown.iter().any(|name| !close(name, &all).is_empty()) {
        // The close names say more than the whole list.
        lines.push("No tool matched.".to_string());
    } else if picked.is_empty() {
        lines.push(format!("No tool matched. Tools: {}.", all.join(", ")));
    } else {
        let found: Vec<&str> = picked.iter().map(|&i| others[i].name.as_str()).collect();
        lines.push(loaded_line(&found));
        lines.push(CALL_NOW.to_string());
    }
    for &index in found.already {
        lines.push(format!("{}: already loaded", others[index].name));
    }
    if !unknown.is_empty() {
        let named: Vec<String> = unknown.iter().map(|name| hinted(name, &all)).collect();
        lines.push(format!("Unknown: {}.", named.join(", ")));
    }
    lines.join("\n")
}

/// `name`, with `(did you mean a or b?)` when tools are named close to it.
fn hinted(name: &str, all: &[&str]) -> String {
    match close(name, all).as_slice() {
        [] => name.to_string(),
        near => format!("{name} (did you mean {}?)", near.join(" or ")),
    }
}

/// At most three of `all` close to `name`, closest first: within a third of
/// its length in edits (at least 2), or holding it whole (3 letters or more).
fn close<'a>(name: &str, all: &[&'a str]) -> Vec<&'a str> {
    let name = name.to_lowercase();
    let length = name.chars().count();
    let most = (length / 3).max(2);
    let mut near: Vec<(usize, &str)> = (all.iter())
        .map(|&tool| (edits(&name, &tool.to_lowercase()), tool))
        .filter(|&(edits, tool)| {
            edits <= most || (length >= 3 && tool.to_lowercase().contains(&name))
        })
        .collect();
    near.sort_by_key(|&(edits, _)| edits);
    near.into_iter().take(3).map(|(_, tool)| tool).collect()
}

/// The Levenshtein distance of `a` and `b`, in characters.
fn edits(a: &str, b: &str) -> usize {
    let b: Vec<char> = b.chars().collect();
    let mut row: Vec<usize> = (0..=b.len()).collect();
    for (i, x) in a.chars().enumerate() {
        let mut diagonal = row[0];
        row[0] = i + 1;
        for (j, &y) in b.iter().enumerate() {
            let above = row[j + 1];
            row[j + 1] = (above + 1)
                .min(row[j] + 1)
                .min(diagonal + usize::from(x != y));
            diagonal = above;
        }
    }
    row[b.len()]
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
