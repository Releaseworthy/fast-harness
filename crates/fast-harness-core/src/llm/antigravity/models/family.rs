//! Model families and the effort words each takes (PLAN.md "Antigravity
//! provider"). Our words and what Google gets:
//!
//! - Gemini 3 and later (`thinkingLevel`): `minimal` (flash and lite only),
//!   `low`, `medium`, `high` go as the same level; `gemini-3-pro*` takes only
//!   `low` and `high`. Thinking cannot be switched off: `off` asks for the
//!   lowest level the model takes.
//! - Gemini 2.x (`thinkingBudget`): `low` 8192, `medium` 16384, `high` 24576
//!   tokens; `off` is budget 0, not offered for `pro`.
//! - Claude with adaptive thinking (catalog `supportsAdaptiveThinking`, else
//!   Claude 5 and later by id): `low`, `medium`, `high` as `thinkingLevel`;
//!   `off` sends no thinking config.
//! - Other thinking Claude: `low` 8192, `medium` 16384, `high` 32768 as
//!   `thinking_budget`; `off` sends no thinking config.
//! - `xhigh` and `max` ask for the highest level a model takes.
//! - A `gemini-`/`claude-` id ending in `-minimal|-low|-medium|-high` thinks at
//!   that level whatever the effort, and offers no effort.

use super::{lookup, CatalogEntry, PREFIX};

/// Gemini 2.x budgets for `low`, `medium`, `high`.
pub const GEMINI_BUDGETS: [(&str, u64); 3] = [("low", 8192), ("medium", 16384), ("high", 24576)];
/// Claude budgets for `minimal`, `low`, `medium`, `high` (DSH antigravity-switch tiers).
pub const CLAUDE_BUDGETS: [(&str, u64); 4] = [
    ("minimal", 4096),
    ("low", 8192),
    ("medium", 16384),
    ("high", 32768),
];

/// How a model is asked to think.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Family {
    /// Gemini 3 and later: `thinkingLevel`.
    GeminiLevel { flash_like: bool },
    /// Other Gemini: `thinkingBudget`.
    GeminiBudget { flash_like: bool },
    /// Claude with adaptive thinking: `thinkingLevel`.
    ClaudeAdaptive,
    /// Claude with `thinking_budget`.
    ClaudeBudget,
    /// A model sent no thinking config (Claude that does not think, GPT-OSS, others).
    NoThinking,
}

pub(super) fn is_level(word: &str) -> bool {
    matches!(word, "minimal" | "low" | "medium" | "high")
}

/// The thinking level carried in a `gemini-`/`claude-` id suffix.
pub fn id_level(model: &str) -> Option<&'static str> {
    let lower = model.to_lowercase();
    if !(lower.starts_with("gemini-") || lower.starts_with("claude-")) {
        return None;
    }
    ["minimal", "low", "medium", "high"]
        .into_iter()
        .find(|level| lower.ends_with(&format!("-{level}")))
}

/// The major version in `gemini-<n>` or `claude-<name>-<n>`.
fn major(lower: &str, prefix: &str) -> Option<u32> {
    let rest = lower.strip_prefix(prefix)?;
    let rest = if prefix == "claude-" {
        rest.split_once('-')?.1
    } else {
        rest
    };
    let digits: String = rest.chars().take_while(char::is_ascii_digit).collect();
    digits.parse().ok()
}

/// The family of `model` (bare or `antigravity/` id), from `entry` when given.
pub fn family_of(model: &str, entry: Option<&CatalogEntry>) -> Family {
    let lower = model.strip_prefix(PREFIX).unwrap_or(model).to_lowercase();
    let flash_like = lower.contains("flash") || lower.contains("lite");
    if lower.starts_with("gemini-") {
        return match major(&lower, "gemini-") {
            Some(n) if n >= 3 => Family::GeminiLevel { flash_like },
            _ => Family::GeminiBudget { flash_like },
        };
    }
    if !lower.starts_with("claude-") || lower == "claude-sonnet-4-6" {
        return Family::NoThinking;
    }
    let by_id = major(&lower, "claude-").is_some_and(|n| n >= 5);
    let adaptive = entry
        .and_then(|e| e.supports_adaptive_thinking)
        .unwrap_or(by_id);
    let thinks = entry.and_then(|e| e.supports_thinking) == Some(true)
        || lower.contains("thinking")
        || id_level(&lower).is_some()
        || (adaptive && entry.and_then(|e| e.supports_thinking).is_none());
    match (thinks, adaptive) {
        (false, _) => Family::NoThinking,
        (true, true) => Family::ClaudeAdaptive,
        (true, false) => Family::ClaudeBudget,
    }
}

/// The family of `model` with its remembered catalog entry.
pub fn family(model: &str) -> Family {
    family_of(model, lookup(model).as_ref())
}

/// Effort words `model` (bare or `antigravity/` id) takes, lowest first,
/// then `off` when thinking can be switched off; empty when none.
pub fn efforts(model: &str) -> Vec<String> {
    efforts_of(model, lookup(model).as_ref())
}

pub(super) fn efforts_of(model: &str, entry: Option<&CatalogEntry>) -> Vec<String> {
    let bare = model.strip_prefix(PREFIX).unwrap_or(model).to_lowercase();
    if id_level(&bare).is_some() {
        return Vec::new();
    }
    let words: &[&str] = match family_of(&bare, entry) {
        Family::GeminiLevel { flash_like: true } => &["minimal", "low", "medium", "high"],
        Family::GeminiLevel { .. } if bare.starts_with("gemini-3-pro") => &["low", "high"],
        Family::GeminiLevel { .. } => &["low", "medium", "high"],
        Family::GeminiBudget { .. } if bare.contains("pro") => &["low", "medium", "high"],
        Family::GeminiBudget { .. } | Family::ClaudeAdaptive | Family::ClaudeBudget => {
            &["low", "medium", "high", "off"]
        }
        Family::NoThinking => &[],
    };
    words.iter().map(|word| word.to_string()).collect()
}

/// The effort word `model` takes for `effort`: itself when listed; `off`
/// as the lowest level when thinking cannot be switched off; `xhigh`, `max`
/// and unknown words as `high` (else the highest); `None` when `effort` is
/// `None` or the model takes no effort.
pub fn fit_effort(model: &str, effort: Option<&str>) -> Option<String> {
    let wanted = effort?;
    let levels = efforts(model);
    let thinking: Vec<&String> = levels.iter().filter(|level| *level != "off").collect();
    if levels.iter().any(|level| level == wanted) {
        return Some(wanted.to_string());
    }
    if wanted == "off" {
        return thinking.first().map(|level| level.to_string());
    }
    if levels.iter().any(|level| level == "high") {
        return Some("high".to_string());
    }
    thinking.last().map(|level| level.to_string())
}
