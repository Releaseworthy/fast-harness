//! Records of a provider this build does not know (Google, whose code moved to
//! branch `antigravity`, PLAN.md D53) load without error, are not listed, and
//! come back unchanged on save, so the old branch still finds its accounts.

use fast_harness_core::accounts::model::{Account, AccountKind, Provider};
use fast_harness_core::accounts::store::AccountStore;
use serde_json::{json, Value};
use std::path::PathBuf;

struct TempDir(PathBuf);

impl TempDir {
    fn new(tag: &str) -> TempDir {
        let dir = std::env::temp_dir().join(format!("fh-foreign-{tag}-{}", std::process::id()));
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

fn google_record() -> Value {
    json!({
        "id": "g-1098765",
        "kind": "own",
        "provider": "google",
        "email": "ana@example.com",
        "addedAt": 5,
        "secret": "bm90LWEtcmVhbC1zZWNyZXQ=",
        "futureField": {"kept": true}
    })
}

fn file() -> Value {
    json!({
        "version": 3,
        "active": "k-12345678",
        "activeGoogle": "g-1098765",
        "accounts": [
            {"id": "k-12345678", "kind": "apiKey", "provider": "anthropic", "addedAt": 1},
            google_record()
        ]
    })
}

fn read(path: &PathBuf) -> Value {
    serde_json::from_str(&std::fs::read_to_string(path).unwrap()).unwrap()
}

#[test]
fn unknown_provider_records_load_and_survive_save() {
    let dir = TempDir::new("keep");
    let path = dir.0.join("accounts.json");
    std::fs::write(&path, serde_json::to_string_pretty(&file()).unwrap()).unwrap();
    let mut store = AccountStore::load(&path).expect("a google record must not fail the load");
    let ids: Vec<&str> = store.list().iter().map(|a| a.id.as_str()).collect();
    assert_eq!(ids, ["k-12345678"], "the google record is not listed");
    assert_eq!(store.active().map(|a| a.id.as_str()), Some("k-12345678"));
    let added = Account {
        id: "k-87654321".into(),
        kind: AccountKind::ApiKey,
        provider: Provider::Anthropic,
        email: None,
        name: None,
        plan: None,
        added_at: 2,
    };
    store.upsert(added, None).unwrap();
    store.remove("k-12345678");
    store.save().unwrap();
    let saved = read(&path);
    assert_eq!(saved["activeGoogle"], "g-1098765");
    let accounts = saved["accounts"].as_array().unwrap();
    assert_eq!(accounts.len(), 2);
    assert_eq!(accounts[0]["id"], "k-87654321");
    assert_eq!(
        accounts[1],
        google_record(),
        "kept byte for byte in meaning"
    );
    let again = AccountStore::load(&path).unwrap();
    assert_eq!(again.list().len(), 1);
    again.save().unwrap();
    assert_eq!(read(&path)["accounts"][1], google_record());
}

#[test]
fn unknown_record_ids_are_not_accounts() {
    let dir = TempDir::new("ids");
    let path = dir.0.join("accounts.json");
    std::fs::write(&path, file().to_string()).unwrap();
    let mut store = AccountStore::load(&path).unwrap();
    assert!(store.get("g-1098765").is_none());
    assert!(store.set_active("g-1098765").is_err());
    assert!(store.remove("g-1098765").is_none());
    store.save().unwrap();
    assert_eq!(read(&path)["accounts"][1], google_record());
}
