//! The model catalog for front ends: the last known list (`models.json`)
//! and a live list from Claude when it has a credential (`GET /v1/models`),
//! plus the models of `providers/*.json`.

use std::path::{Path, PathBuf};
use std::sync::Arc;

use serde::{Deserialize, Serialize};

use crate::auth::AuthSource;
use crate::http::Transport;
use crate::llm::anthropic::{builtin_models, remember_models, Anthropic, ModelInfo};
use crate::trace;

/// File name of the model cache inside the fast-harness home.
pub const CACHE_FILE: &str = "models.json";
/// Format version of the model cache.
pub const CACHE_VERSION: u32 = 1;

#[derive(Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct CacheFile {
    version: u32,
    /// Unix milliseconds of the fetch.
    fetched_at: u64,
    models: Vec<ModelInfo>,
}

/// The cached list of `home`; `None` when the file is missing, corrupt, of
/// another version or empty.
pub fn read_cache(home: &Path) -> Option<Vec<ModelInfo>> {
    let bytes = std::fs::read(home.join(CACHE_FILE)).ok()?;
    let file: CacheFile = serde_json::from_slice(&bytes).ok()?;
    (file.version == CACHE_VERSION && !file.models.is_empty()).then_some(file.models)
}

/// Writes `models` to the cache of `home` (temp file, then rename).
pub fn write_cache(home: &Path, models: &[ModelInfo]) -> Result<(), String> {
    let file = CacheFile {
        version: CACHE_VERSION,
        fetched_at: trace::now_ms(),
        models: models.to_vec(),
    };
    let text = serde_json::to_string_pretty(&file).map_err(|err| err.to_string())?;
    write_atomic(home, CACHE_FILE, &text)
}

/// Writes `text` to `home/name` through `store::write_atomic` (a cache: the
/// last writer wins, no lock).
pub(super) fn write_atomic(home: &Path, name: &str, text: &str) -> Result<(), String> {
    let path = home.join(name);
    crate::store::write_atomic(&path, text.as_bytes())
        .map_err(|err| format!("{}: {err}", path.display()))
}

/// Fills the live capability registries from the caches of `home`, if any.
pub fn load_cache(home: &Path) {
    if let Some(models) = read_cache(home) {
        remember_models(&models);
    }
}

/// The model list for pickers, usable from any thread. Take a new one from
/// `Runtime::models` each time a picker opens: it holds the credential
/// choice of that moment.
#[derive(Clone)]
pub struct ModelSource {
    home: PathBuf,
    auth: Result<Arc<dyn AuthSource>, String>,
    transport: Arc<dyn Transport>,
    provider: Arc<Anthropic>,
}

/// One provider's live list, or why it failed.
type Listed = Result<Vec<ModelInfo>, String>;

impl ModelSource {
    /// `auth` is Claude's credential source, or the user-facing reason there is none.
    pub fn new(
        home: PathBuf,
        auth: Result<Arc<dyn AuthSource>, String>,
        transport: Arc<dyn Transport>,
        provider: Arc<Anthropic>,
    ) -> ModelSource {
        ModelSource {
            home,
            auth,
            transport,
            provider,
        }
    }

    /// Claude's cached list (else the built-in table), then the custom
    /// providers' models. Reads one small file.
    pub fn cached(&self) -> Vec<ModelInfo> {
        let mut list = self.cached_claude();
        list.extend(custom_models());
        list
    }

    fn cached_claude(&self) -> Vec<ModelInfo> {
        read_cache(&self.home).unwrap_or_else(builtin_models)
    }

    /// Blocking live list of Claude (seconds on a slow network: call it off
    /// the UI thread) when it has a credential, then the custom providers'
    /// models. Fails when there is no credential or the list failed. A
    /// successful list fills the registry and cache. A failure never holds
    /// or switches an account.
    pub fn fetch(&self) -> Result<Vec<ModelInfo>, String> {
        let source = self.auth.as_ref().map_err(Clone::clone)?;
        let mut list = self.list_claude(source.as_ref())?;
        list.extend(custom_models());
        Ok(list)
    }
    /// `GET /v1/models`; fills the registry and `models.json`.
    fn list_claude(&self, source: &dyn AuthSource) -> Listed {
        let transport = self.transport.as_ref();
        let lease = source.lease(transport).map_err(|err| err.message)?;
        let list = self
            .provider
            .list_models(transport, &lease.auth)
            .map_err(|err| format!("model list: {err}"))?;
        if list.is_empty() {
            return Err("model list: the API returned no models".into());
        }
        // A cache write failure only costs the next start its list.
        let _ = write_cache(&self.home, &list);
        Ok(list)
    }
}

/// The models of the custom providers of `providers/*.json` (PLAN.md
/// D41), named `<provider id>/<model id>`, in file order.
pub fn custom_models() -> Vec<ModelInfo> {
    let providers = crate::llm::custom::providers();
    let custom = providers.iter().filter(|p| p.is_custom());
    let pairs = custom.flat_map(|p| p.models.iter().map(move |m| (p, m)));
    pairs
        .map(|(provider, model)| ModelInfo {
            id: format!("{}/{}", provider.id, model.id),
            display_name: model.name.clone().unwrap_or_else(|| model.id.clone()),
            context_window: model.context_window.unwrap_or(0),
            max_output: model.max_output.unwrap_or(0),
            reasoning: !model.efforts.is_empty(),
            efforts: model.efforts.clone(),
            deprecated: false,
            live: None,
        })
        .collect()
}
