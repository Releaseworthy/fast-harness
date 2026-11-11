//! AccountPool failover on rate limits and auth errors, against
//! `ScriptedTransport` and a temporary NH folder.

use fast_harness_core::accounts::model::{now_unix_ms, Account, AccountKind, Secret};
use fast_harness_core::accounts::pool::AccountPool;
use fast_harness_core::accounts::store::AccountStore;
use fast_harness_core::auth::{AuthSource, Failover, Lease};
use fast_harness_core::http::{HttpHead, ScriptedResponse, ScriptedTransport};
use fast_harness_core::llm::{Auth, ErrorKind, LlmError};
use std::fs;
use std::path::PathBuf;

const HOUR: u64 = 3_600_000;
const MODEL: &str = "claude-opus-5-5";

struct Fixture {
    dir: PathBuf,
}

impl Fixture {
    fn new(name: &str) -> Fixture {
        let dir = std::env::temp_dir().join(format!(
            "fast-harness-pool-fail-{name}-{}",
            std::process::id()
        ));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        Fixture { dir }
    }

    fn store_path(&self) -> PathBuf {
        self.dir.join("accounts.json")
    }

    fn add_own(&self, uuid: &str) -> String {
        let mut store = AccountStore::load(self.store_path()).unwrap();
        let account = Account {
            id: Account::oauth_id(uuid),
            kind: AccountKind::Own,
            email: None,
            name: None,
            plan: Some("Max 20x".into()),
            added_at: 1,
        };
        let secret = Secret::Own {
            refresh: format!("test-refresh-{uuid}"),
            access: format!("test-access-{uuid}"),
            expires_at: now_unix_ms() + 8 * HOUR,
        };
        let id = account.id.clone();
        store.upsert(account, Some(secret)).unwrap();
        store.save().unwrap();
        id
    }

    fn pool(&self) -> AccountPool {
        let usage = self.dir.join("usage.json");
        AccountPool::open(self.store_path(), usage).unwrap()
    }
}

impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.dir);
    }
}

fn lease_of(id: &str, access: &str) -> Lease {
    Lease {
        account: id.into(),
        label: id.into(),
        auth: Auth::OAuth(access.into()),
    }
}

/// Unified headers: seven-day percent (fraction) and reset in hours from now.
fn seven_day_head(seven: f64, reset_hours: u64) -> HttpHead {
    let now_s = now_unix_ms() / 1000;
    let p = "anthropic-ratelimit-unified-";
    let headers = vec![
        (format!("{p}5h-utilization"), "0.1".to_string()),
        (format!("{p}5h-reset"), (now_s + 3600).to_string()),
        (format!("{p}7d-utilization"), seven.to_string()),
        (
            format!("{p}7d-reset"),
            (now_s + reset_hours * 3600).to_string(),
        ),
        (format!("{p}status"), "allowed".to_string()),
    ];
    HttpHead {
        status: 200,
        headers,
    }
}

fn rate_limit(head: Option<HttpHead>) -> LlmError {
    LlmError {
        kind: ErrorKind::RateLimit,
        status: Some(429),
        message: "rate limited".into(),
        head,
    }
}

fn rejected_head(reset_in_s: u64) -> HttpHead {
    let reset = (now_unix_ms() / 1000 + reset_in_s).to_string();
    let p = "anthropic-ratelimit-unified-";
    HttpHead {
        status: 429,
        headers: vec![
            (format!("{p}status"), "rejected".to_string()),
            (format!("{p}reset"), reset),
        ],
    }
}

/// Leases and reports a rate limit without usage headers.
fn limit_active(pool: &AccountPool, http: &ScriptedTransport) -> Failover {
    let lease = pool.lease(http).unwrap();
    pool.on_failure(&lease, &rate_limit(None), MODEL, http)
}

fn active(pool: &AccountPool) -> String {
    let status = pool.status();
    status.into_iter().find(|s| s.active).unwrap().account.id
}

#[test]
fn rate_limit_fails_over_in_switch_order_with_the_48_hour_rule() {
    let fx = Fixture::new("order");
    let a = fx.add_own("aaaa");
    let b = fx.add_own("bbbb");
    let c = fx.add_own("cccc");
    let d = fx.add_own("dddd");
    let pool = fx.pool();
    pool.set_active(&a).unwrap();
    // b and d reset within 48 hours (d emptier), c is emptiest but resets in 4 days.
    pool.observe(&lease_of(&b, "x"), &seven_day_head(0.7, 30));
    pool.observe(&lease_of(&c, "x"), &seven_day_head(0.1, 96));
    pool.observe(&lease_of(&d, "x"), &seven_day_head(0.5, 40));
    let http = ScriptedTransport::new(Vec::new());
    let err = rate_limit(Some(rejected_head(2 * 3600)));
    assert_eq!(
        pool.on_failure(&lease_of(&a, "x"), &err, MODEL, &http),
        Failover::Retry
    );
    assert_eq!(active(&pool), d);
    let held = pool
        .status()
        .into_iter()
        .find(|s| s.account.id == a)
        .unwrap();
    let left = held.held_until.unwrap() - now_unix_ms() as i64;
    assert!(
        (2 * HOUR as i64 - 5_000..=2 * HOUR as i64).contains(&left),
        "{left}"
    );
    assert_eq!(limit_active(&pool, &http), Failover::Retry);
    assert_eq!(active(&pool), b);
    assert_eq!(limit_active(&pool, &http), Failover::Retry);
    assert_eq!(active(&pool), c);
    // Everyone is held now: wait for the soonest hold, at most one minute.
    assert_eq!(limit_active(&pool, &http), Failover::Wait(60_000));
    // After the wait the held active account is simply tried again.
    assert_eq!(pool.lease(&http).unwrap().account, c);
    assert!(http.requests().is_empty());
}

#[test]
fn wait_is_the_soonest_hold_when_under_a_minute() {
    let fx = Fixture::new("wait");
    let a = fx.add_own("aaaa");
    let pool = fx.pool();
    let http = ScriptedTransport::new(Vec::new());
    let lease = pool.lease(&http).unwrap();
    let err = rate_limit(Some(rejected_head(20)));
    let Failover::Wait(ms) = pool.on_failure(&lease, &err, MODEL, &http) else {
        panic!("expected a wait");
    };
    assert!((15_000..=20_000).contains(&ms), "{ms}");
    assert_eq!(active(&pool), a);
    let overload = LlmError::new(ErrorKind::Overloaded, "529");
    assert_eq!(
        pool.on_failure(&lease, &overload, MODEL, &http),
        Failover::Fail
    );
}

#[test]
fn own_auth_failure_forces_one_refresh_then_fails_over() {
    let fx = Fixture::new("own-auth");
    let a = fx.add_own("aaaa");
    let b = fx.add_own("bbbb");
    let pool = fx.pool();
    pool.set_active(&a).unwrap();
    let body =
        r#"{"access_token":"test-access-a2","refresh_token":"test-refresh-a2","expires_in":28800}"#;
    let http = ScriptedTransport::new(vec![ScriptedResponse::new(200, body, 8)]);
    let lease = pool.lease(&http).unwrap();
    let err = LlmError::new(ErrorKind::Auth, "401");
    assert_eq!(pool.on_failure(&lease, &err, MODEL, &http), Failover::Retry);
    let lease = pool.lease(&http).unwrap();
    assert_eq!(lease.auth, Auth::OAuth("test-access-a2".into()));
    // The new token fails too: no second refresh, the account is held and b takes over.
    assert_eq!(pool.on_failure(&lease, &err, MODEL, &http), Failover::Retry);
    assert_eq!(http.requests().len(), 1);
    assert_eq!(active(&pool), b);
    let held = pool
        .status()
        .into_iter()
        .find(|s| s.account.id == a)
        .unwrap();
    assert_eq!(held.hold_reason.as_deref(), Some("credentials refused"));
}

#[test]
fn own_auth_failure_with_a_refused_refresh_and_nobody_left_waits() {
    let fx = Fixture::new("own-dead");
    let a = fx.add_own("aaaa");
    let pool = fx.pool();
    let refused = ScriptedResponse::new(400, r#"{"error":"invalid_grant"}"#, 4);
    let http = ScriptedTransport::new(vec![refused]);
    let err = LlmError::new(ErrorKind::Auth, "401");
    let lease = pool.lease(&http).unwrap();
    assert_eq!(lease.auth, Auth::OAuth("test-access-aaaa".into()));
    assert_eq!(
        pool.on_failure(&lease, &err, MODEL, &http),
        Failover::Wait(60_000)
    );
    assert_eq!(http.requests().len(), 1);
    let held = pool.status().into_iter().find(|s| s.account.id == a);
    assert!(held.unwrap().held_until.is_some());
    // Within the same failure streak the refresh is never tried again.
    assert_eq!(
        pool.on_failure(&lease, &err, MODEL, &http),
        Failover::Wait(60_000)
    );
    assert_eq!(http.requests().len(), 1);
}
