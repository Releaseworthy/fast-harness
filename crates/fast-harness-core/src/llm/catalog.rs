//! Per-model facts across the providers (PLAN.md D24): a `<provider id>/<id>`
//! model of `providers/*.json` belongs to that provider, any other id to
//! Claude. Front ends ask a [`Catalog`] instead of choosing a provider module
//! themselves. `antigravity/<id>` models moved to branch `antigravity`
//! (PLAN.md D53): such a value falls back to the default model with a note
//! ([`retired_note`]).
//!
//! A [`Catalog`] is a value owned by one `app::Runtime` (PLAN.md D67): the
//! provider files of the settings it loaded and its live model table. Two
//! runtimes in one process, or two projects of one runtime, never see each
//! other's providers.

use std::sync::Arc;

use super::anthropic::{self, LiveModels, ModelInfo};
use super::custom::{ModelFile, Price, ProviderFile};

/// Provider id of Claude models (the trace's `provider` field).
pub const ANTHROPIC: &str = "anthropic";
/// Providers in picker order.
pub const PROVIDERS: [&str; 1] = [ANTHROPIC];

/// The prefix of the Google Antigravity models that left this branch (D53).
pub const RETIRED_PREFIX: &str = "antigravity/";

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

/// The label of `model` named `name` where the provider is not otherwise
/// shown (a closed picker, a status line).
pub fn label(_model: &str, name: &str) -> String {
    name.to_string()
}

/// The provider files and the live model table of one runtime. Cheap to
/// clone (two `Arc`s): agents, model sources and front ends keep a copy.
/// The file list never changes in place: a settings reload makes a new
/// catalog ([`Catalog::with_files`]) that shares the live table, so an agent
/// built before keeps the providers it started with.
#[derive(Clone, Debug, Default)]
pub struct Catalog {
    files: Arc<Vec<ProviderFile>>,
    live: Arc<LiveModels>,
}

impl Catalog {
    /// `files` (a later file with the same id replaces the earlier one) and
    /// an empty live table.
    pub fn new(files: Vec<ProviderFile>) -> Catalog {
        Catalog::default().with_files(files)
    }

    /// A catalog of `files` sharing this one's live table.
    pub fn with_files(&self, files: Vec<ProviderFile>) -> Catalog {
        let mut out: Vec<ProviderFile> = Vec::with_capacity(files.len());
        for file in files {
            out.retain(|known| known.id != file.id);
            out.push(file);
        }
        Catalog {
            files: Arc::new(out),
            live: Arc::clone(&self.live),
        }
    }

    /// Every provider file, built-in overrides and custom providers.
    pub fn files(&self) -> &[ProviderFile] {
        &self.files
    }

    /// The live model table, shared with the runtime's Claude provider.
    pub fn live(&self) -> &Arc<LiveModels> {
        &self.live
    }

    /// The custom provider serving `model` (`<id>/<model id>`).
    pub fn custom(&self, model: &str) -> Option<&ProviderFile> {
        let (prefix, _) = model.split_once('/')?;
        (self.files.iter()).find(|p| p.is_custom() && p.id == prefix)
    }

    /// The provider id that serves `model`: a provider of `providers/*.json`
    /// by its prefix (PLAN.md D41), else Claude.
    pub fn provider_of(&self, model: &str) -> &str {
        self.custom(model).map_or(ANTHROPIC, |p| p.id.as_str())
    }

    /// Every model a subagent may name (PLAN.md D82): the built-in table,
    /// then models only the live list knows, then `<provider>/<id>` of the
    /// custom providers. Dated snapshots (`-20251101`) are left out.
    pub fn model_ids(&self) -> Vec<String> {
        let dated = |id: &str| {
            let tail = id.rsplit('-').next().unwrap_or_default();
            tail.len() == 8 && tail.bytes().all(|b| b.is_ascii_digit())
        };
        let table = anthropic::builtin_models().into_iter().map(|m| m.id);
        let mut out: Vec<String> = table.collect();
        for id in self.live.ids() {
            if !out.contains(&id) {
                out.push(id);
            }
        }
        out.retain(|id| !dated(id) && !is_retired(id));
        let custom = self.files.iter().filter(|p| p.is_custom());
        out.extend(custom.flat_map(|p| p.models.iter().map(move |m| format!("{}/{}", p.id, m.id))));
        out
    }

    /// Every provider in picker order: the built-in ones, then the custom ones.
    pub fn providers(&self) -> Vec<&str> {
        let mut out = PROVIDERS.to_vec();
        let custom = self.files.iter().filter(|p| p.is_custom());
        out.extend(custom.map(|p| p.id.as_str()));
        out
    }

    /// The picker group title of a provider id.
    pub fn group_title(&self, provider: &str) -> &str {
        match provider {
            ANTHROPIC => "Claude",
            other => (self.files.iter())
                .find(|p| p.id == other)
                .map_or("Claude", ProviderFile::title),
        }
    }

    /// `models` split by provider, in [`Catalog::providers`] order, keeping
    /// the order inside each group; empty groups are left out.
    pub fn grouped<'a>(&'a self, models: &'a [ModelInfo]) -> Vec<(&'a str, Vec<&'a ModelInfo>)> {
        (self.providers().into_iter())
            .map(|provider| {
                let group: Vec<&ModelInfo> = (models.iter())
                    .filter(|m| self.provider_of(&m.id) == provider)
                    .collect();
                (provider, group)
            })
            .filter(|(_, group)| !group.is_empty())
            .collect()
    }

    /// The entry of `model`: `<id>/<model id>` of any provider file, or a
    /// bare id in an `anthropic` file.
    pub fn model(&self, model: &str) -> Option<&ModelFile> {
        let (provider, id) = model.split_once('/').unwrap_or((ANTHROPIC, model));
        let file = self.files.iter().find(|p| p.id == provider)?;
        file.models.iter().find(|m| m.id == id)
    }

    /// The price of `model`, when its entry has one.
    pub fn price(&self, model: &str) -> Option<Price> {
        self.model(model).and_then(|entry| entry.price)
    }

    /// Effort words `model` takes, lowest first; empty when none.
    pub fn efforts(&self, model: &str) -> Vec<String> {
        if self.custom(model).is_some() {
            return self
                .model(model)
                .map(|m| m.efforts.clone())
                .unwrap_or_default();
        }
        let levels = anthropic::efforts(&self.live, model);
        levels.into_iter().map(String::from).collect()
    }

    /// The effort `model` takes for `effort` (see each provider's `fit_effort`).
    pub fn fit_effort(&self, model: &str, effort: Option<&str>) -> Option<String> {
        if self.custom(model).is_some() {
            let known = self.efforts(model);
            return effort
                .filter(|e| known.iter().any(|k| k == e))
                .map(String::from);
        }
        anthropic::fit_effort(&self.live, model, effort)
    }

    /// The context window of `model` in tokens from the provider files, the
    /// live table or the built-in table; 0 when unknown.
    pub fn context_window(&self, model: &str) -> u64 {
        if let Some(window) = self.model(model).and_then(|m| m.context_window) {
            return window;
        }
        anthropic::context_window(&self.live, model)
    }

    /// The display name of `model` from the provider files or the built-in
    /// table; `None` when unknown.
    pub fn display_name(&self, model: &str) -> Option<String> {
        if self.custom(model).is_some() {
            let entry = self.model(model)?;
            return Some(entry.name.clone().unwrap_or_else(|| entry.id.clone()));
        }
        anthropic::builtin_models()
            .into_iter()
            .find(|m| m.id == model)
            .map(|m| m.display_name)
    }

    /// The `maxOutput` of `model`'s entry in `providers/*.json`, when set:
    /// the request's `max_tokens` stays at or below it.
    pub fn max_output_override(&self, model: &str) -> Option<u64> {
        (self.model(model))
            .and_then(|m| m.max_output)
            .filter(|n| *n > 0)
    }
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
        let catalog = Catalog::default();
        assert_eq!(catalog.provider_of("claude-opus-5-5"), ANTHROPIC);
        assert_eq!(catalog.efforts("claude-opus-5-5").last().unwrap(), "max");
        assert_eq!(
            catalog
                .fit_effort("claude-opus-4-5", Some("max"))
                .as_deref(),
            Some("high")
        );
        assert_eq!(
            catalog.display_name("claude-opus-5-5").as_deref(),
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
