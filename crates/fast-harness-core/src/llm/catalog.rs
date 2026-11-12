//! Per-model facts across the providers (PLAN.md D24): a `<provider id>/<id>`
//! model of `providers/*.json` belongs to that provider, any other id to
//! Claude. Front ends ask here instead of choosing a provider module themselves.
//! `antigravity/<id>` models moved to branch `antigravity` (PLAN.md D53): such
//! a value falls back to the default model with a note ([`retired_note`]).

use super::anthropic::{self, ModelInfo};
use super::custom;

/// Provider id of Claude models (the trace's `provider` field).
pub const ANTHROPIC: &str = "anthropic";
/// Providers in picker order.
pub const PROVIDERS: [&str; 1] = [ANTHROPIC];

/// The prefix of the Google Antigravity models that left this branch (D53).
pub const RETIRED_PREFIX: &str = "antigravity/";

/// The provider id that serves `model`: a provider of `providers/*.json` by
/// its prefix (PLAN.md D41), else Claude.
pub fn provider_of(model: &str) -> &'static str {
    custom::provider_of(model).map_or(ANTHROPIC, |(id, _)| id)
}

/// Whether `model` is an `antigravity/<id>` model, gone from this branch.
pub fn is_retired(model: &str) -> bool {
    model.starts_with(RETIRED_PREFIX)
}

/// The note shown when `model` is retired and `fallback` runs instead.
pub fn retired_note(model: &str, fallback: &str) -> Option<String> {
    is_retired(model).then(|| {
        format!(
            "{model} is not available: Google Antigravity moved to branch `antigravity` \
             (PLAN.md D53); using {fallback}"
        )
    })
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
        ANTHROPIC => "Claude",
        other => custom::title_of(other).unwrap_or("Claude"),
    }
}

/// The label of `model` named `name` where the provider is not otherwise
/// shown (a closed picker, a status line).
pub fn label(_model: &str, name: &str) -> String {
    name.to_string()
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
    anthropic::efforts(model)
        .into_iter()
        .map(String::from)
        .collect()
}

/// The effort `model` takes for `effort` (see each provider's `fit_effort`).
pub fn fit_effort(model: &str, effort: Option<&str>) -> Option<String> {
    if custom::provider_of(model).is_some() {
        let known = efforts(model);
        return effort
            .filter(|e| known.iter().any(|k| k == e))
            .map(String::from);
    }
    anthropic::fit_effort(model, effort)
}

/// The context window of `model` in tokens from the remembered catalog or
/// the built-in table; 0 when unknown.
pub fn context_window(model: &str) -> u64 {
    if let Some(window) = custom::model(model).and_then(|m| m.context_window) {
        return window;
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

    #[test]
    fn labels_are_the_name() {
        assert_eq!(
            label("claude-sonnet-5-5", "Claude Sonnet 5.5"),
            "Claude Sonnet 5.5"
        );
    }

    #[test]
    fn dispatch_by_prefix() {
        assert_eq!(provider_of("claude-opus-5-5"), ANTHROPIC);
        assert_eq!(efforts("claude-opus-5-5").last().unwrap(), "max");
        assert_eq!(
            fit_effort("claude-opus-4-5", Some("max")).as_deref(),
            Some("high")
        );
        assert_eq!(
            display_name("claude-opus-5-5").as_deref(),
            Some("Claude Opus 5.5")
        );
    }

    #[test]
    fn antigravity_models_are_retired_with_a_note() {
        assert!(is_retired("antigravity/gemini-3-flash"));
        assert!(!is_retired("claude-opus-5-5"));
        let note = retired_note("antigravity/gemini-3-flash", "claude-opus-5-5").unwrap();
        assert!(note.contains("branch `antigravity`") && note.contains("claude-opus-5-5"));
        assert_eq!(retired_note("claude-opus-5-5", "claude-opus-5-5"), None);
    }
}
