//! accounts.json version 3: providers, Google secrets, active per provider.

use fast_harness_core::accounts::model::{Account, AccountKind, Provider, Secret};
use fast_harness_core::accounts::store::{AccountStore, StoreError};
use std::path::PathBuf;
use std::sync::atomic::{AtomicU32, Ordering};

struct TempHome(PathBuf);

impl TempHome {
    fn new() -> TempHome {
        static COUNTER: AtomicU32 = AtomicU32::new(0);
        let n = COUNTER.fetch_add(1, Ordering::SeqCst);
        let dir = std::env::temp_dir().join(format!(
            "fast-harness-google-store-{}-{n}",
            std::process::id()
        ));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        TempHome(dir)
    }

    fn file(&self) -> PathBuf {
        self.0.join("accounts.json")
    }
}

impl Drop for TempHome {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

fn google(id: &str) -> Account {
    Account {
        id: Account::google_id(id),
        kind: AccountKind::Own,
        provider: Provider::Google,
        email: Some(format!("{id}@example.com")),
        name: None,
        plan: Some("Google AI Pro".into()),
        added_at: 5,
    }
}

fn claude(id: &str) -> Account {
    Account {
        id: id.into(),
        kind: AccountKind::Own,
        provider: Provider::Anthropic,
        email: None,
        name: None,
        plan: None,
        added_at: 1,
    }
}

fn google_secret() -> Secret {
    Secret::Google {
        refresh: "fake-g-refresh".into(),
        access: "fake-g-access".into(),
        expires_at: 42,
        project_id: Some("proj-1".into()),
    }
}

#[test]
fn version_2_records_load_as_anthropic_and_save_as_version_3() {
    let home = TempHome::new();
    let v2 = r#"{"version":2,"active":"u-a","accounts":[{"id":"u-a","kind":"own","addedAt":1},
        {"id":"k-00112233","kind":"apiKey","addedAt":2}]}"#;
    std::fs::write(home.file(), v2).unwrap();
    let store = AccountStore::load(home.file()).unwrap();
    assert!(store
        .list()
        .iter()
        .all(|a| a.provider == Provider::Anthropic));
    assert_eq!(store.active().unwrap().id, "u-a");
    assert!(store.active_for(Provider::Google).is_none());
    store.save().unwrap();
    let raw: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(home.file()).unwrap()).unwrap();
    assert_eq!(raw["version"], 3);
    assert_eq!(raw["accounts"][0]["provider"], "anthropic");
    assert!(raw.get("activeGoogle").is_none());
}

#[test]
fn google_record_round_trips_with_its_secret_encrypted() {
    let home = TempHome::new();
    let mut store = AccountStore::load(home.file()).unwrap();
    store.upsert(google("123"), Some(google_secret())).unwrap();
    store.save().unwrap();
    let text = std::fs::read_to_string(home.file()).unwrap();
    assert!(!text.contains("fake-g-refresh") && !text.contains("proj-1"));
    let raw: serde_json::Value = serde_json::from_str(&text).unwrap();
    assert_eq!(raw["accounts"][0]["provider"], "google");
    assert_eq!(raw["accounts"][0]["id"], "g-123");
    let loaded = AccountStore::load(home.file()).unwrap();
    assert_eq!(loaded.get("g-123"), Some(&google("123")));
    assert_eq!(loaded.secret("g-123").unwrap(), Some(google_secret()));
    let shown = format!("{:?}", google_secret());
    assert!(!shown.contains("fake-g-refresh") && !shown.contains("fake-g-access"));
}

#[test]
fn secrets_must_fit_the_provider() {
    let mut store = AccountStore::load(TempHome::new().file()).unwrap();
    let err = store
        .upsert(claude("u-a"), Some(google_secret()))
        .unwrap_err();
    assert!(matches!(err, StoreError::SecretKindMismatch(_)));
    let own = Secret::Own {
        refresh: "r".into(),
        access: "a".into(),
        expires_at: 1,
    };
    let err = store.upsert(google("9"), Some(own)).unwrap_err();
    assert!(matches!(err, StoreError::SecretKindMismatch(_)));
}

#[test]
fn active_is_kept_per_provider() {
    let home = TempHome::new();
    let mut store = AccountStore::load(home.file()).unwrap();
    store.upsert(claude("u-a"), None).unwrap();
    store.upsert(google("1"), None).unwrap();
    store.upsert(google("2"), None).unwrap();
    store.set_active("u-a").unwrap();
    store.set_active("g-2").unwrap();
    assert_eq!(store.active().unwrap().id, "u-a");
    assert_eq!(store.active_for(Provider::Anthropic).unwrap().id, "u-a");
    assert_eq!(store.active_for(Provider::Google).unwrap().id, "g-2");
    store.save().unwrap();
    let raw: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(home.file()).unwrap()).unwrap();
    assert_eq!(
        (raw["active"].as_str(), raw["activeGoogle"].as_str()),
        (Some("u-a"), Some("g-2"))
    );
    let mut loaded = AccountStore::load(home.file()).unwrap();
    assert_eq!(loaded.active_for(Provider::Google).unwrap().id, "g-2");
    loaded.remove("g-2");
    assert!(loaded.active_for(Provider::Google).is_none());
    assert_eq!(loaded.active().unwrap().id, "u-a");
}

#[test]
fn a_pointer_at_the_other_provider_is_not_active() {
    let home = TempHome::new();
    let text = r#"{"version":3,"active":"g-1","activeGoogle":"u-a","accounts":[
        {"id":"u-a","kind":"own","provider":"anthropic","addedAt":1},
        {"id":"g-1","kind":"own","provider":"google","addedAt":2}]}"#;
    std::fs::write(home.file(), text).unwrap();
    let store = AccountStore::load(home.file()).unwrap();
    assert!(store.active().is_none());
    assert!(store.active_for(Provider::Google).is_none());
}

#[test]
fn the_anthropic_pool_skips_google_accounts() {
    use fast_harness_core::accounts::pool::AccountPool;
    use fast_harness_core::auth::AuthSource;
    use fast_harness_core::http::ScriptedTransport;
    use fast_harness_core::llm::Auth;
    let home = TempHome::new();
    let mut store = AccountStore::load(home.file()).unwrap();
    store.upsert(google("7"), Some(google_secret())).unwrap();
    store.set_active("g-7").unwrap();
    store.save().unwrap();
    let pool = AccountPool::open(home.file(), home.0.join("usage.json")).unwrap();
    let http = ScriptedTransport::new(vec![]);
    assert!(pool.lease(&http).is_err());
    assert_eq!(pool.poll_usage(&http), 0);
    assert!(http.requests().is_empty());
    let status = pool.status();
    assert!(status[0].active && status[0].token_ok);
    assert_eq!(pool.google_project("g-7").as_deref(), Some("proj-1"));
    pool.add_api_key("test-key-1", "Key").unwrap();
    let lease = pool.lease(&http).unwrap();
    assert_eq!(lease.auth, Auth::ApiKey("test-key-1".into()));
    assert!(http.requests().is_empty());
}
