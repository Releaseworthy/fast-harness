//! Which models the `subagent` tool may start children on, and its model
//! guide (PLAN.md D82): `subagent.models` turns models off (all on by
//! default), `subagent.guide` is appended to the tool's description.

use serde_json::{json, Value};

use crate::config::SubagentConfig;
use crate::llm::Catalog;

/// Model family words a guide line may name.
const FAMILIES: [&str; 4] = ["haiku", "sonnet", "opus", "fable"];

/// The catalog's models a child may run on, in catalog order.
pub fn enabled(catalog: &Catalog, settings: &SubagentConfig) -> Vec<String> {
    let mut ids = catalog.model_ids();
    ids.retain(|id| settings.allows(id));
    ids
}

/// The `model` property of the tool schema: an `enum` of `enabled`.
pub fn model_schema(enabled: &[String]) -> Value {
    let mut prop = json!({"type": "string", "description": "Model id; default: the parent's."});
    if !enabled.is_empty() {
        prop["enum"] = json!(enabled);
    }
    prop
}

/// `guide` without the lines that name only model families none of
/// `enabled` belongs to; "" when nothing is left but the heading.
pub fn guide(guide: &str, enabled: &[String]) -> String {
    let on = |family: &str| enabled.iter().any(|id| id.to_lowercase().contains(family));
    let lines: Vec<&str> = guide
        .lines()
        .filter(|line| {
            let lower = line.to_lowercase();
            let named: Vec<&str> = FAMILIES
                .iter()
                .copied()
                .filter(|f| lower.contains(f))
                .collect();
            named.is_empty() || named.iter().any(|f| on(f))
        })
        .collect();
    let any_model = lines.iter().any(|line| {
        let lower = line.to_lowercase();
        FAMILIES.iter().any(|f| lower.contains(f))
    });
    let text = lines.join("\n");
    let mentions = FAMILIES.iter().any(|f| guide.to_lowercase().contains(f));
    match mentions && !any_model {
        true => String::new(),
        false => text.trim().to_string(),
    }
}

/// `Err` with a clear note when `model` (the call's, else the default
/// `default_model`) is turned off for subagents.
pub fn check(
    settings: &SubagentConfig,
    catalog: &Catalog,
    asked: Option<&str>,
    default_model: &str,
) -> Result<(), String> {
    let model = asked.unwrap_or(default_model).trim();
    if settings.allows(model) {
        return Ok(());
    }
    let list = enabled(catalog, settings);
    let list = match list.is_empty() {
        true => "none".to_string(),
        false => list.join(", "),
    };
    let what = match asked {
        Some(_) => format!("model `{model}` is disabled for subagents"),
        None => format!("the default subagent model `{model}` is disabled; pass `model`"),
    };
    Err(format!(
        "{what} (setting subagent.models); enabled models: {list}"
    ))
}

#[cfg(test)]
#[path = "spawn_models_tests.rs"]
mod tests;
