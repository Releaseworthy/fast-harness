//! AccountPool leases, linked accounts, observe and usage polling, against
//! `ScriptedTransport`, a temporary NH folder and a synthetic DSH file.

use nh_core::accounts::model::{now_unix_ms, Account, AccountKind, Secret};
use nh_core::accounts::oauth::TOKEN_URL;
use nh_core::accounts::pool::AccountPool;
use nh_core::accounts::remote::USAGE_URL;
use nh_core::accounts::store::AccountStore;
use nh_core::auth::AuthSource;
use nh_core::http::{HttpHead, ScriptedResponse, ScriptedTransport};
use nh_core::llm::{Auth, ErrorKind, LlmError};
use std::fs;
use std::path::PathBuf;

const HOUR: u64 = 3_600_000;

struct Fixture {
    dir: PathBuf,
    dsh: PathBuf,
    dsh_bytes: Vec<u8>,
}

impl Fixture {
    fn new(name: &str) -> Fixture {
        let dir = std::env::temp_dir().join(format!("nh-pool-lease-{name}-{}", std::process::id()));
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
        let mut text = String::from("# test fixture\nversion: 1\nrecords:\n");
        for (uuid, access, expires) in records {
            text.push_str(&format!(
                "  claude-switch/u-{uuid}:\n    kind: grant\n    payload:\n      type: oauth\n      \
refresh: test-refresh-{uuid}\n      access: {access}\n      expires: {expires}\n      account:\n        \
uuid: {uuid}\n        email: '{uuid}@example.test'\n        plan: Max 20x\n"
            ));
        }
        fs::write(&self.dsh, &text).unwrap();
        self.dsh_bytes = text.into_bytes();
    }

    fn store_path(&self) -> PathBuf {
        self.dir.join("accounts.json")
    }

    fn add_own(&self, uuid: &str, access: &str, expires_at: u64) -> String {
        let mut store = AccountStore::load(self.store_path()).unwrap();
        let account = Account {
            id: Account::oauth_id(uuid),
            kind: AccountKind::Own,
            email: Some(format!("{uuid}@example.test")),
            name: None,
            plan: Some("Max 20x".into()),
            added_at: 1,
        };
        let secret = Secret::Own {
            refresh: format!("test-refresh-{uuid}"),
            access: access.into(),
            expires_at,
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

    fn own_access(&self, id: &str) -> String {
        match AccountStore::load(self.store_path())
            .unwrap()
            .secret(id)
            .unwrap()
        {
            Some(Secret::Own { access, .. }) => access,
            other => panic!("unexpected secret {other:?}"),
        }
    }

    /// The pool never changes DSH's file.
    fn check_dsh(&self) {
        assert_eq!(fs::read(&self.dsh).unwrap(), self.dsh_bytes);
    }
}

impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.dir);
    }
}

fn token(access: &str) -> ScriptedResponse {
    let body = format!(
        r#"{{"access_token":"{access}","refresh_token":"test-refresh-new","expires_in":28800}}"#
    );
    ScriptedResponse::new(200, &body, 11)
}

fn usage_head(status: u16, five: f64, seven: f64) -> HttpHead {
    let now_s = now_unix_ms() / 1000;
    let p = "anthropic-ratelimit-unified-";
    let pairs = [
        (format!("{p}5h-utilization"), five.to_string()),
        (format!("{p}5h-reset"), (now_s + 3600).to_string()),
        (format!("{p}7d-utilization"), seven.to_string()),
        (format!("{p}7d-reset"), (now_s + 86_400 * 3).to_string()),
        (format!("{p}status"), "allowed".to_string()),
    ];
    HttpHead {
        status,
        headers: pairs.into_iter().collect(),
    }
}

#[test]
fn lease_refreshes_an_expiring_own_account_and_saves_the_pair() {
    let fx = Fixture::new("refresh");
    let id = fx.add_own("aaaa", "test-access-old", now_unix_ms() + 30_000);
    let pool = fx.pool();
    let http = ScriptedTransport::new(vec![token("test-access-new")]);
    let lease = pool.lease(&http).unwrap();
    assert_eq!(lease.account, id);
    assert_eq!(lease.label, "aaaa@example.test");
    assert_eq!(lease.auth, Auth::OAuth("test-access-new".into()));
    assert_eq!(http.requests()[0].url, TOKEN_URL);
    assert_eq!(fx.own_access(&id), "test-access-new");
    let again = pool.lease(&http).unwrap();
    assert_eq!(again.auth, Auth::OAuth("test-access-new".into()));
    assert_eq!(http.requests().len(), 1);
    assert!(pool.status()[0].active);
    fx.check_dsh();
}

#[test]
fn a_dead_refresh_holds_the_account_and_moves_on() {
    let fx = Fixture::new("dead");
    let a = fx.add_own("aaaa", "test-access-a", now_unix_ms() + 10_000);
    let b = fx.add_own("bbbb", "test-access-b", now_unix_ms() + 8 * HOUR);
    let pool = fx.pool();
    pool.set_active(&a).unwrap();
    let refused = ScriptedResponse::new(400, r#"{"error":"invalid_grant"}"#, 4);
    let http = ScriptedTransport::new(vec![refused]);
    let lease = pool.lease(&http).unwrap();
    assert_eq!(lease.account, b);
    let status = pool.status();
    let held = status.iter().find(|s| s.account.id == a).unwrap();
    assert_eq!(held.hold_reason.as_deref(), Some("refresh token refused"));
    let left = held.held_until.unwrap() - now_unix_ms() as i64;
    assert!((HOUR as i64 - 5_000..=HOUR as i64).contains(&left));
    assert!(status.iter().find(|s| s.account.id == b).unwrap().active);
    // The dead refresh token is never sent again, even as a last resort.
    pool.set_active(&a).unwrap();
    assert_eq!(pool.lease(&http).unwrap().account, b);
    assert_eq!(http.requests().len(), 1);
    fx.check_dsh();
}

#[test]
fn linked_tokens_are_read_fresh_and_expired_ones_skipped() {
    let mut fx = Fixture::new("linked");
    let now = now_unix_ms();
    fx.write_dsh(&[
        ("1111", "test-access-1", now - 1_000),
        ("2222", "test-access-2", now + HOUR),
    ]);
    let pool = fx.pool();
    assert_eq!(pool.import_linked().unwrap(), 2);
    assert_eq!(pool.import_linked().unwrap(), 0);
    let saved = fs::read_to_string(fx.store_path()).unwrap();
    assert!(!saved.contains("test-access") && !saved.contains("test-refresh"));
    pool.set_active("u-1111").unwrap();
    let http = ScriptedTransport::new(Vec::new());
    let lease = pool.lease(&http).unwrap();
    assert_eq!(lease.account, "u-2222");
    assert_eq!(lease.auth, Auth::OAuth("test-access-2".into()));
    let ok: Vec<bool> = pool.status().iter().map(|s| s.token_ok).collect();
    assert_eq!(ok, [false, true]);
    fx.check_dsh();
    fx.write_dsh(&[
        ("1111", "test-access-1b", now + HOUR),
        ("2222", "test-access-2b", now + HOUR),
    ]);
    assert_eq!(
        pool.lease(&http).unwrap().auth,
        Auth::OAuth("test-access-2b".into())
    );
    pool.set_active("u-1111").unwrap();
    assert_eq!(
        pool.lease(&http).unwrap().auth,
        Auth::OAuth("test-access-1b".into())
    );
    assert!(http.requests().is_empty());
    fx.check_dsh();
}

#[test]
fn api_keys_lease_the_key_and_fail_on_auth() {
    let fx = Fixture::new("apikey");
    let pool = fx.pool();
    let account = pool.add_api_key(" test-key-1 ", "work").unwrap();
    assert_eq!(account.name.as_deref(), Some("work"));
    let http = ScriptedTransport::new(Vec::new());
    let lease = pool.lease(&http).unwrap();
    assert_eq!(lease.auth, Auth::ApiKey("test-key-1".into()));
    assert_eq!(lease.label, "work");
    let err = LlmError::new(ErrorKind::Auth, "401");
    assert_eq!(
        pool.on_failure(&lease, &err, "claude-opus-5-5", &http),
        nh_core::auth::Failover::Fail
    );
    let saved = fs::read_to_string(fx.store_path()).unwrap();
    assert!(!saved.contains("test-key-1"));
    pool.remove(&account.id).unwrap();
    assert!(pool.lease(&http).is_err());
    fx.check_dsh();
}

#[test]
fn observe_records_header_usage_and_saves_the_snapshot() {
    let fx = Fixture::new("observe");
    let id = fx.add_own("aaaa", "test-access-a", now_unix_ms() + 8 * HOUR);
    let pool = fx.pool();
    let http = ScriptedTransport::new(Vec::new());
    let lease = pool.lease(&http).unwrap();
    pool.observe(&lease, &usage_head(200, 0.25, 0.6));
    let usage = pool.status()[0].usage.clone().unwrap();
    assert_eq!(usage.five_hour.unwrap().utilization, 25.0);
    assert_eq!(usage.seven_day.unwrap().utilization, 60.0);
    assert_eq!(usage.status.as_deref(), Some("allowed"));
    let snapshot = fs::read_to_string(fx.dir.join("usage.json")).unwrap();
    assert!(snapshot.contains(&id) && !snapshot.contains("test-access"));
    drop(pool);
    let reopened = fx.pool();
    let usage = reopened.status()[0].usage.clone().unwrap();
    assert_eq!(usage.seven_day.unwrap().utilization, 60.0);
    fx.check_dsh();
}

#[test]
fn poll_reads_idle_accounts_and_backs_off_after_429() {
    let mut fx = Fixture::new("poll");
    let now = now_unix_ms();
    fx.write_dsh(&[("2222", "test-access-2", now + HOUR)]);
    fx.add_own("aaaa", "test-access-a", now + 8 * HOUR);
    let pool = fx.pool();
    pool.import_linked().unwrap();
    pool.set_read_gap(std::time::Duration::ZERO);
    let body = r#"{"five_hour":{"utilization":5,"resets_at":"2030-01-01T00:00:00Z"},"seven_day":{"utilization":9}}"#;
    let limited = ScriptedResponse::new(429, "{}", 4).with_header("retry-after", "0");
    let http = ScriptedTransport::new(vec![ScriptedResponse::new(200, body, 5), limited]);
    assert_eq!(pool.poll_usage(&http), 1);
    let urls: Vec<String> = http.requests().into_iter().map(|r| r.url).collect();
    assert_eq!(urls, [USAGE_URL, USAGE_URL]);
    let auth = &http.requests()[1].headers[0].1;
    assert_eq!(auth, "Bearer test-access-2");
    assert_eq!(pool.poll_usage(&http), 0);
    assert_eq!(http.requests().len(), 2);
    let status = pool.status();
    assert_eq!(
        status[0]
            .usage
            .as_ref()
            .unwrap()
            .seven_day
            .as_ref()
            .unwrap()
            .utilization,
        9.0
    );
    assert!(status[1].usage.is_none());
    fx.check_dsh();
}
