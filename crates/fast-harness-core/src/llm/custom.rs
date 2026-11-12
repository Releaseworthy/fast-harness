//! `providers/*.json` (PLAN.md D41): providers and models defined by the
//! user (and a trusted project) without rebuilding. A file names a provider
//! `id` with an `api` (`anthropic-messages` or `openai-chat`), its `baseUrl`,
//! the key (`keyEnv`: an environment variable name, or `account`), extra
//! `headers` and `models`; its models are named `<id>/<model id>`. A file
//! whose id is a built-in provider (`anthropic`; `antigravity` stays reserved for the branch of D53) only
//! overrides that provider's model numbers (window, maxOutput, price,
//! compaction). The process-wide registry ([`remember`]) is read by
//! `catalog`, the pickers, the adapters and `stats`.

use std::collections::{BTreeMap, HashMap};
use std::sync::{OnceLock, RwLock};

use serde::Deserialize;

use crate::config::compaction::CompactionPatch;
use crate::config::layers::{LayerFile, Skipped};
use crate::types::Usage;

pub const API_ANTHROPIC: &str = "anthropic-messages";
pub const API_OPENAI: &str = "openai-chat";
/// Built-in provider ids whose files only override model numbers.
pub const BUILTIN: [&str; 2] = ["anthropic", "antigravity"];

#[derive(Clone, Debug, Default, PartialEq, Deserialize)]
#[serde(default, rename_all = "camelCase")]
#[non_exhaustive]
pub struct ProviderFile {
    pub id: String,
    /// The picker group title; the id when absent.
    pub name: Option<String>,
    pub api: Option<String>,
    pub base_url: String,
    /// The environment variable holding the API key.
    pub key_env: Option<String>,
    /// `claude` (the Claude accounts) instead of a key.
    pub account: Option<String>,
    pub headers: BTreeMap<String, String>,
    pub models: Vec<ModelFile>,
}

#[derive(Clone, Debug, Default, PartialEq, Deserialize)]
#[serde(default, rename_all = "camelCase")]
#[non_exhaustive]
pub struct ModelFile {
    pub id: String,
    pub name: Option<String>,
    pub context_window: Option<u64>,
    pub max_output: Option<u64>,
    pub efforts: Vec<String>,
    pub price: Option<Price>,
    pub compaction: CompactionPatch,
    /// `anthropic`, `implicit` or `none` (informational for now).
    pub cache: Option<String>,
}

/// US dollars per million tokens. `cacheRead` defaults to `input`,
/// `cacheWrite` (the 5 minute write) to `input`, `cacheWrite1h` to twice
/// `input`.
#[derive(Clone, Copy, Debug, Default, PartialEq, Deserialize)]
#[serde(default, rename_all = "camelCase")]
#[non_exhaustive]
pub struct Price {
    pub input: f64,
    pub output: f64,
    pub cache_read: Option<f64>,
    pub cache_write: Option<f64>,
    pub cache_write_1h: Option<f64>,
}

impl Price {
    /// Input and output prices; the cache prices take their defaults.
    pub fn new(input: f64, output: f64) -> Price {
        Price {
            input,
            output,
            ..Price::default()
        }
    }

    /// The cost of `usage` in US dollars.
    pub fn cost(&self, usage: &Usage) -> f64 {
        let read = self.cache_read.unwrap_or(self.input);
        let write_5m = self.cache_write.unwrap_or(self.input);
        let write_1h = self.cache_write_1h.unwrap_or(2.0 * self.input);
        let short = usage.cache_write.saturating_sub(usage.cache_write_1h);
        let tokens = usage.input as f64 * self.input
            + usage.output as f64 * self.output
            + usage.cache_read as f64 * read
            + short as f64 * write_5m
            + usage.cache_write_1h as f64 * write_1h;
        tokens / 1_000_000.0
    }
}

impl ModelFile {
    /// A model entry with nothing but its id; set the other public fields
    /// on the value (window, max output, efforts, price).
    pub fn new(id: impl Into<String>) -> ModelFile {
        ModelFile {
            id: id.into(),
            ..ModelFile::default()
        }
    }
}

impl ProviderFile {
    /// A provider file holding `models`; `Ext::add_provider` sets its id.
    /// The credential (`key_env`, `account`) is set on the value.
    pub fn new(models: Vec<ModelFile>) -> ProviderFile {
        ProviderFile {
            models,
            ..ProviderFile::default()
        }
    }

    /// True for a provider of its own (not an override of a built-in one).
    pub fn is_custom(&self) -> bool {
        !BUILTIN.contains(&self.id.as_str())
    }

    pub fn title(&self) -> &str {
        self.name.as_deref().unwrap_or(&self.id)
    }
}

/// Why `file` cannot load; `None` when it is usable.
fn problem(file: &ProviderFile) -> Option<String> {
    if file.id.trim().is_empty() || file.id.contains('/') {
        return Some("`id` is missing or holds a `/`".into());
    }
    if !file.is_custom() {
        return None;
    }
    match file.api.as_deref() {
        Some(API_ANTHROPIC | API_OPENAI) => {}
        Some(other) => return Some(format!("unknown api `{other}`")),
        None => {
            return Some(format!(
                "`api` is missing ({API_ANTHROPIC} or {API_OPENAI})"
            ))
        }
    }
    file.base_url
        .trim()
        .is_empty()
        .then(|| "`baseUrl` is missing".into())
}

/// Parses `files` in order; a later file with the same id replaces the
/// earlier one. Returns the providers, the files that loaded and the
/// skipped ones with their reasons.
pub fn read(files: &[LayerFile]) -> (Vec<ProviderFile>, Vec<LayerFile>, Vec<Skipped>) {
    let (mut out, mut loaded, mut skipped) = (Vec::<ProviderFile>::new(), Vec::new(), Vec::new());
    for file in files {
        let parsed = std::fs::read(&file.path)
            .map_err(|err| err.to_string())
            .and_then(|bytes| {
                serde_json::from_slice::<ProviderFile>(&bytes).map_err(|err| err.to_string())
            })
            .and_then(|provider| match problem(&provider) {
                Some(reason) => Err(reason),
                None => Ok(provider),
            });
        match parsed {
            Ok(provider) => {
                out.retain(|known| known.id != provider.id);
                out.push(provider);
                loaded.push(file.clone());
            }
            Err(reason) => skipped.push(Skipped {
                path: file.path.clone(),
                reason,
            }),
        }
    }
    (out, loaded, skipped)
}

#[derive(Default)]
struct Registry {
    providers: Vec<ProviderFile>,
    /// Ids and titles kept for the `&'static str` the catalog hands out.
    names: HashMap<String, &'static str>,
}

fn registry() -> &'static RwLock<Registry> {
    static REGISTRY: OnceLock<RwLock<Registry>> = OnceLock::new();
    REGISTRY.get_or_init(RwLock::default)
}

/// A process-lifetime copy of `text` (each distinct id or title once).
fn intern(names: &mut HashMap<String, &'static str>, text: &str) -> &'static str {
    if let Some(known) = names.get(text) {
        return known;
    }
    let leaked: &'static str = Box::leak(text.to_string().into_boxed_str());
    names.insert(text.to_string(), leaked);
    leaked
}

/// Adds `providers` to the registry, replacing those with the same id.
pub fn remember(providers: &[ProviderFile]) {
    let mut reg = registry().write().unwrap_or_else(|p| p.into_inner());
    for provider in providers {
        reg.providers.retain(|known| known.id != provider.id);
        reg.providers.push(provider.clone());
        let (id, title) = (provider.id.clone(), provider.title().to_string());
        intern(&mut reg.names, &id);
        intern(&mut reg.names, &title);
    }
}

/// Every remembered provider, in the order they were added.
pub fn providers() -> Vec<ProviderFile> {
    let reg = registry().read().unwrap_or_else(|p| p.into_inner());
    reg.providers.clone()
}

/// The custom provider ids, in registry order.
pub fn custom_ids() -> Vec<&'static str> {
    let reg = registry().read().unwrap_or_else(|p| p.into_inner());
    let custom = reg.providers.iter().filter(|p| p.is_custom());
    custom
        .filter_map(|p| reg.names.get(&p.id).copied())
        .collect()
}

/// The custom provider serving `model` (`<id>/<model id>`).
pub fn provider_of(model: &str) -> Option<(&'static str, ProviderFile)> {
    let (prefix, _) = model.split_once('/')?;
    let reg = registry().read().unwrap_or_else(|p| p.into_inner());
    let file = reg
        .providers
        .iter()
        .find(|p| p.is_custom() && p.id == prefix)?;
    Some((reg.names.get(prefix).copied()?, file.clone()))
}

/// The group title of provider `id`, when it is remembered.
pub fn title_of(id: &str) -> Option<&'static str> {
    let reg = registry().read().unwrap_or_else(|p| p.into_inner());
    let file = reg.providers.iter().find(|p| p.id == id)?;
    reg.names.get(file.title()).copied()
}

/// The model entry of `model`: `<id>/<model id>` of any remembered
/// provider, or a bare id in an `anthropic` file.
pub fn model(model: &str) -> Option<ModelFile> {
    let (provider, id) = match model.split_once('/') {
        Some((provider, id)) => (provider, id),
        None => ("anthropic", model),
    };
    let reg = registry().read().unwrap_or_else(|p| p.into_inner());
    let file = reg.providers.iter().find(|p| p.id == provider)?;
    file.models.iter().find(|m| m.id == id).cloned()
}

/// The price of `model`, when its entry has one.
pub fn price(model_id: &str) -> Option<Price> {
    model(model_id).and_then(|entry| entry.price)
}
