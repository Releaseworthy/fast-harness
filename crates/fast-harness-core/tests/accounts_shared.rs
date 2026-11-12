//! One `accounts.json` shared by several processes (PLAN.md D56): two pools
//! opened on one home stand for two processes, and the cross-process test runs
//! this binary again as children. Secrets are encrypted with DPAPI, so these
//! tests run on Windows only, like the other secret tests.

mod agent_support;

use agent_support::{echo_registry, env, parts, text_reply, tool_reply};
use fast_harness_core::accounts::model::{now_unix_ms, Account, AccountKind, Secret};
use fast_harness_core::accounts::pool::AccountPool;
use fast_harness_core::accounts::store::AccountStore;
use fast_harness_core::agent::{Agent, Conversation};
use fast_harness_core::auth::{AuthSource, Failover};
use fast_harness_core::events::AgentEvent;
use fast_harness_core::http::{ScriptedResponse, ScriptedTransport};
use fast_harness_core::llm::{Auth, ErrorKind, LlmError};
use fast_harness_core::store::WAIT;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::atomic::AtomicBool;
use std::sync::Arc;

const ROLE: &str = "FH_ACCOUNTS_TEST_ROLE";
const HOME: &str = "FH_ACCOUNTS_TEST_HOME";
const INDEX: &str = "FH_ACCOUNTS_TEST_INDEX";

struct Home(PathBuf);

impl Home {
    fn new(name: &str) -> Home {
        let dir = std::env::temp_dir().join(format!(
            "fast-harness-accounts-shared-{name}-{}",
            std::process::id()
        ));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        Home(dir)
    }

    fn file(&self) -> PathBuf {
        self.0.join("accounts.json")
    }

    fn pool(&self) -> AccountPool {
        open(&self.0)
    }

    fn ids(&self) -> Vec<String> {
        let store = AccountStore::load(self.file()).unwrap();
        store.list().iter().map(|a| a.id.clone()).collect()
    }

    /// An own account whose access token expired, refresh token `refresh`.
    fn add_expired_own(&self, refresh: &str) -> String {
        let account = Account {
            id: Account::oauth_id("shared"),
            kind: AccountKind::Own,
            provider: Default::default(),
            email: Some("shared@example.test".into()),
            name: None,
            plan: Some("Max 20x".into()),
            added_at: 1,
        };
        let secret = Secret::Own {
            refresh: refresh.into(),
            access: "test-access-old".into(),
            expires_at: now_unix_ms() - 1_000,
        };
        let id = account.id.clone();
        let mut store = AccountStore::load(self.file()).unwrap();
        (store.update(WAIT, |store| store.upsert(account, Some(secret)))).unwrap();
        id
    }

    fn own_access(&self, id: &str) -> String {
        match AccountStore::load(self.file()).unwrap().secret(id).unwrap() {
            Some(Secret::Own { access, .. }) => access,
            _ => panic!("no own secret"),
        }
    }
}

impl Drop for Home {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

fn open(home: &Path) -> AccountPool {
    AccountPool::open(home.join("accounts.json"), home.join("usage.json")).unwrap()
}

fn token(access: &str, refresh: &str) -> ScriptedResponse {
    let body =
        format!(r#"{{"access_token":"{access}","refresh_token":"{refresh}","expires_in":28800}}"#);
    ScriptedResponse::new(200, &body, 11)
}

fn listed(pool: &AccountPool) -> usize {
    pool.status().len()
}

#[test]
#[cfg_attr(
    not(windows),
    ignore = "secrets are encrypted with DPAPI, which only Windows has"
)]
fn keys_added_by_two_pools_both_stay() {
    let home = Home::new("two-keys");
    let (a, b) = (home.pool(), home.pool());
    a.add_api_key("test-key-one", "").unwrap();
    b.add_api_key("test-key-two", "").unwrap();
    assert_eq!(home.ids().len(), 2, "b's save kept a's account");
    assert_eq!((listed(&a), listed(&b)), (2, 2));
}

#[test]
#[cfg_attr(
    not(windows),
    ignore = "secrets are encrypted with DPAPI, which only Windows has"
)]
fn set_active_and_failover_of_one_pool_keep_the_other_pools_accounts() {
    let home = Home::new("switch");
    let seed = home.pool();
    seed.add_api_key("test-key-zero", "").unwrap();
    let other = seed.add_api_key("test-key-other", "").unwrap().id;
    let (a, b) = (home.pool(), home.pool());
    let one = a.add_api_key("test-key-one", "").unwrap().id;
    b.set_active(&other).unwrap();
    assert!(home.ids().contains(&one), "set_active kept a's account");
    let http = ScriptedTransport::new(vec![]);
    let lease = b.lease(&http).unwrap();
    assert_eq!(lease.account, other);
    let two = a.add_api_key("test-key-two", "").unwrap().id;
    let limit = LlmError::new(ErrorKind::RateLimit, "429");
    let failover = b.on_failure(&lease, &limit, "claude-opus-5-5", &http);
    assert_eq!(failover, Failover::Retry);
    let ids = home.ids();
    assert_eq!(ids.len(), 4, "{ids:?}");
    assert!(ids.contains(&two), "the failover kept a's newest account");
    let store = AccountStore::load(home.file()).unwrap();
    assert_ne!(store.active().unwrap().id, other, "the failover switched");
    assert_eq!(b.save_error(), None);
}

/// Fails on the code before D56: pool b refreshed with the spent refresh
/// token it held, the scripted transport had no answer, and its lease failed.
#[test]
#[cfg_attr(
    not(windows),
    ignore = "secrets are encrypted with DPAPI, which only Windows has"
)]
fn a_token_refreshed_by_one_pool_is_used_by_the_other_without_a_request() {
    let home = Home::new("refresh");
    let id = home.add_expired_own("test-refresh-one");
    let (a, b) = (home.pool(), home.pool());
    let http_a = ScriptedTransport::new(vec![token("test-access-new", "test-refresh-two")]);
    assert_eq!(
        a.lease(&http_a).unwrap().auth,
        Auth::OAuth("test-access-new".into())
    );
    assert_eq!(http_a.requests().len(), 1);
    let http_b = ScriptedTransport::new(vec![]);
    let lease = b.lease(&http_b).unwrap();
    assert_eq!(lease.account, id);
    assert_eq!(lease.auth, Auth::OAuth("test-access-new".into()));
    assert!(http_b.requests().is_empty(), "b sent no refresh");
    let status = b.status();
    assert!(
        status[0].token_ok && status[0].held_until.is_none(),
        "not dead"
    );
    assert_eq!(home.own_access(&id), "test-access-new");
}

#[test]
#[cfg_attr(
    not(windows),
    ignore = "secrets are encrypted with DPAPI, which only Windows has"
)]
fn four_processes_adding_ten_keys_each_keep_all_forty() {
    let home = Home::new("processes");
    let children: Vec<_> = (0..4)
        .map(|i| {
            let mut cmd = Command::new(std::env::current_exe().unwrap());
            cmd.args([
                "--exact",
                "child_add_keys",
                "--nocapture",
                "--test-threads=1",
            ]);
            cmd.env(ROLE, "child_add_keys")
                .env(HOME, &home.0)
                .env(INDEX, i.to_string());
            (cmd.stdout(Stdio::piped()).stderr(Stdio::piped()))
                .spawn()
                .unwrap()
        })
        .collect();
    for child in children {
        let out = child.wait_with_output().unwrap();
        let err = String::from_utf8_lossy(&out.stderr);
        assert!(out.status.success(), "child failed: {err}");
    }
    assert_eq!(home.ids().len(), 40);
    assert_eq!(listed(&home.pool()), 40);
}

#[test]
fn child_add_keys() {
    if std::env::var(ROLE).as_deref() != Ok("child_add_keys") {
        return;
    }
    let home = PathBuf::from(std::env::var(HOME).unwrap());
    let index = std::env::var(INDEX).unwrap();
    let pool = open(&home);
    for n in 0..10 {
        pool.add_api_key(&format!("test-key-{index}-{n}"), "")
            .unwrap();
    }
}

#[test]
#[cfg_attr(
    not(windows),
    ignore = "secrets are encrypted with DPAPI, which only Windows has"
)]
fn a_removal_in_one_pool_shows_in_the_others_status() {
    let home = Home::new("reload");
    let (a, b) = (home.pool(), home.pool());
    let gone = a.add_api_key("test-key-gone", "").unwrap().id;
    a.add_api_key("test-key-kept", "").unwrap();
    assert_eq!(listed(&b), 2);
    a.remove(&gone).unwrap();
    let ids: Vec<String> = b.status().into_iter().map(|s| s.account.id).collect();
    assert_eq!(ids.len(), 1);
    assert!(!ids.contains(&gone));
}

/// Read-only on Windows: a rename over the file fails, reading still works.
fn set_read_only(path: &Path, on: bool) {
    let mut permissions = fs::metadata(path).unwrap().permissions();
    #[allow(clippy::permissions_set_readonly_false)]
    permissions.set_readonly(on);
    fs::set_permissions(path, permissions).unwrap();
}

#[test]
#[cfg_attr(
    not(windows),
    ignore = "secrets are encrypted with DPAPI, which only Windows has"
)]
fn a_failed_write_after_a_refresh_keeps_the_pair_warns_once_and_is_retried() {
    let home = Home::new("save-error");
    let id = home.add_expired_own("test-refresh-one");
    let pool = Arc::new(home.pool());
    set_read_only(&home.file(), true);
    let http = ScriptedTransport::new(vec![token("test-access-new", "test-refresh-two")]);
    let lease = pool.lease(&http).unwrap();
    assert_eq!(lease.auth, Auth::OAuth("test-access-new".into()));
    let error = pool.save_error().expect("the failed write is visible");
    assert!(
        !error.contains("test-access") && !error.contains("test-refresh"),
        "{error}"
    );
    assert_eq!(pool.warning(), Some(error));
    assert_eq!(home.own_access(&id), "test-access-old", "not written yet");

    let env = env("accounts-warning");
    let calls = [("toolu_1", "echo", r#"{"text":"x"}"#)];
    let replies = vec![tool_reply(&calls), text_reply("done")];
    let transport = Arc::new(ScriptedTransport::new(replies));
    let auth: Arc<dyn AuthSource> = pool.clone();
    let agent = Agent::new(parts(&transport, auth, &env.cwd), echo_registry());
    let mut conv = Conversation::create(&env.home, &env.cwd, None).unwrap();
    let mut seen = Vec::new();
    agent.run_turn(&mut conv, "go", &AtomicBool::new(false), &mut |e| {
        seen.push(e)
    });
    let warnings = (seen.iter())
        .filter(|e| matches!(e, AgentEvent::Error(m) if m.starts_with("accounts: ")))
        .count();
    assert_eq!(warnings, 1, "once per turn over two model calls");
    assert_eq!(transport.requests().len(), 2);

    set_read_only(&home.file(), false);
    assert_eq!(pool.status().len(), 1);
    assert_eq!(pool.save_error(), None, "the next pool call wrote the pair");
    assert_eq!(home.own_access(&id), "test-access-new");
    drop(env);
}
