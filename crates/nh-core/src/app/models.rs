//! The model catalog for front ends: the last known list (`models.json`)
//! and a live `GET /v1/models` with the credential the runtime would use.

use std::path::{Path, PathBuf};
use std::sync::Arc;

use serde::{Deserialize, Serialize};

use crate::auth::AuthSource;
use crate::http::Transport;
use crate::llm::anthropic::{builtin_models, remember_models, Anthropic, ModelInfo};
use crate::trace;

/// File name of the model cache inside the nh home.
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
    std::fs::create_dir_all(home).map_err(|err| format!("{}: {err}", home.display()))?;
    let path = home.join(CACHE_FILE);
    let tmp = home.join(format!("{CACHE_FILE}.{}.tmp", std::process::id()));
    std::fs::write(&tmp, text).map_err(|err| format!("{}: {err}", tmp.display()))?;
    std::fs::rename(&tmp, &path).map_err(|err| {
        let _ = std::fs::remove_file(&tmp);
        format!("{}: {err}", path.display())
    })
}

/// Fills the live capability registry from the cache of `home`, if any.
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

impl ModelSource {
    /// `auth` is the credential source, or the user-facing reason there is none.
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

    /// The cached list, else the built-in table. Reads one small file.
    pub fn cached(&self) -> Vec<ModelInfo> {
        read_cache(&self.home).unwrap_or_else(builtin_models)
    }

    /// Blocking `GET /v1/models` (seconds on a slow network: call it off
    /// the UI thread). On success the list fills the capability registry
    /// and the cache. A failure never holds or switches the account.
    pub fn fetch(&self) -> Result<Vec<ModelInfo>, String> {
        let source = self.auth.clone()?;
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
