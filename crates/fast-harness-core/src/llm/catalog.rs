//! Per-model facts across both providers (PLAN.md D24): an `antigravity/<id>`
//! model belongs to Google Antigravity, any other id to Claude. Front ends ask
//! here instead of choosing a provider module themselves.

use super::anthropic::{self, ModelInfo};
use super::antigravity;
use super::custom;

/// Provider id of Claude models (the trace's `provider` field).
pub const ANTHROPIC: &str = "anthropic";
/// Provider id of `antigravity/<id>` models.
pub const ANTIGRAVITY: &str = "antigravity";
/// Providers in picker order.
pub const PROVIDERS: [&str; 2] = [ANTHROPIC, ANTIGRAVITY];

/// The provider id that serves `model`: `antigravity/<id>`, a provider of
/// `providers/*.json` by its prefix (PLAN.md D41), else Claude.
pub fn provider_of(model: &str) -> &'static str {
    if antigravity::is_antigravity(model) {
        return ANTIGRAVITY;
    }
    custom::provider_of(model).map_or(ANTHROPIC, |(id, _)| id)
}

/// Every provider in picker order: the built-in ones, then the custom ones.
pub fn providers() -> Vec<&'static str> {
    let mut out = PROVIDERS.to_vec();
    out.extend(custom::custom_ids());
    out
}

/// The picker group title of a provider id.
pub fn group_title(provider: &str) -> &'static str {
    match provider {
        ANTIGRAVITY => "Google Antigravity",
        ANTHROPIC => "Claude",
        other => custom::title_of(other).unwrap_or("Claude"),
    }
}

/// The label of `model` named `name` where the provider is not otherwise
/// shown (a closed picker, a status line): Antigravity models whose name does
/// not say Gemini get " · Antigravity", so Claude through Google never reads
/// like Claude through Anthropic.
pub fn label(model: &str, name: &str) -> String {
    let lower = name.to_ascii_lowercase();
    // A name that is the id itself already says antigravity.
    let plain = lower.starts_with("gemini") || lower.contains("antigravity");
    if antigravity::is_antigravity(model) && !plain {
        format!("{name} · Antigravity")
    } else {
        name.to_string()
    }
}

/// `models` split by provider, in [`PROVIDERS`] order, keeping the order
/// inside each group; empty groups are left out.
pub fn grouped(models: &[ModelInfo]) -> Vec<(&'static str, Vec<&ModelInfo>)> {
    providers()
        .iter()
        .map(|provider| {
            let group: Vec<&ModelInfo> = models
                .iter()
                .filter(|m| provider_of(&m.id) == *provider)
                .collect();
            (*provider, group)
        })
        .filter(|(_, group)| !group.is_empty())
        .collect()
}

/// Effort words `model` takes, lowest first; empty when none.
pub fn efforts(model: &str) -> Vec<String> {
    if custom::provider_of(model).is_some() {
        return custom::model(model).map(|m| m.efforts).unwrap_or_default();
    }
    if antigravity::is_antigravity(model) {
        antigravity::models::efforts(model)
    } else {
        anthropic::efforts(model)
            .into_iter()
            .map(String::from)
            .collect()
    }
}

/// The effort `model` takes for `effort` (see each provider's `fit_effort`).
pub fn fit_effort(model: &str, effort: Option<&str>) -> Option<String> {
    if custom::provider_of(model).is_some() {
        let known = efforts(model);
        return effort
            .filter(|e| known.iter().any(|k| k == e))
            .map(String::from);
    }
    if antigravity::is_antigravity(model) {
        antigravity::models::fit_effort(model, effort)
    } else {
        anthropic::fit_effort(model, effort)
    }
}

/// The context window of `model` in tokens from the remembered catalog or
/// the built-in table; 0 when unknown.
pub fn context_window(model: &str) -> u64 {
    if let Some(window) = custom::model(model).and_then(|m| m.context_window) {
        return window;
    }
    if antigravity::is_antigravity(model) {
        return antigravity::models::lookup(model)
            .map(|entry| antigravity::models::to_info(&entry).context_window)
            .unwrap_or(0);
    }
    anthropic::context_window(model)
}

/// The display name of `model` from the remembered catalog or the built-in
/// table; `None` when unknown.
pub fn display_name(model: &str) -> Option<String> {
    if custom::provider_of(model).is_some() {
        let entry = custom::model(model)?;
        return Some(entry.name.unwrap_or(entry.id));
    }
    if antigravity::is_antigravity(model) {
        return antigravity::models::lookup(model)
            .map(|entry| entry.display_name)
            .filter(|name| !name.is_empty());
    }
    anthropic::builtin_models()
        .into_iter()
        .find(|m| m.id == model)
        .map(|m| m.display_name)
}

/// The `maxOutput` of `model`'s entry in `providers/*.json`, when set: the
/// request's `max_tokens` stays at or below it.
pub fn max_output_override(model: &str) -> Option<u64> {
    custom::model(model)
        .and_then(|m| m.max_output)
        .filter(|n| *n > 0)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::llm::antigravity::models::{remember, CatalogEntry};

    #[test]
    fn labels_tell_antigravity_claude_apart() {
        assert_eq!(
            label("antigravity/claude-sonnet-4-6", "Claude Sonnet 4.6"),
            "Claude Sonnet 4.6 · Antigravity"
        );
        assert_eq!(
            label("antigravity/gemini-3.8-flash-high", "Gemini 3.8 Flash"),
            "Gemini 3.8 Flash"
        );
        assert_eq!(
            label("claude-sonnet-5-5", "Claude Sonnet 5.5"),
            "Claude Sonnet 5.5"
        );
    }

    #[test]
    fn dispatch_by_prefix() {
        assert_eq!(provider_of("claude-opus-5-5"), ANTHROPIC);
        assert_eq!(provider_of("antigravity/claude-opus-5-5"), ANTIGRAVITY);
        assert_eq!(group_title(ANTIGRAVITY), "Google Antigravity");
        assert_eq!(efforts("claude-opus-5-5").last().unwrap(), "max");
        assert_eq!(efforts("antigravity/gemini-3-pro-catalog"), ["low", "high"]);
        assert_eq!(
            fit_effort("antigravity/gemini-3-pro-catalog", Some("max")).as_deref(),
            Some("high")
        );
        assert_eq!(
            fit_effort("claude-opus-4-5", Some("max")).as_deref(),
            Some("high")
        );
        remember(&[CatalogEntry {
            id: "gemini-catalog-name".into(),
            display_name: "Gemini Catalog".into(),
            ..CatalogEntry::default()
        }]);
        let name = display_name("antigravity/gemini-catalog-name");
        assert_eq!(name.as_deref(), Some("Gemini Catalog"));
        assert_eq!(display_name("antigravity/unknown"), None);
        assert_eq!(
            display_name("claude-opus-5-5").as_deref(),
            Some("Claude Opus 5.5")
        );
    }
}
