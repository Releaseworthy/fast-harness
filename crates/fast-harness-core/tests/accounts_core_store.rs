use fast_harness_core::accounts::model::{Account, AccountKind, Secret};
use fast_harness_core::accounts::store::{AccountStore, StoreError};
use fast_harness_core::store::WAIT;
use std::path::PathBuf;
use std::sync::atomic::{AtomicU32, Ordering};
use std::time::{SystemTime, UNIX_EPOCH};

/// A fresh temporary NH folder, removed on drop.
struct TempHome(PathBuf);

impl TempHome {
    fn new() -> TempHome {
        static COUNTER: AtomicU32 = AtomicU32::new(0);
        let nanos = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .subsec_nanos();
        let n = COUNTER.fetch_add(1, Ordering::SeqCst);
        let dir = std::env::temp_dir().join(format!(
            "fast-harness-accounts-test-{}-{nanos}-{n}",
            std::process::id()
        ));
        std::fs::create_dir_all(&dir).unwrap();
        TempHome(dir)
    }

    fn file(&self) -> PathBuf {
        self.0.join("nested").join("accounts.json")
    }
}

impl Drop for TempHome {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

fn account(id: &str, kind: AccountKind) -> Account {
    Account {
        id: id.into(),
        kind,
        provider: Default::default(),
        email: Some(format!("{id}@example.test")),
        name: None,
        plan: Some("pro".into()),
        added_at: 1_000,
    }
}

#[test]
fn missing_file_is_empty() {
    let home = TempHome::new();
    let store = AccountStore::load(home.file()).unwrap();
    assert!(store.list().is_empty());
    assert!(store.active().is_none());
    assert_eq!(store.path(), home.file());
}

#[test]
fn accounts_without_secrets_round_trip() {
    let home = TempHome::new();
    let mut store = AccountStore::load(home.file()).unwrap();
    store
        .update(WAIT, |store| {
            store.upsert(account("k-0011aabb", AccountKind::ApiKey), None)?;
            store.upsert(account("u-own-1", AccountKind::Own), None)?;
            store.set_active("u-own-1")
        })
        .unwrap();
    let names: Vec<_> = std::fs::read_dir(home.file().parent().unwrap())
        .unwrap()
        .map(|e| e.unwrap().file_name())
        .collect();
    assert_eq!(names, ["accounts.json"], "no temp file left behind");

    let loaded = AccountStore::load(home.file()).unwrap();
    assert_eq!(loaded.list().len(), 2);
    assert_eq!(loaded.active().unwrap().id, "u-own-1");
    assert_eq!(loaded.get("k-0011aabb").unwrap().kind, AccountKind::ApiKey);
    assert!(loaded.secret("k-0011aabb").unwrap().is_none());
    let raw: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(home.file()).unwrap()).unwrap();
    assert_eq!(raw["version"], 3);
    assert_eq!(raw["active"], "u-own-1");
    assert_eq!(
        AccountStore::load(home.file()).unwrap().list(),
        loaded.list()
    );
    assert!(raw["accounts"][0].get("secret").is_none());
}

#[test]
fn upsert_replaces_and_remove_clears_active() {
    let home = TempHome::new();
    let mut store = AccountStore::load(home.file()).unwrap();
    store
        .upsert(account("u-a", AccountKind::Own), None)
        .unwrap();
    let mut changed = account("u-a", AccountKind::Own);
    changed.plan = Some("max".into());
    store.upsert(changed, None).unwrap();
    assert_eq!(store.list().len(), 1);
    assert_eq!(store.get("u-a").unwrap().plan.as_deref(), Some("max"));
    store.set_active("u-a").unwrap();
    assert_eq!(store.remove("u-a").unwrap().id, "u-a");
    assert!(store.active().is_none());
    assert!(store.remove("u-a").is_none());
}

#[test]
fn unknown_ids_and_wrong_secrets_are_errors() {
    let home = TempHome::new();
    let mut store = AccountStore::load(home.file()).unwrap();
    let key = Secret::ApiKey {
        key: "test-key-1".into(),
    };
    assert!(matches!(
        store.set_active("u-x"),
        Err(StoreError::UnknownAccount(_))
    ));
    assert!(matches!(
        store.secret("u-x"),
        Err(StoreError::UnknownAccount(_))
    ));
    assert!(matches!(
        store.set_secret("u-x", key.clone()),
        Err(StoreError::UnknownAccount(_))
    ));
    let own = account("u-l", AccountKind::Own);
    assert!(matches!(
        store.upsert(own, Some(key.clone())),
        Err(StoreError::SecretKindMismatch(_))
    ));
    assert!(store.list().is_empty());
    store
        .upsert(account("u-o", AccountKind::Own), None)
        .unwrap();
    assert!(matches!(
        store.set_secret("u-o", key),
        Err(StoreError::SecretKindMismatch(_))
    ));
}

#[test]
fn bad_files_are_errors() {
    let home = TempHome::new();
    std::fs::create_dir_all(home.file().parent().unwrap()).unwrap();
    std::fs::write(home.file(), r#"{"version":4,"accounts":[]}"#).unwrap();
    assert!(matches!(
        AccountStore::load(home.file()),
        Err(StoreError::UnsupportedVersion(4))
    ));
    std::fs::write(home.file(), "{not json").unwrap();
    assert!(matches!(
        AccountStore::load(home.file()),
        Err(StoreError::Json(_))
    ));
}

/// A version 1 file: an own, an API-key and two linked records (no secrets).
fn v1_file(active: &str) -> String {
    let rec = |id: &str, kind: &str| {
        format!(r#"{{"id":"{id}","kind":"{kind}","email":"{id}@example.test","addedAt":1}}"#)
    };
    format!(
        r#"{{"version":1,"active":"{active}","accounts":[{},{},{},{}]}}"#,
        rec("u-link-1", "linked"),
        rec("u-own-1", "own"),
        rec("k-0011aabb", "apiKey"),
        rec("u-link-2", "linked")
    )
}

#[test]
fn version_1_drops_linked_records_and_saves_as_version_3() {
    let home = TempHome::new();
    std::fs::create_dir_all(home.file().parent().unwrap()).unwrap();
    std::fs::write(home.file(), v1_file("k-0011aabb")).unwrap();
    let mut store = AccountStore::load(home.file()).unwrap();
    let ids: Vec<&str> = store.list().iter().map(|a| a.id.as_str()).collect();
    assert_eq!(ids, ["u-own-1", "k-0011aabb"]);
    assert_eq!(store.active().unwrap().id, "k-0011aabb");
    store.update(WAIT, |_| Ok(())).unwrap();
    let text = std::fs::read_to_string(home.file()).unwrap();
    let raw: serde_json::Value = serde_json::from_str(&text).unwrap();
    assert_eq!(raw["version"], 3);
    assert_eq!(raw["accounts"].as_array().unwrap().len(), 2);
    assert!(!text.contains("linked"));
    let again = AccountStore::load(home.file()).unwrap();
    assert_eq!(again.list(), store.list());
    assert_eq!(again.active().unwrap().id, "k-0011aabb");
}

#[test]
fn version_1_active_on_a_linked_record_falls_back() {
    let home = TempHome::new();
    std::fs::create_dir_all(home.file().parent().unwrap()).unwrap();
    std::fs::write(home.file(), v1_file("u-link-2")).unwrap();
    let store = AccountStore::load(home.file()).unwrap();
    assert_eq!(store.active().unwrap().id, "u-own-1");
    // Version 2 never holds linked records: one is a schema error.
    let v2 = v1_file("u-own-1").replace(r#""version":1"#, r#""version":2"#);
    std::fs::write(home.file(), v2).unwrap();
    assert!(matches!(
        AccountStore::load(home.file()),
        Err(StoreError::Json(_))
    ));
}

#[test]
fn unknown_top_level_keys_survive_save() {
    let home = TempHome::new();
    std::fs::create_dir_all(home.file().parent().unwrap()).unwrap();
    let text = r#"{"version":1,"active":null,"accounts":[],"usage":{"u-a":1}}"#;
    std::fs::write(home.file(), text).unwrap();
    let mut store = AccountStore::load(home.file()).unwrap();
    store.update(WAIT, |_| Ok(())).unwrap();
    let raw: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(home.file()).unwrap()).unwrap();
    assert_eq!(raw["usage"]["u-a"], 1);
}

#[test]
fn debug_lists_ids_only() {
    let home = TempHome::new();
    let mut store = AccountStore::load(home.file()).unwrap();
    store
        .upsert(account("u-a", AccountKind::Own), None)
        .unwrap();
    let text = format!("{store:?}");
    assert!(text.contains("u-a"));
    assert!(!text.contains("example.test"));
}

#[cfg(windows)]
mod encrypted {
    use super::*;

    fn own_secret(n: u32) -> Secret {
        Secret::Own {
            refresh: format!("test-refresh-{n}"),
            access: format!("test-access-{n}"),
            expires_at: 1_794_400_000_000,
        }
    }

    #[test]
    fn secrets_are_encrypted_and_round_trip() {
        let home = TempHome::new();
        let mut store = AccountStore::load(home.file()).unwrap();
        let key = Secret::ApiKey {
            key: "test-key-1".into(),
        };
        let key_id = Account::api_key_id("test-key-1");
        store
            .update(WAIT, |store| {
                store.upsert(account(&key_id, AccountKind::ApiKey), Some(key.clone()))?;
                store.upsert(account("u-own", AccountKind::Own), Some(own_secret(1)))
            })
            .unwrap();

        let text = std::fs::read_to_string(home.file()).unwrap();
        for plain in ["test-key-1", "test-access-1", "test-refresh-1"] {
            assert!(!text.contains(plain), "plaintext secret on disk");
        }
        let loaded = AccountStore::load(home.file()).unwrap();
        assert_eq!(loaded.secret(&key_id).unwrap(), Some(key));
        assert_eq!(loaded.secret("u-own").unwrap(), Some(own_secret(1)));
    }

    #[test]
    fn upsert_without_secret_keeps_it_and_set_secret_replaces_it() {
        let home = TempHome::new();
        let mut store = AccountStore::load(home.file()).unwrap();
        store
            .upsert(account("u-own", AccountKind::Own), Some(own_secret(1)))
            .unwrap();
        store
            .upsert(account("u-own", AccountKind::Own), None)
            .unwrap();
        assert_eq!(store.secret("u-own").unwrap(), Some(own_secret(1)));
        store.set_secret("u-own", own_secret(2)).unwrap();
        assert_eq!(store.secret("u-own").unwrap(), Some(own_secret(2)));
    }

    #[test]
    fn tampered_secret_is_an_error() {
        let home = TempHome::new();
        let mut store = AccountStore::load(home.file()).unwrap();
        store
            .update(WAIT, |store| {
                store.upsert(account("u-own", AccountKind::Own), Some(own_secret(1)))
            })
            .unwrap();
        let mut raw: serde_json::Value =
            serde_json::from_str(&std::fs::read_to_string(home.file()).unwrap()).unwrap();
        raw["accounts"][0]["secret"] = "AAAA".into();
        std::fs::write(home.file(), raw.to_string()).unwrap();
        let loaded = AccountStore::load(home.file()).unwrap();
        assert!(loaded.secret("u-own").is_err());
        raw["accounts"][0]["secret"] = "not base64!".into();
        std::fs::write(home.file(), raw.to_string()).unwrap();
        let loaded = AccountStore::load(home.file()).unwrap();
        assert!(matches!(
            loaded.secret("u-own"),
            Err(StoreError::BadSecret(_))
        ));
    }
}
