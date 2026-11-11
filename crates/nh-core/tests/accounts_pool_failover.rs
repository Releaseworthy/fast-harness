//! AccountPool failover on rate limits and auth errors, against
//! `ScriptedTransport`, a temporary NH folder and a synthetic DSH file.

use nh_core::accounts::model::{now_unix_ms, Account, AccountKind, Secret};
use nh_core::accounts::pool::AccountPool;
use nh_core::accounts::store::AccountStore;
use nh_core::auth::{AuthSource, Failover, Lease};
use nh_core::http::{HttpHead, ScriptedResponse, ScriptedTransport};
use nh_core::llm::{Auth, ErrorKind, LlmError};
use std::fs;
use std::path::PathBuf;

const HOUR: u64 = 3_600_000;
const MODEL: &str = "claude-opus-5-5";

struct Fixture {
    dir: PathBuf,
    dsh: PathBuf,
    dsh_bytes: Vec<u8>,
}

impl Fixture {
    fn new(name: &str) -> Fixture {
        let dir = std::env::temp_dir().join(format!("nh-pool-fail-{name}-{}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(dir.join("dsh")).unwrap();
        let mut fixture = Fixture {
            dsh: dir.join("dsh").join(".credentials.yaml"),
            dir,
            dsh_bytes: Vec::new(),
        };
        fixture.write_dsh(&[]);
        fixture
    }

    /// Writes the synthetic DSH file: `(uuid, access token, expiry ms)`.
    fn write_dsh(&mut self, records: &[(&str, &str, u64)]) {
        let mut text = String::from("version: 1\nrecords:\n");
        for (uuid, access, expires) in records {
            text.push_str(&format!(
                "  claude-switch/u-{uuid}:\n    payload:\n      type: oauth\n      \
refresh: test-refresh-{uuid}\n      access: {access}\n      expires: {expires}\n"
            ));
        }
        fs::write(&self.dsh, &text).unwrap();
        self.dsh_bytes = text.into_bytes();
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
        AccountPool::open(self.store_path(), Some(self.dsh.clone()), usage).unwrap()
    }

    fn check_dsh(&self) {
        assert_eq!(fs::read(&self.dsh).unwrap(), self.dsh_bytes);
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
    fx.check_dsh();
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
    fx.check_dsh();
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
    fx.check_dsh();
}

#[test]
fn linked_auth_failure_retries_only_on_a_new_token() {
    let mut fx = Fixture::new("linked-auth");
    let now = now_unix_ms();
    fx.write_dsh(&[
        ("1111", "test-access-1", now + HOUR),
        ("2222", "test-access-2", now + HOUR),
    ]);
    let pool = fx.pool();
    assert_eq!(pool.import_linked().unwrap(), 2);
    pool.set_active("u-1111").unwrap();
    let http = ScriptedTransport::new(Vec::new());
    let err = LlmError::new(ErrorKind::Auth, "401");
    let lease = pool.lease(&http).unwrap();
    assert_eq!(pool.on_failure(&lease, &err, MODEL, &http), Failover::Retry);
    assert_eq!(active(&pool), "u-2222");
    let held = pool
        .status()
        .into_iter()
        .find(|s| s.account.id == "u-1111")
        .unwrap();
    assert!(held.held_until.is_some());
    let lease = pool.lease(&http).unwrap();
    assert_eq!(lease.auth, Auth::OAuth("test-access-2".into()));
    fx.write_dsh(&[
        ("1111", "test-access-1", now + HOUR),
        ("2222", "test-access-2b", now + HOUR),
    ]);
    assert_eq!(pool.on_failure(&lease, &err, MODEL, &http), Failover::Retry);
    let fresh = pool
        .status()
        .into_iter()
        .find(|s| s.account.id == "u-2222")
        .unwrap();
    assert!(fresh.held_until.is_none());
    assert_eq!(
        pool.lease(&http).unwrap().auth,
        Auth::OAuth("test-access-2b".into())
    );
    // Unchanged again with nobody left: wait (at most a minute), never refresh.
    let lease = pool.lease(&http).unwrap();
    assert_eq!(
        pool.on_failure(&lease, &err, MODEL, &http),
        Failover::Wait(60_000)
    );
    assert!(http.requests().is_empty());
    fx.check_dsh();
}
