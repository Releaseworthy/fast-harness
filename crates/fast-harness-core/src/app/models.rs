//! The model catalog for front ends: the last known lists (`models.json`,
//! `antigravity-models.json`) and a live list from each provider that has a
//! credential (Claude's `GET /v1/models`, Antigravity's `fetchAvailableModels`).

use std::path::{Path, PathBuf};
use std::sync::Arc;

use serde::{Deserialize, Serialize};

use super::ag_models::{cached_ag_models, load_ag_cache, write_ag_cache};
use crate::auth::AuthSource;
use crate::http::Transport;
use crate::llm::anthropic::{builtin_models, remember_models, Anthropic, ModelInfo};
use crate::llm::antigravity::hub::Hub;
use crate::llm::antigravity::models::{self as ag, CatalogEntry};
use crate::llm::catalog::{group_title, ANTHROPIC, ANTIGRAVITY};
use crate::llm::Auth;
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

/// Writes `text` to `home/name` through a temp file and a rename.
pub(super) fn write_atomic(home: &Path, name: &str, text: &str) -> Result<(), String> {
    std::fs::create_dir_all(home).map_err(|err| format!("{}: {err}", home.display()))?;
    let path = home.join(name);
    let tmp = home.join(format!("{name}.{}.tmp", std::process::id()));
    std::fs::write(&tmp, text).map_err(|err| format!("{}: {err}", tmp.display()))?;
    std::fs::rename(&tmp, &path).map_err(|err| {
        let _ = std::fs::remove_file(&tmp);
        format!("{}: {err}", path.display())
    })
}

/// Fills the live capability registries from the caches of `home`, if any.
pub fn load_cache(home: &Path) {
    if let Some(models) = read_cache(home) {
        remember_models(&models);
    }
    load_ag_cache(home);
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
    /// The Google accounts and the Hub, when there is a Google account.
    google: Option<(Arc<dyn AuthSource>, Arc<Hub>)>,
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
            google: None,
        }
    }

    /// Also lists Antigravity's models with the Google accounts of `google`.
    pub fn with_google(mut self, google: Arc<dyn AuthSource>, hub: Arc<Hub>) -> ModelSource {
        self.google = Some((google, hub));
        self
    }

    /// Claude's cached list (else the built-in table), then the cached
    /// Antigravity list. Reads two small files.
    pub fn cached(&self) -> Vec<ModelInfo> {
        let mut list = self.cached_of(ANTHROPIC);
        list.extend(self.cached_of(ANTIGRAVITY));
        list.extend(custom_models());
        list
    }

    fn cached_of(&self, provider: &str) -> Vec<ModelInfo> {
        match provider {
            ANTIGRAVITY => cached_ag_models(&self.home),
            _ => read_cache(&self.home).unwrap_or_else(builtin_models),
        }
    }

    /// Blocking live lists (seconds on a slow network: call it off the UI
    /// thread) of each provider with a credential, merged in picker order; a
    /// provider without a credential or whose list failed keeps its cached
    /// part. Fails only when every listed provider failed. A successful list
    /// fills that provider's registry and cache. A failure never holds or
    /// switches an account.
    pub fn fetch(&self) -> Result<Vec<ModelInfo>, String> {
        let claude = self
            .auth
            .as_ref()
            .ok()
            .map(|s| self.list_claude(s.as_ref()));
        let google = self
            .google
            .as_ref()
            .map(|(s, hub)| self.list_ag(s.as_ref(), hub));
        let tried: Vec<(&str, &Listed)> = [(ANTHROPIC, &claude), (ANTIGRAVITY, &google)]
            .into_iter()
            .filter_map(|(provider, listed)| listed.as_ref().map(|l| (provider, l)))
            .collect();
        if tried.is_empty() {
            return Err(self.auth.as_ref().err().cloned().unwrap_or_default());
        }
        let failures: Vec<String> = tried
            .iter()
            .filter_map(|(provider, l)| l.as_ref().err().map(|e| (*provider, e)))
            .map(|(provider, err)| format!("{}: {err}", group_title(provider)))
            .collect();
        if failures.len() == tried.len() {
            return Err(match tried.as_slice() {
                [(_, Err(only))] => only.clone(),
                _ => failures.join("; "),
            });
        }
        let mut list = self.live_or_cached(ANTHROPIC, claude);
        list.extend(self.live_or_cached(ANTIGRAVITY, google));
        list.extend(custom_models());
        Ok(list)
    }

    fn live_or_cached(&self, provider: &str, listed: Option<Listed>) -> Vec<ModelInfo> {
        match listed {
            Some(Ok(list)) => list,
            _ => self.cached_of(provider),
        }
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

    /// `fetchAvailableModels` with a Google lease; fills the registry and
    /// `antigravity-models.json`.
    fn list_ag(&self, source: &dyn AuthSource, hub: &Hub) -> Listed {
        let transport = self.transport.as_ref();
        let lease = source.lease(transport).map_err(|err| err.message)?;
        let Auth::Google { access, project } = &lease.auth else {
            return Err("model list: the Google account gave no Google credential".into());
        };
        let list = ag::list(transport, hub, access, project)
            .map_err(|err| format!("model list: {err}"))?;
        if list.is_empty() {
            return Err("model list: Antigravity returned no models".into());
        }
        let entries: Vec<CatalogEntry> = list.iter().filter_map(|m| ag::lookup(&m.id)).collect();
        let _ = write_ag_cache(&self.home, &entries);
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
