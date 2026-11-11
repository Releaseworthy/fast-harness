//! Thinking and effort per model, as pi-ai's `buildParams`, `streamSimple`
//! and `mapThinkingLevelToEffort` choose them (read 2026-11-11). The
//! `thinking: {type: adaptive, display: summarized}` shape matches the
//! streaming docs example (<https://platform.claude.com/docs/en/build-with-claude/streaming>,
//! read 2026-11-11). The default thinking budgets (1024, 2048, 8192, 16384)
//! and the 32000 default cap are pi-ai's `simple-options` values, which are
//! not in the reference bundle: unverified against the current pi-ai.
//!
//! `ChatRequest::effort`:
//! - `None`: the model default. Budget and unknown models get no `thinking`
//!   field; adaptive models get adaptive thinking without an effort; managed
//!   models get pi-ai's default effort `high`.
//! - `"off"`: `thinking: {type: disabled}`, except where pi-ai marks thinking
//!   as impossible to switch off (`thinkingLevelMap.off: null`): those models
//!   keep their default.
//! - a level: mapped like pi-ai: `minimal` and `low` to `low`, `medium`,
//!   `high`, and `xhigh` or `max` only where the model has them (else `high`).

use serde_json::{json, Value};

use super::models::{ModelSpec, Thinking};
use super::wire;

/// pi-ai's default `max_tokens` cap when the caller sets none.
const DEFAULT_MAX_TOKENS: u32 = 32_000;
/// Output tokens kept free next to a thinking budget.
const MIN_OUTPUT_TOKENS: u32 = 1024;

#[derive(Debug, Default, PartialEq)]
pub(crate) struct Plan {
    pub thinking: Option<Value>,
    pub output_config: Option<Value>,
    /// Managed models: effort of the trailing `system` message.
    pub trailing_effort: Option<String>,
    pub max_tokens: u32,
    pub betas: Vec<&'static str>,
}

pub(crate) fn map_effort(spec: Option<&ModelSpec>, level: &str) -> String {
    let extra = spec.map_or(&["xhigh", "max"][..], |spec| spec.extra_efforts);
    if extra.contains(&level) {
        return level.to_string();
    }
    match level {
        "minimal" | "low" => "low",
        "medium" => "medium",
        _ => "high",
    }
    .to_string()
}

/// pi-ai's default thinking budgets per level (`xhigh` and `max` use `high`).
fn budget_for(level: &str) -> u32 {
    match level {
        "minimal" => 1024,
        "low" => 2048,
        "medium" => 8192,
        _ => 16_384,
    }
}

pub(crate) fn plan(spec: Option<&ModelSpec>, effort: Option<&str>, max_tokens: u32) -> Plan {
    let model_max = spec.map_or(DEFAULT_MAX_TOKENS, |s| s.max_output as u32);
    let base = if max_tokens == 0 {
        model_max.min(DEFAULT_MAX_TOKENS)
    } else {
        max_tokens
    };
    let mut plan = Plan {
        max_tokens: base,
        ..Plan::default()
    };
    let adaptive = || json!({"type": "adaptive", "display": "summarized"});
    let disabled = || Some(json!({"type": "disabled"}));
    match (spec.map(|s| s.thinking), effort) {
        (Some(Thinking::Managed), _) => managed(&mut plan, spec, effort),
        (Some(Thinking::Adaptive), None) => plan.thinking = Some(adaptive()),
        (Some(_), Some("off")) if spec.is_some_and(|s| s.no_off) => {}
        (_, Some("off")) => plan.thinking = disabled(),
        (Some(Thinking::Budget), Some(level)) => budget(&mut plan, model_max, level),
        (_, Some(level)) => {
            plan.thinking = Some(adaptive());
            plan.output_config = Some(json!({"effort": map_effort(spec, level)}));
        }
        (_, None) => {}
    }
    plan
}

/// pi-ai `supportsMidConvoEffort`: always adaptive with prefix-mismatch
/// dropping, top-level effort `high`, the chosen effort in a trailing system message.
fn managed(plan: &mut Plan, spec: Option<&ModelSpec>, effort: Option<&str>) {
    plan.thinking = Some(json!({
        "type": "adaptive",
        "display": "summarized",
        "block_binding": {"prefix_mismatch_behavior": "drop_block"},
    }));
    plan.output_config = Some(json!({"effort": "high"}));
    let level = effort.filter(|level| *level != "off").unwrap_or("high");
    plan.trailing_effort = Some(map_effort(spec, level));
    plan.betas = vec![
        wire::BETA_MID_CONVERSATION_OUTPUT_CONFIG,
        wire::BETA_THINKING_BINDING_CONTROLS,
    ];
}

/// Budget thinking for older models: `max_tokens` grows by the budget up to
/// the model's cap and the budget leaves room for an answer.
fn budget(plan: &mut Plan, model_max: u32, level: &str) {
    let mut budget = budget_for(level);
    let max_tokens = plan.max_tokens.saturating_add(budget).min(model_max);
    if max_tokens <= budget {
        budget = max_tokens.saturating_sub(MIN_OUTPUT_TOKENS);
    }
    let budget = budget.min(max_tokens.saturating_sub(MIN_OUTPUT_TOKENS));
    let budget = if budget == 0 { 1024 } else { budget };
    plan.max_tokens = max_tokens;
    plan.thinking =
        Some(json!({"type": "enabled", "budget_tokens": budget, "display": "summarized"}));
    plan.betas = vec![wire::BETA_INTERLEAVED_THINKING];
}

#[cfg(test)]
mod tests {
    use super::super::models::find;
    use super::*;

    #[test]
    fn effort_mapping_per_model() {
        let opus46 = find("claude-opus-4-6");
        assert_eq!(map_effort(opus46, "max"), "max");
        assert_eq!(map_effort(opus46, "xhigh"), "high");
        assert_eq!(map_effort(opus46, "minimal"), "low");
        assert_eq!(map_effort(find("claude-opus-4-7"), "xhigh"), "xhigh");
    }

    #[test]
    fn adaptive_budget_and_off() {
        let opus47 = find("claude-opus-4-7");
        let p = plan(opus47, Some("medium"), 1000);
        assert_eq!(p.output_config, Some(json!({"effort": "medium"})));
        assert_eq!(plan(opus47, None, 0).max_tokens, 32_000);
        assert_eq!(
            plan(opus47, Some("off"), 0).thinking,
            Some(json!({"type": "disabled"}))
        );
        assert_eq!(plan(find("claude-fable-5"), Some("off"), 0).thinking, None);
        let haiku = plan(find("claude-haiku-4-5"), Some("high"), 60_000);
        assert_eq!(haiku.max_tokens, 64_000);
        assert_eq!(haiku.thinking.unwrap()["budget_tokens"], 16_384);
        assert_eq!(plan(find("claude-haiku-4-5"), None, 10).thinking, None);
        let managed = plan(find("claude-opus-5-5"), Some("off"), 0);
        assert_eq!(managed.trailing_effort.as_deref(), Some("high"));
        assert_eq!(
            plan(None, Some("max"), 5).output_config,
            Some(json!({"effort": "max"}))
        );
    }
}
