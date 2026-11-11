//! The Antigravity model cache `antigravity-models.json` (PLAN.md "Files on
//! disk"): the catalog entries of the last successful `fetchAvailableModels`.

use std::path::Path;

use serde::{Deserialize, Serialize};

use super::models::write_atomic;
use crate::llm::anthropic::ModelInfo;
use crate::llm::antigravity::models::{remember, to_info, CatalogEntry};
use crate::trace;

/// File name of the Antigravity model cache inside the fast-harness home.
pub const AG_CACHE_FILE: &str = "antigravity-models.json";
/// Format version of the Antigravity model cache.
pub const AG_CACHE_VERSION: u32 = 1;

#[derive(Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct AgCacheFile {
    version: u32,
    /// Unix milliseconds of the fetch.
    fetched_at: u64,
    models: Vec<CatalogEntry>,
}

/// The cached Antigravity entries of `home`; `None` when the file is
/// missing, corrupt, of another version or empty.
pub fn read_ag_cache(home: &Path) -> Option<Vec<CatalogEntry>> {
    let bytes = std::fs::read(home.join(AG_CACHE_FILE)).ok()?;
    let file: AgCacheFile = serde_json::from_slice(&bytes).ok()?;
    (file.version == AG_CACHE_VERSION && !file.models.is_empty()).then_some(file.models)
}

/// Writes `entries` to the Antigravity cache of `home` (temp file, then rename).
pub fn write_ag_cache(home: &Path, entries: &[CatalogEntry]) -> Result<(), String> {
    let file = AgCacheFile {
        version: AG_CACHE_VERSION,
        fetched_at: trace::now_ms(),
        models: entries.to_vec(),
    };
    let text = serde_json::to_string_pretty(&file).map_err(|err| err.to_string())?;
    write_atomic(home, AG_CACHE_FILE, &text)
}

/// The cached Antigravity models as picker entries (`antigravity/<id>`).
pub fn cached_ag_models(home: &Path) -> Vec<ModelInfo> {
    read_ag_cache(home)
        .unwrap_or_default()
        .iter()
        .map(to_info)
        .collect()
}

/// Remembers the cached entries for requests (`models::lookup`).
pub fn load_ag_cache(home: &Path) {
    if let Some(entries) = read_ag_cache(home) {
        remember(&entries);
    }
}
