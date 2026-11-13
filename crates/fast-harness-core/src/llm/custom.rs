//! `providers/*.json` (PLAN.md D41): providers and models defined by the
//! user (and a trusted project) without rebuilding. A file names a provider
//! `id` with an `api` (`anthropic-messages` or `openai-chat`), its `baseUrl`,
//! the key (`keyEnv`: an environment variable name, or `account`), extra
//! `headers` and `models`; its models are named `<id>/<model id>`. A file
//! whose id is a built-in provider (`anthropic`; `antigravity` stays reserved for the branch of D53) only
//! overrides that provider's model numbers (window, maxOutput, price,
//! compaction). The loaded files live in the runtime's `catalog::Catalog`
//! (PLAN.md D67), read by the pickers, the adapters and `stats`.

use std::collections::BTreeMap;

use serde::Deserialize;

use crate::config::compaction::CompactionPatch;
use crate::config::ext_file;
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
                let value: serde_json::Value =
                    serde_json::from_slice(&bytes).map_err(|err| err.to_string())?;
                ext_file::on(ext_file::json_header(&value)?)?;
                serde_json::from_value::<ProviderFile>(value).map_err(|err| err.to_string())
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
