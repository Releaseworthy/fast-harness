use fast_harness_core::accounts::crypto::{hex, sha256};
use fast_harness_core::accounts::model::{now_unix_ms, Account, AccountKind, Secret};
use serde_json::json;

fn own_account() -> Account {
    Account {
        id: Account::oauth_id("0000aaaa-1111-2222-3333-444455556666"),
        kind: AccountKind::Own,
        provider: Default::default(),
        email: Some("one@example.test".into()),
        name: Some("Test One".into()),
        plan: Some("max".into()),
        added_at: 1_700_000_000_000,
    }
}

#[test]
fn account_serializes_camel_case() {
    let value = serde_json::to_value(own_account()).unwrap();
    assert_eq!(
        value,
        json!({
            "id": "u-0000aaaa-1111-2222-3333-444455556666",
            "kind": "own",
            "provider": "anthropic",
            "email": "one@example.test",
            "name": "Test One",
            "plan": "max",
            "addedAt": 1_700_000_000_000u64,
        })
    );
    let back: Account = serde_json::from_value(value).unwrap();
    assert_eq!(back, own_account());
    // Version 2 records have no provider: they are Anthropic accounts.
    let mut old = serde_json::to_value(own_account()).unwrap();
    old.as_object_mut().unwrap().remove("provider");
    let back: Account = serde_json::from_value(old).unwrap();
    assert_eq!(back, own_account());
    assert_eq!(Account::google_id("1098765"), "g-1098765");
}

#[test]
fn optional_fields_are_omitted() {
    let account = Account {
        id: Account::api_key_id("test-key-1"),
        kind: AccountKind::ApiKey,
        provider: Default::default(),
        email: None,
        name: None,
        plan: None,
        added_at: 5,
    };
    let value = serde_json::to_value(&account).unwrap();
    assert_eq!(
        value,
        json!({"id": account.id, "kind": "apiKey", "provider": "anthropic", "addedAt": 5})
    );
    assert_eq!(
        serde_json::to_value(AccountKind::Own).unwrap(),
        json!("own")
    );
    // Linked accounts were removed (PLAN.md D8): the kind no longer parses.
    assert!(serde_json::from_value::<AccountKind>(json!("linked")).is_err());
}

#[test]
fn ids() {
    assert_eq!(Account::oauth_id("ABCD-12_x.y"), "u-abcd-12xy");
    let id = Account::api_key_id("test-key-1");
    assert_eq!(id.len(), 10);
    assert_eq!(&id[2..], &hex(&sha256(b"test-key-1"))[..8]);
    assert!(id.starts_with("k-"));
}

#[test]
fn secret_debug_is_redacted() {
    let own = Secret::Own {
        refresh: "test-refresh-1".into(),
        access: "test-access-1".into(),
        expires_at: 42,
    };
    let key = Secret::ApiKey {
        key: "test-key-1".into(),
    };
    let text = format!("{own:?} {key:?} {own:#?}");
    assert!(!text.contains("test-refresh-1"));
    assert!(!text.contains("test-access-1"));
    assert!(!text.contains("test-key-1"));
    assert!(text.contains("<redacted>"));
    assert!(text.contains("42"));
}

#[test]
fn secret_serde_shape() {
    let own = Secret::Own {
        refresh: "test-refresh-1".into(),
        access: "test-access-1".into(),
        expires_at: 42,
    };
    let value = serde_json::to_value(&own).unwrap();
    assert_eq!(
        value,
        json!({"type": "own", "refresh": "test-refresh-1", "access": "test-access-1", "expiresAt": 42})
    );
    assert_eq!(serde_json::from_value::<Secret>(value).unwrap(), own);
    let key = json!({"type": "apiKey", "key": "test-key-1"});
    assert_eq!(
        serde_json::from_value::<Secret>(key).unwrap(),
        Secret::ApiKey {
            key: "test-key-1".into()
        }
    );
}

#[test]
fn clock_is_after_2020() {
    assert!(now_unix_ms() > 1_577_836_800_000);
}
