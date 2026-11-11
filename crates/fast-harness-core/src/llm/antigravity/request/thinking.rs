//! `generationConfig`: `maxOutputTokens` capped by the model's max output and
//! the thinking config of the model's family (DSH antigravity-switch
//! `fixThinking`, effort mapping in [`crate::llm::antigravity::models`]).

use serde_json::{json, Map, Value};

use crate::llm::antigravity::models::{
    self, family_of, id_level, CatalogEntry, Family, CLAUDE_BUDGETS, GEMINI_BUDGETS,
};

/// Claude needs more output room than its thinking budget.
pub const CLAUDE_THINKING_MAX_OUTPUT: u64 = 64_000;
/// Claude thinking budget when nothing else names one.
pub const DEFAULT_THINKING_BUDGET: u64 = 16_000;

/// The generation config for bare id `model`, asked `max_tokens` and `effort`.
pub fn generation_config(model: &str, max_tokens: u32, effort: Option<&str>) -> Value {
    let entry = models::lookup(model);
    let mut max = u64::from(max_tokens);
    if let Some(cap) = entry.as_ref().and_then(|e| e.max_output) {
        max = max.min(cap);
    }
    let effort = models::fit_effort(model, effort);
    let effort = effort.as_deref();
    let mut config = Map::new();
    let thinking = match family_of(model, entry.as_ref()) {
        Family::NoThinking => None,
        Family::GeminiLevel { .. } => Some(gemini_level(model, effort, entry.as_ref())),
        Family::GeminiBudget { .. } => Some(gemini_budget(effort, entry.as_ref())),
        Family::ClaudeAdaptive => claude_adaptive(model, effort, entry.as_ref()),
        Family::ClaudeBudget => {
            let budget = claude_budget(model, effort, entry.as_ref());
            if let Some(budget) = budget {
                if max <= budget {
                    max = CLAUDE_THINKING_MAX_OUTPUT;
                }
            }
            budget.map(|b| json!({"include_thoughts": true, "thinking_budget": b}))
        }
    };
    config.insert("maxOutputTokens".into(), json!(max));
    if let Some(thinking) = thinking {
        config.insert("thinkingConfig".into(), thinking);
    }
    Value::Object(config)
}

/// Gemini 3+: the id's level, else the effort's, else the catalog's, else `high`.
fn gemini_level(model: &str, effort: Option<&str>, entry: Option<&CatalogEntry>) -> Value {
    let level = id_level(model)
        .map(str::to_string)
        .or_else(|| effort.map(str::to_string))
        .or_else(|| entry.and_then(|e| e.thinking_level.clone()))
        .unwrap_or_else(|| "high".to_string());
    json!({"includeThoughts": true, "thinkingLevel": level})
}

/// Gemini 2.x: budget 0 for `off`, the effort's budget, else the model's default.
fn gemini_budget(effort: Option<&str>, entry: Option<&CatalogEntry>) -> Value {
    if effort == Some("off") {
        return json!({"thinkingBudget": 0});
    }
    let budget = effort
        .and_then(|word| table(&GEMINI_BUDGETS, word))
        .or_else(|| entry.and_then(|e| e.thinking_budget));
    match budget {
        Some(budget) => json!({"includeThoughts": true, "thinkingBudget": budget}),
        None => json!({"includeThoughts": true}),
    }
}

/// Adaptive Claude: no config for `off`; the id's level, the effort's, the
/// catalog's, else `high` (`minimal` reads as `low`).
fn claude_adaptive(
    model: &str,
    effort: Option<&str>,
    entry: Option<&CatalogEntry>,
) -> Option<Value> {
    if effort == Some("off") {
        return None;
    }
    let level = id_level(model)
        .map(str::to_string)
        .or_else(|| effort.map(str::to_string))
        .or_else(|| entry.and_then(|e| e.thinking_level.clone()))
        .unwrap_or_else(|| "high".to_string());
    let level = if level == "minimal" {
        "low".to_string()
    } else {
        level
    };
    Some(json!({"includeThoughts": true, "thinkingLevel": level}))
}

/// Budget Claude: `None` for `off`; the id tier's budget, the effort's, the
/// catalog's, else 16000.
fn claude_budget(model: &str, effort: Option<&str>, entry: Option<&CatalogEntry>) -> Option<u64> {
    if effort == Some("off") {
        return None;
    }
    let budget = id_level(model)
        .and_then(|level| table(&CLAUDE_BUDGETS, level))
        .or_else(|| effort.and_then(|word| table(&CLAUDE_BUDGETS, word)))
        .or_else(|| entry.and_then(|e| e.thinking_budget))
        .unwrap_or(DEFAULT_THINKING_BUDGET);
    Some(budget)
}

fn table(rows: &[(&str, u64)], word: &str) -> Option<u64> {
    rows.iter().find(|(name, _)| *name == word).map(|(_, n)| *n)
}
