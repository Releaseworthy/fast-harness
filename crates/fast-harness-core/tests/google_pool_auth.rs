//! The Google pool as an AuthSource: leases, refresh, failover, holds.

mod google_pool_support;

use fast_harness_core::accounts::google::{HoldReason, GOOGLE_BROKEN_HOLD_MS};
use fast_harness_core::accounts::model::Secret;
use fast_harness_core::accounts::store::AccountStore;
use fast_harness_core::auth::{AuthSource, Failover, Lease};
use fast_harness_core::http::ScriptedTransport;
use fast_harness_core::llm::{Auth, ErrorKind, LlmError};
use google_pool_support::{add, home, manifest, quota, token, Home, HOUR, NOW};

fn status_of<'a>(
    list: &'a [fast_harness_core::accounts::google::GoogleStatus],
    id: &str,
) -> &'a fast_harness_core::accounts::google::GoogleStatus {
    list.iter().find(|s| s.account.id == id).unwrap()
}

fn lease(home: &Home) -> Lease {
    home.google.lease(&ScriptedTransport::new(vec![])).unwrap()
}

#[test]
#[cfg_attr(
    not(windows),
    ignore = "secrets are encrypted with DPAPI, which only Windows has"
)]
fn lease_refreshes_an_expiring_token_and_saves_it() {
    let home = home();
    let a = add(&home, "a", NOW + 60_000);
    home.google.use_account(&a).unwrap();
    let http = ScriptedTransport::new(vec![token("fake-access-new")]);
    let lease = home.google.lease(&http).unwrap();
    assert_eq!(lease.account, a);
    assert_eq!(lease.label, "a@example.com");
    let expected = Auth::Google {
        access: "fake-access-new".into(),
        project: "proj-a".into(),
    };
    assert_eq!(lease.auth, expected);
    let request = &http.requests()[0];
    assert_eq!(request.url, "https://token.example/token");
    assert!(request.body.contains("grant_type=refresh_token"));
    let store = AccountStore::load(home.dir.join("accounts.json")).unwrap();
    match store.secret(&a).unwrap() {
        Some(Secret::Google { access, .. }) => assert_eq!(access, "fake-access-new"),
        other => panic!("unexpected secret {other:?}"),
    }
    let text = std::fs::read_to_string(home.dir.join("accounts.json")).unwrap();
    assert!(!text.contains("fake-access-new"), "tokens are encrypted");
    // The fresh token is used again without a request.
    assert_eq!(lease_again(&home).auth, expected);
}

fn lease_again(home: &Home) -> Lease {
    lease(home)
}

#[test]
#[cfg_attr(
    not(windows),
    ignore = "secrets are encrypted with DPAPI, which only Windows has"
)]
fn lease_explains_a_missing_account_or_client() {
    let home = home();
    let err = lease_err(&home);
    assert!(err.contains("no Google account"), "{err}");
    // Expired: an expiring token would still be used when the refresh fails.
    add(&home, "a", NOW - 1);
    home.google.set_client(None);
    let err = lease_err(&home);
    if std::env::var("FAST_HARNESS_GOOGLE_CLIENT_ID").is_err() {
        assert!(err.contains("no Google OAuth client"), "{err}");
    }
}

fn lease_err(home: &Home) -> String {
    let http = ScriptedTransport::new(vec![]);
    home.google.lease(&http).unwrap_err().message
}

#[test]
#[cfg_attr(
    not(windows),
    ignore = "secrets are encrypted with DPAPI, which only Windows has"
)]
fn auth_error_refreshes_once_then_switches() {
    let home = home();
    let a = add(&home, "a", NOW + HOUR);
    let b = add(&home, "b", NOW + HOUR);
    home.google.use_account(&a).unwrap();
    let first = lease(&home);
    let err = LlmError::new(ErrorKind::Auth, "HTTP 401: invalid credentials");
    let http = ScriptedTransport::new(vec![token("fake-access-a2")]);
    assert_eq!(
        home.google.on_failure(&first, &err, "gemini-3-pro", &http),
        Failover::Retry
    );
    let second = lease(&home);
    assert_eq!(second.account, a);
    assert!(matches!(&second.auth, Auth::Google { access, .. } if access == "fake-access-a2"));
    let none = ScriptedTransport::new(vec![]);
    assert_eq!(
        home.google.on_failure(&second, &err, "gemini-3-pro", &none),
        Failover::Retry
    );
    let status = home.google.status();
    let held = status_of(&status, &a).hold.clone().unwrap();
    assert_eq!(held.reason, HoldReason::Refused);
    assert_eq!(held.until, NOW + GOOGLE_BROKEN_HOLD_MS);
    assert!(status_of(&status, &b).active);
    assert_eq!(lease(&home).account, b);
}

#[test]
#[cfg_attr(
    not(windows),
    ignore = "secrets are encrypted with DPAPI, which only Windows has"
)]
fn rate_limit_holds_until_the_group_reset_and_switches() {
    let home = home();
    let a = add(&home, "a", NOW + HOUR);
    let b = add(&home, "b", NOW + HOUR);
    home.google.use_account(&a).unwrap();
    let first = lease(&home);
    let err = LlmError::new(ErrorKind::RateLimit, "RESOURCE_EXHAUSTED");
    let http = ScriptedTransport::new(vec![
        manifest(),
        quota(0.0, "2026-11-01T03:00:00Z", 0.5, "2026-11-01T04:00:00Z"),
    ]);
    let model = "antigravity/gemini-3-pro";
    assert_eq!(
        home.google.on_failure(&first, &err, model, &http),
        Failover::Retry
    );
    let request = &http.requests()[1];
    assert!(
        request.url.ends_with(":retrieveUserQuotaSummary"),
        "{}",
        request.url
    );
    assert!(request.body.contains("proj-a"));
    let status = home.google.status();
    let held = status_of(&status, &a).hold.clone().unwrap();
    assert_eq!(
        (held.until, held.reason),
        (NOW + 3 * HOUR, HoldReason::RateLimit)
    );
    assert!(status_of(&status, &a).quota.is_some());
    assert!(status_of(&status, &b).active);
    assert_eq!(lease(&home).account, b);
}

#[test]
#[cfg_attr(
    not(windows),
    ignore = "secrets are encrypted with DPAPI, which only Windows has"
)]
fn every_account_held_waits_for_the_soonest_hold() {
    let home = home();
    let a = add(&home, "a", NOW + HOUR);
    add(&home, "b", NOW + HOUR);
    home.google.use_account(&a).unwrap();
    let err = LlmError::new(ErrorKind::RateLimit, "429");
    let http = ScriptedTransport::new(vec![
        manifest(),
        quota(0.5, "2026-11-01T04:00:00Z", 0.0, "2026-11-01T03:00:00Z"),
        quota(0.5, "2026-11-01T04:00:00Z", 0.0, "2026-11-01T02:00:00Z"),
    ]);
    let first = lease(&home);
    let model = "claude-sonnet-4-6";
    assert_eq!(
        home.google.on_failure(&first, &err, model, &http),
        Failover::Retry
    );
    let second = lease(&home);
    assert_ne!(second.account, a);
    let wait = home.google.on_failure(&second, &err, model, &http);
    assert_eq!(wait, Failover::Wait(2 * HOUR));
}

#[test]
#[cfg_attr(
    not(windows),
    ignore = "secrets are encrypted with DPAPI, which only Windows has"
)]
fn unknown_reset_uses_retry_after_then_fifteen_minutes() {
    let home = home();
    let a = add(&home, "a", NOW + HOUR);
    home.google.use_account(&a).unwrap();
    let lease = lease(&home);
    let err = LlmError::new(ErrorKind::RateLimit, "429");
    // The quota read fails (no answer scripted): no reset, no retry delay.
    let http = ScriptedTransport::new(vec![]);
    let wait = home.google.on_failure(&lease, &err, "gemini-3-pro", &http);
    assert_eq!(wait, Failover::Wait(15 * 60_000));
    assert!(home.google.status()[0].last_error.is_some());
}

#[test]
#[cfg_attr(
    not(windows),
    ignore = "secrets are encrypted with DPAPI, which only Windows has"
)]
fn validation_hold_keeps_the_url() {
    let home = home();
    let a = add(&home, "a", NOW + HOUR);
    let b = add(&home, "b", NOW + HOUR);
    home.google.use_account(&a).unwrap();
    let first = lease(&home);
    let message = "HTTP 403: Validation required, verify at \
https://accounts.example.com/verify?id=fake-1.";
    let err = LlmError::new(ErrorKind::Auth, message);
    let http = ScriptedTransport::new(vec![]);
    assert_eq!(
        home.google.on_failure(&first, &err, "gemini-3-pro", &http),
        Failover::Retry
    );
    let status = home.google.status();
    let held = status_of(&status, &a).hold.clone().unwrap();
    assert_eq!(held.reason, HoldReason::Validation);
    assert_eq!(
        held.url.as_deref(),
        Some("https://accounts.example.com/verify?id=fake-1")
    );
    assert!(status_of(&status, &b).active);
    // Choosing the account lifts its hold.
    home.google.use_account(&a).unwrap();
    assert!(home.google.status().iter().all(|s| s.hold.is_none()));
}

/// The message the Antigravity provider makes of the live TOS 403.
const TOS_MESSAGE: &str = "account disabled by the provider: PERMISSION_DENIED (TOS_VIOLATION): \
This service has been disabled in this account for violation of Terms of Service.";

#[test]
#[cfg_attr(
    not(windows),
    ignore = "secrets are encrypted with DPAPI, which only Windows has"
)]
fn disabled_only_account_fails_at_once_without_a_hold_wait() {
    let home = home();
    let a = add(&home, "a", NOW + HOUR);
    home.google.use_account(&a).unwrap();
    let first = lease(&home);
    let err = LlmError::new(ErrorKind::Auth, TOS_MESSAGE);
    let http = ScriptedTransport::new(vec![]);
    let outcome = home.google.on_failure(&first, &err, "gemini-3-pro", &http);
    assert_eq!(outcome, Failover::Fail, "no hour-long wait, no refresh");
    assert!(http.requests().is_empty(), "no token refresh for a ban");
    let status = home.google.status();
    let held = status_of(&status, &a).hold.clone().unwrap();
    assert_eq!(held.reason, HoldReason::Disabled);
    assert_eq!(
        status_of(&status, &a).last_error.as_deref(),
        Some(TOS_MESSAGE)
    );
    // The account stays in the list.
    assert_eq!(status.len(), 1);
}

#[test]
#[cfg_attr(
    not(windows),
    ignore = "secrets are encrypted with DPAPI, which only Windows has"
)]
fn disabled_account_hands_over_to_another() {
    let home = home();
    let a = add(&home, "a", NOW + HOUR);
    let b = add(&home, "b", NOW + HOUR);
    home.google.use_account(&a).unwrap();
    let first = lease(&home);
    let err = LlmError::new(ErrorKind::Auth, TOS_MESSAGE);
    let http = ScriptedTransport::new(vec![]);
    assert_eq!(
        home.google.on_failure(&first, &err, "gemini-3-pro", &http),
        Failover::Retry
    );
    assert_eq!(lease(&home).account, b);
    // A rate limit on b never waits for a's permanent hold.
    let second = lease(&home);
    let limit = LlmError::new(ErrorKind::RateLimit, "429");
    let wait = home
        .google
        .on_failure(&second, &limit, "gemini-3-pro", &http);
    assert!(
        matches!(wait, Failover::Wait(ms) if ms <= 15 * 60_000),
        "{wait:?}"
    );
}

#[test]
#[cfg_attr(
    not(windows),
    ignore = "secrets are encrypted with DPAPI, which only Windows has"
)]
fn other_errors_and_foreign_leases_fail() {
    let home = home();
    add(&home, "a", NOW + HOUR);
    let lease = lease(&home);
    let http = ScriptedTransport::new(vec![]);
    let err = LlmError::new(ErrorKind::Overloaded, "busy");
    assert_eq!(
        home.google.on_failure(&lease, &err, "gemini-3-pro", &http),
        Failover::Fail
    );
    let foreign = Lease {
        auth: Auth::OAuth("fake".into()),
        ..lease
    };
    let err = LlmError::new(ErrorKind::RateLimit, "429");
    assert_eq!(
        home.google
            .on_failure(&foreign, &err, "gemini-3-pro", &http),
        Failover::Fail
    );
}
