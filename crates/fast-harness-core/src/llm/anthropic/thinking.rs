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

use super::live::{LiveCaps, LiveModels};
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

/// The plan for `model`: a table model as [`plan`] decides; a model missing
/// from the table, when the last model list (or the cache) gave its
/// capabilities, as the API says ([`plan_live`]); else [`plan`]'s default.
pub(crate) fn plan_model(
    live: &LiveModels,
    model: &str,
    spec: Option<&ModelSpec>,
    effort: Option<&str>,
    max_tokens: u32,
) -> Plan {
    if spec.is_none() {
        if let Some(info) = live.lookup(model) {
            if let Some(caps) = info.live {
                let model_max = u32::try_from(info.max_output).unwrap_or(u32::MAX);
                let model_max = if model_max == 0 {
                    DEFAULT_MAX_TOKENS
                } else {
                    model_max
                };
                return plan_live(&caps, &info.efforts, model_max, effort, max_tokens);
            }
        }
    }
    plan(spec, effort, max_tokens)
}

/// Adaptive thinking (with `output_config.effort` when the model takes
/// effort) when the API lists adaptive, budget thinking when it lists only
/// `enabled`, no `thinking` field when the model cannot think.
fn plan_live(
    caps: &LiveCaps,
    levels: &[String],
    model_max: u32,
    effort: Option<&str>,
    max_tokens: u32,
) -> Plan {
    let base = if max_tokens == 0 {
        model_max.min(DEFAULT_MAX_TOKENS)
    } else {
        max_tokens
    };
    let mut plan = Plan {
        max_tokens: base,
        ..Plan::default()
    };
    let effort_config = |level: &str| json!({"effort": live_effort(levels, level)});
    match effort {
        Some("off") if caps.can_disable => plan.thinking = Some(json!({"type": "disabled"})),
        Some("off") => {}
        None if caps.adaptive => {
            plan.thinking = Some(json!({"type": "adaptive", "display": "summarized"}))
        }
        Some(level) if caps.adaptive => {
            plan.thinking = Some(json!({"type": "adaptive", "display": "summarized"}));
            plan.output_config = caps.effort.then(|| effort_config(level));
        }
        Some(level) if caps.budget => budget(&mut plan, model_max, level),
        Some(level) if caps.effort => plan.output_config = Some(effort_config(level)),
        _ => {}
    }
    plan
}

/// `level` when the model lists it, else `high`.
fn live_effort(levels: &[String], level: &str) -> String {
    let listed = levels.iter().any(|known| known == level);
    if listed { level } else { "high" }.to_string()
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

    fn live_info(id: &str, caps: LiveCaps, levels: &[&str]) -> super::super::models::ModelInfo {
        super::super::models::ModelInfo {
            id: id.into(),
            display_name: id.into(),
            context_window: 0,
            max_output: 64_000,
            reasoning: true,
            efforts: levels.iter().map(|level| level.to_string()).collect(),
            deprecated: false,
            live: Some(caps),
        }
    }

    #[test]
    fn live_capabilities_plan_models_missing_from_the_table() {
        let adaptive = LiveCaps {
            effort: true,
            adaptive: true,
            budget: false,
            can_disable: true,
        };
        let budget_caps = LiveCaps {
            budget: true,
            ..LiveCaps::default()
        };
        let table_id = "claude-sonnet-4-5-20250929";
        let live = LiveModels::default();
        live.remember(&[
            live_info("claude-plan-adaptive", adaptive, &["low", "high", "off"]),
            live_info(
                "claude-plan-budget",
                budget_caps,
                &["low", "medium", "high"],
            ),
            live_info("claude-plan-none", LiveCaps::default(), &[]),
            live_info(table_id, LiveCaps::default(), &[]),
        ]);
        let p = plan_model(&live, "claude-plan-adaptive", None, Some("medium"), 0);
        assert_eq!(p.thinking.unwrap()["type"], "adaptive");
        assert_eq!(p.output_config, Some(json!({"effort": "high"})));
        assert_eq!(p.max_tokens, 32_000);
        let low = plan_model(&live, "claude-plan-adaptive", None, Some("low"), 0);
        assert_eq!(low.output_config, Some(json!({"effort": "low"})));
        let off = plan_model(&live, "claude-plan-adaptive", None, Some("off"), 0);
        assert_eq!(off.thinking, Some(json!({"type": "disabled"})));
        let none = plan_model(&live, "claude-plan-adaptive", None, None, 0);
        assert_eq!((none.thinking.is_some(), none.output_config), (true, None));
        let b = plan_model(&live, "claude-plan-budget", None, Some("high"), 1000);
        assert_eq!(b.thinking.unwrap()["type"], "enabled");
        assert_eq!(b.betas, [wire::BETA_INTERLEAVED_THINKING]);
        assert_eq!(
            plan_model(&live, "claude-plan-budget", None, None, 0).thinking,
            None
        );
        assert_eq!(
            plan_model(&live, "claude-plan-none", None, Some("high"), 0),
            Plan {
                max_tokens: 32_000,
                ..Plan::default()
            }
        );
        let table = plan_model(&live, table_id, find(table_id), Some("high"), 1000);
        assert_eq!(table.thinking.unwrap()["type"], "enabled");
        let unknown = plan_model(&live, "claude-plan-unknown", None, Some("max"), 5);
        assert_eq!(unknown.output_config, Some(json!({"effort": "max"})));
    }
}
