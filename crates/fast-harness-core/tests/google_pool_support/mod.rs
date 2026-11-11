//! Shared fixtures of the Google pool tests: a temp home, fake accounts,
//! a fake OAuth client, a fixed clock and scripted Google answers.
#![allow(dead_code)]

use fast_harness_core::accounts::google::{GoogleApi, GoogleAuth, GoogleClient, SignIn};
use fast_harness_core::accounts::model::{Account, AccountKind, Provider, Secret};
use fast_harness_core::accounts::pool::AccountPool;
use fast_harness_core::http::ScriptedResponse;
use fast_harness_core::llm::antigravity::hub::Hub;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU32, Ordering};
use std::sync::Arc;
use std::time::Duration;

/// 2026-11-01T00:00:00Z.
pub const NOW: u64 = 1_793_491_200_000;
pub const MINUTE: u64 = 60_000;
pub const HOUR: u64 = 60 * MINUTE;

pub struct Home {
    pub dir: PathBuf,
    pub pool: Arc<AccountPool>,
    pub google: GoogleAuth,
}

impl Drop for Home {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.dir);
    }
}

pub fn api() -> GoogleApi {
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

/// A Google pool over `dir` with the fake client and the clock at [`NOW`].
pub fn google_at(dir: &Path, pool: &Arc<AccountPool>) -> GoogleAuth {
    let google = GoogleAuth::new(Arc::clone(pool), dir).with_api(api());
    google.set_client(Some(GoogleClient {
        id: "fake-client-id".into(),
        secret: "fake-client-secret".into(),
    }));
    google.set_clock(Some(NOW));
    google
}

pub fn home() -> Home {
    static COUNTER: AtomicU32 = AtomicU32::new(0);
    let n = COUNTER.fetch_add(1, Ordering::SeqCst);
    let dir = std::env::temp_dir().join(format!(
        "fast-harness-google-pool-{}-{n}",
        std::process::id()
    ));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    let pool =
        Arc::new(AccountPool::open(dir.join("accounts.json"), dir.join("usage.json")).unwrap());
    let google = google_at(&dir, &pool);
    Home { dir, pool, google }
}

/// Adds Google account `name` (`<name>@example.com`, project `proj-<name>`).
pub fn add(home: &Home, name: &str, expires_at: u64) -> String {
    let account = Account {
        id: Account::google_id(&format!("user-{name}")),
        kind: AccountKind::Own,
        provider: Provider::Google,
        email: Some(format!("{name}@example.com")),
        name: None,
        plan: None,
        added_at: 1,
    };
    let secret = Secret::Google {
        refresh: format!("fake-refresh-{name}"),
        access: format!("fake-access-{name}"),
        expires_at,
        project_id: Some(format!("proj-{name}")),
    };
    home.pool.add_google(SignIn { account, secret }).unwrap().id
}

pub fn reply(status: u16, body: &str) -> ScriptedResponse {
    ScriptedResponse::new(status, body, 3)
}

pub fn manifest() -> ScriptedResponse {
    reply(200, "version: 2.29.0\n")
}

pub fn token(access: &str) -> ScriptedResponse {
    reply(
        200,
        &format!(r#"{{"access_token":"{access}","expires_in":86400}}"#),
    )
}

/// A quota answer: the Gemini 5 h window has `gemini_left` until
/// `gemini_reset`, the Claude and GPT one `claude_left` until `claude_reset`.
pub fn quota(
    gemini_left: f64,
    gemini_reset: &str,
    claude_left: f64,
    claude_reset: &str,
) -> ScriptedResponse {
    let group = |name: &str, left: f64, reset: &str| {
        format!(
            r#"{{"displayName":"{name}","buckets":[
            {{"bucketId":"5h","window":"5h","remainingFraction":{left},"resetTime":"{reset}"}},
            {{"bucketId":"weekly","window":"weekly","remainingFraction":1,"resetTime":"2026-11-08T00:00:00Z"}}]}}"#
        )
    };
    let body = format!(
        r#"{{"groups":[{},{}]}}"#,
        group("Gemini Models", gemini_left, gemini_reset),
        group("Claude and GPT models", claude_left, claude_reset)
    );
    reply(200, &body)
}
