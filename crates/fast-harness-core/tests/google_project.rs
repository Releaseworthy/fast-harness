//! Google project, plan, quota and the whole sign-in, on a scripted transport.

use fast_harness_core::accounts::google::{
    begin_login, complete_sign_in, parse_quota_summary, GoogleApi, GoogleClient,
    FALLBACK_PROJECT_ID,
};
use fast_harness_core::accounts::model::{AccountKind, Provider, Secret};
use fast_harness_core::accounts::store::AccountStore;
use fast_harness_core::http::{ScriptedResponse, ScriptedTransport};
use fast_harness_core::llm::antigravity::hub::Hub;
use serde_json::json;
use std::path::PathBuf;
use std::time::Duration;

fn api() -> GoogleApi {
    GoogleApi {
        authorize_url: "https://auth.example/auth".into(),
        token_url: "https://token.example/token".into(),
        userinfo_url: "https://userinfo.example/v1".into(),
        hub: Hub::new(
            "https://manifest.example/m.yml",
            &["https://daily.example", "https://sandbox.example"],
        ),
        onboard_wait: Duration::ZERO,
    }
}

fn reply(status: u16, body: &str) -> ScriptedResponse {
    ScriptedResponse::new(status, body, 3)
}

fn manifest() -> ScriptedResponse {
    reply(200, "version: 2.29.0\n")
}

#[test]
fn load_code_assist_names_project_and_paid_tier() {
    let body = r#"{"cloudaicompanionProject":"proj-123","currentTier":{"id":"free-tier"},
        "paidTier":{"id":"g1-pro-tier"},"allowedTiers":[{"id":"free-tier","isDefault":true}]}"#;
    let http = ScriptedTransport::new(vec![manifest(), reply(200, body)]);
    let info = api()
        .load_project(&http, "fake-access", None, true)
        .unwrap();
    assert_eq!(info.project_id.as_deref(), Some("proj-123"));
    assert_eq!(info.tier.as_deref(), Some("g1-pro-tier"));
    assert_eq!(info.tier_name.as_deref(), Some("Google AI Pro"));
    let request = &http.requests()[1];
    assert_eq!(
        request.url,
        "https://daily.example/v1internal:loadCodeAssist"
    );
    assert_eq!(request.body, r#"{"metadata":{"ideType":"ANTIGRAVITY"}}"#);
}

#[test]
fn project_object_and_named_current_tier() {
    let body = r#"{"cloudaicompanionProject":{"id":"proj-obj"},"currentTier":{"id":"x-tier","name":"Custom"}}"#;
    let http = ScriptedTransport::new(vec![manifest(), reply(200, body)]);
    let info = api()
        .load_project(&http, "fake-access", None, true)
        .unwrap();
    assert_eq!(info.project_id.as_deref(), Some("proj-obj"));
    assert_eq!(info.tier_name.as_deref(), Some("Custom"));
}

#[test]
fn no_project_onboards_with_the_default_tier_and_polls() {
    let load = r#"{"allowedTiers":[{"id":"legacy-tier"},{"id":"free-tier","isDefault":true}]}"#;
    let pending = r#"{"name":"op/1","done":false}"#;
    let done = r#"{"done":true,"response":{"cloudaicompanionProject":{"id":"proj-new"}}}"#;
    let script = vec![
        manifest(),
        reply(200, load),
        reply(200, pending),
        reply(200, done),
    ];
    let http = ScriptedTransport::new(script);
    let info = api()
        .load_project(&http, "fake-access", None, true)
        .unwrap();
    assert_eq!(info.project_id.as_deref(), Some("proj-new"));
    assert_eq!(info.tier_name.as_deref(), Some("Free"));
    let requests = http.requests();
    assert_eq!(requests.len(), 4);
    assert_eq!(
        requests[2].url,
        "https://daily.example/v1internal:onboardUser"
    );
    let body: serde_json::Value = serde_json::from_str(&requests[2].body).unwrap();
    assert_eq!(body["tier_id"], "free-tier");
    assert_eq!(
        body["metadata"],
        json!({"ide_type":"ANTIGRAVITY","ide_version":"2.29.0","ide_name":"antigravity"})
    );
    let agent = requests[2]
        .headers
        .iter()
        .find(|(k, _)| k == "user-agent")
        .unwrap();
    assert!(agent.1.ends_with(") google-api-nodejs-client/10.3.0"));
}

#[test]
fn refused_onboarding_uses_the_fallback_project() {
    let http = ScriptedTransport::new(vec![manifest(), reply(200, "{}"), reply(403, "{}")]);
    let info = api()
        .load_project(&http, "fake-access", None, true)
        .unwrap();
    assert_eq!(info.project_id.as_deref(), Some(FALLBACK_PROJECT_ID));
    assert_eq!(info.tier, None);
}

#[test]
fn failed_load_keeps_the_hint() {
    let http = ScriptedTransport::new(vec![manifest(), reply(500, "x"), reply(503, "y")]);
    let info = api()
        .load_project(&http, "fake-access", Some("hint-1"), true)
        .unwrap();
    assert_eq!(info.project_id.as_deref(), Some("hint-1"));
    assert_eq!(http.requests().len(), 3);
}

#[test]
fn quota_summary_rules() {
    let reply = json!({"groups": [
        {"displayName": "Gemini Models", "buckets": [
            {"bucketId": "gemini-5h", "window": "5h", "remainingFraction": 0.25, "resetTime": "2026-11-12T05:00:00Z"},
            {"bucketId": "gemini-weekly", "window": "weekly", "remainingFraction": 1, "resetTime": "2026-11-19T00:00:00Z"}
        ]},
        {"description": "Claude and GPT models", "buckets": [
            {"window": "5h", "resetTime": "2026-11-12T06:00:00Z"},
            {"bucketId": "x-7d", "remainingFraction": 0.5, "disabled": true}
        ]},
        "junk"
    ]});
    let quota = parse_quota_summary(&reply).unwrap();
    assert_eq!(quota.groups.len(), 2);
    let gemini = &quota.groups[0];
    assert_eq!(gemini.name, "Gemini Models");
    let five = gemini.five_hour.as_ref().unwrap();
    assert!((five.used - 0.75).abs() < 1e-9);
    assert_eq!(five.resets_at, Some(1_794_459_600_000));
    assert_eq!(five.used_at(1_794_459_600_000), 0.0);
    let week = gemini.weekly.as_ref().unwrap();
    assert_eq!((week.used, week.resets_at), (0.0, None));
    let claude = &quota.groups[1];
    assert_eq!(claude.name, "Claude and GPT models");
    let five = claude.five_hour.as_ref().unwrap();
    assert_eq!(five.used, 1.0);
    assert!(five.resets_at.is_some());
    assert!(claude.weekly.as_ref().unwrap().disabled);
    assert!(parse_quota_summary(&json!({})).is_none());
}

#[test]
fn retrieve_quota_posts_the_project() {
    let body = r#"{"groups":[{"displayName":"Gemini Models","buckets":[]}]}"#;
    let http = ScriptedTransport::new(vec![manifest(), reply(200, body)]);
    let quota = api()
        .retrieve_quota(&http, "fake-access", "proj-123")
        .unwrap();
    assert_eq!(quota.groups[0].name, "Gemini Models");
    let request = &http.requests()[1];
    assert_eq!(
        request.url,
        "https://daily.example/v1internal:retrieveUserQuotaSummary"
    );
    assert_eq!(request.body, r#"{"project":"proj-123"}"#);
    let http = ScriptedTransport::new(vec![
        manifest(),
        reply(429, r#"{"error":{"status":"RESOURCE_EXHAUSTED"}}"#),
    ]);
    let err = api().retrieve_quota(&http, "fake-access", "p").unwrap_err();
    assert!(err.to_string().contains("RESOURCE_EXHAUSTED"));
}

#[test]
fn complete_sign_in_stores_an_active_google_account() {
    let dir =
        std::env::temp_dir().join(format!("fast-harness-google-signin-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    let path: PathBuf = dir.join("accounts.json");
    let mut store = AccountStore::load(&path).unwrap();
    let client = GoogleClient {
        id: "test-client.apps.example".into(),
        secret: "fake-secret".into(),
    };
    let flow = begin_login(&api(), &client).unwrap();
    let http = ScriptedTransport::new(vec![
        reply(
            200,
            r#"{"access_token":"fake-access","refresh_token":"fake-refresh","expires_in":3600}"#,
        ),
        reply(
            200,
            r#"{"id":"1098765","email":"ada@example.com","name":"Ada"}"#,
        ),
        manifest(),
        reply(
            200,
            r#"{"cloudaicompanionProject":"proj-9","paidTier":{"id":"g1-ultra-tier"}}"#,
        ),
    ]);
    let account = complete_sign_in(&mut store, &api(), &http, &client, &flow, "code-1").unwrap();
    assert_eq!(account.id, "g-1098765");
    assert_eq!(
        (account.kind, account.provider),
        (AccountKind::Own, Provider::Google)
    );
    assert_eq!(account.plan.as_deref(), Some("Google AI Ultra"));
    let loaded = AccountStore::load(&path).unwrap();
    assert_eq!(loaded.active_for(Provider::Google).unwrap().id, "g-1098765");
    assert!(loaded.active().is_none());
    match loaded.secret("g-1098765").unwrap() {
        Some(Secret::Google {
            refresh,
            project_id,
            ..
        }) => {
            assert_eq!(refresh, "fake-refresh");
            assert_eq!(project_id.as_deref(), Some("proj-9"));
        }
        other => panic!("unexpected secret {other:?}"),
    }
    let raw = std::fs::read_to_string(&path).unwrap();
    assert!(!raw.contains("fake-refresh") && !raw.contains("fake-access"));
    let _ = std::fs::remove_dir_all(&dir);
}
