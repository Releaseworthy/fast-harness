//! Per-model facts across both providers (PLAN.md D24): an `antigravity/<id>`
//! model belongs to Google Antigravity, any other id to Claude. Front ends ask
//! here instead of choosing a provider module themselves.

use super::anthropic::{self, ModelInfo};
use super::antigravity;

/// Provider id of Claude models (the trace's `provider` field).
pub const ANTHROPIC: &str = "anthropic";
/// Provider id of `antigravity/<id>` models.
pub const ANTIGRAVITY: &str = "antigravity";
/// Providers in picker order.
pub const PROVIDERS: [&str; 2] = [ANTHROPIC, ANTIGRAVITY];

/// The provider id that serves `model`.
pub fn provider_of(model: &str) -> &'static str {
    if antigravity::is_antigravity(model) {
        ANTIGRAVITY
    } else {
        ANTHROPIC
    }
}

/// The picker group title of a provider id.
pub fn group_title(provider: &str) -> &'static str {
    match provider {
        ANTIGRAVITY => "Google Antigravity",
        _ => "Claude",
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
    PROVIDERS
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
    if antigravity::is_antigravity(model) {
        antigravity::models::fit_effort(model, effort)
    } else {
        anthropic::fit_effort(model, effort)
    }
}

/// The display name of `model` from the remembered catalog or the built-in
/// table; `None` when unknown.
pub fn display_name(model: &str) -> Option<String> {
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
