//! The model catalog front ends use: live fetch, cache file and fallback.

use std::path::PathBuf;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;

use fast_harness_core::app::{read_cache, ModelSource, CACHE_FILE};
use fast_harness_core::auth::{AuthError, AuthSource, Failover, Lease};
use fast_harness_core::http::{HttpHead, ScriptedResponse, ScriptedTransport, Transport};
use fast_harness_core::llm::anthropic::{builtin_models, efforts, Anthropic};
use fast_harness_core::llm::{Auth, LlmError};
use serde_json::{json, Value};

struct TempDir(PathBuf);

impl TempDir {
    fn new(tag: &str) -> TempDir {
        let dir =
            std::env::temp_dir().join(format!("fast-harness-models-{tag}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        TempDir(dir)
    }
}

impl Drop for TempDir {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

/// A fixed credential that counts failure reports.
#[derive(Default)]
struct Counting {
    failures: AtomicUsize,
}

impl AuthSource for Counting {
    fn lease(&self, _http: &dyn Transport) -> Result<Lease, AuthError> {
        Ok(Lease {
            account: "a".into(),
            label: "a".into(),
            auth: Auth::OAuth("t".into()),
        })
    }

    fn observe(&self, _lease: &Lease, _head: &HttpHead) {}

    fn on_failure(&self, _: &Lease, _: &LlmError, _: &str, _: &dyn Transport) -> Failover {
        self.failures.fetch_add(1, Ordering::SeqCst);
        Failover::Retry
    }
}

fn source(home: &TempDir, auth: Arc<Counting>, replies: Vec<ScriptedResponse>) -> ModelSource {
    let auth: Arc<dyn AuthSource> = auth;
    let transport: Arc<dyn Transport> = Arc::new(ScriptedTransport::new(replies));
    ModelSource::new(
        home.0.clone(),
        Ok(auth),
        transport,
        Arc::new(Anthropic::default()),
    )
}

fn page() -> String {
    json!({"data": [
        {"id": "claude-source-1", "display_name": "Source One", "lifecycle": "active",
         "max_input_tokens": 500000, "max_tokens": 64000, "capabilities": {
            "effort": {"supported": true, "low": {"supported": true}, "medium": {"supported": true},
                "high": {"supported": true}, "xhigh": {"supported": true}, "max": {"supported": false}},
            "thinking": {"supported": true, "types": {"adaptive": {"supported": true},
                "disabled": {"supported": false}}}}},
        {"id": "claude-source-old", "lifecycle": "retired"},
    ], "has_more": false, "last_id": "claude-source-old"})
    .to_string()
}

#[test]
fn fetch_fills_registry_and_cache() {
    let home = TempDir::new("fetch");
    let auth = Arc::new(Counting::default());
    let models = source(&home, auth, vec![ScriptedResponse::new(200, &page(), 3)]);
    assert_eq!(models.cached(), builtin_models());
    let list = models.fetch().unwrap();
    assert_eq!(list.len(), 1);
    assert_eq!(list[0].efforts, ["low", "medium", "high", "xhigh"]);
    assert_eq!(
        efforts("claude-source-1"),
        ["low", "medium", "high", "xhigh"]
    );
    let text = std::fs::read_to_string(home.0.join(CACHE_FILE)).unwrap();
    let file: Value = serde_json::from_str(&text).unwrap();
    assert_eq!(file["version"], 1);
    assert!(file["fetchedAt"].as_u64().unwrap() > 0);
    assert_eq!(file["models"][0]["displayName"], "Source One");
    assert_eq!(file["models"][0]["live"]["adaptive"], true);
    assert_eq!(models.cached(), list);
    assert_eq!(read_cache(&home.0), Some(list));
    let leftovers = std::fs::read_dir(&home.0).unwrap().count();
    assert_eq!(leftovers, 1, "only models.json, no temp file");
}

#[test]
fn failed_fetch_keeps_the_account_and_the_cache() {
    let home = TempDir::new("denied");
    let auth = Arc::new(Counting::default());
    let body = r#"{"type":"error","error":{"type":"authentication_error","message":"no"}}"#;
    let denied = ScriptedResponse::new(401, body, 4);
    let models = source(&home, auth.clone(), vec![denied]);
    let err = models.fetch().unwrap_err();
    assert!(
        err.starts_with("model list:") && err.contains("401"),
        "{err}"
    );
    assert_eq!(auth.failures.load(Ordering::SeqCst), 0);
    assert!(!home.0.join(CACHE_FILE).exists());
    assert_eq!(models.cached(), builtin_models());
    let none = ModelSource::new(
        home.0.clone(),
        Err("no account".into()),
        Arc::new(ScriptedTransport::new(Vec::new())),
        Arc::new(Anthropic::default()),
    );
    assert_eq!(none.fetch().unwrap_err(), "no account");
}

#[test]
fn corrupt_or_other_version_cache_is_ignored() {
    let home = TempDir::new("corrupt");
    let path = home.0.join(CACHE_FILE);
    std::fs::write(&path, "{not json").unwrap();
    assert_eq!(read_cache(&home.0), None);
    let other = json!({"version": 2, "fetchedAt": 1, "models": [
        {"id": "x", "displayName": "X", "contextWindow": 1, "maxOutput": 1, "reasoning": false}]});
    std::fs::write(&path, other.to_string()).unwrap();
    assert_eq!(read_cache(&home.0), None);
    let mut v1 = other;
    v1["version"] = json!(1);
    std::fs::write(&path, v1.to_string()).unwrap();
    let list = read_cache(&home.0).unwrap();
    assert_eq!((list[0].id.as_str(), list[0].live), ("x", None));
}

#[test]
fn antigravity_cache_on_disk_is_ignored_and_kept() {
    // Branch `antigravity` (PLAN.md D53) wrote antigravity-models.json: this
    // build never lists its models and never deletes the file.
    let home = TempDir::new("ag-file");
    let file = home.0.join("antigravity-models.json");
    std::fs::write(&file, r#"{"version":1,"models":[{"id":"gemini-x"}]}"#).unwrap();
    let auth = Arc::new(Counting::default());
    let models = source(&home, auth, vec![ScriptedResponse::new(200, &page(), 5)]);
    let list = models.fetch().unwrap();
    assert!(list.iter().all(|m| !m.id.starts_with("antigravity/")));
    assert_eq!(models.cached(), list);
    assert!(file.exists());
}
