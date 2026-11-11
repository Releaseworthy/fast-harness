//! Google OAuth client, authorize URL, token exchange, refresh and userinfo.
//! Fake ids only; no call reaches Google.

use fast_harness_core::accounts::google::{
    begin_login, client_from, save_client, GoogleApi, GoogleClient, GoogleError,
};
use fast_harness_core::http::{ScriptedResponse, ScriptedTransport};
use fast_harness_core::llm::antigravity::hub::Hub;
use std::path::PathBuf;
use std::sync::atomic::{AtomicU32, Ordering};

const CLIENT_ID: &str = "test-client.apps.example";
const CLIENT_SECRET: &str = "fake-client-secret-123";

struct TempHome(PathBuf);

impl TempHome {
    fn new() -> TempHome {
        static COUNTER: AtomicU32 = AtomicU32::new(0);
        let n = COUNTER.fetch_add(1, Ordering::SeqCst);
        let dir = std::env::temp_dir().join(format!(
            "fast-harness-google-oauth-{}-{n}",
            std::process::id()
        ));
        let _ = std::fs::remove_dir_all(&dir);
        TempHome(dir)
    }
}

impl Drop for TempHome {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

fn api() -> GoogleApi {
    GoogleApi {
        authorize_url: "https://auth.example/o/oauth2/v2/auth".into(),
        token_url: "https://token.example/token".into(),
        userinfo_url: "https://userinfo.example/v1".into(),
        hub: Hub::new("https://manifest.example/m.yml", &["https://daily.example"]),
        onboard_wait: std::time::Duration::ZERO,
    }
}

fn client() -> GoogleClient {
    GoogleClient {
        id: CLIENT_ID.into(),
        secret: CLIENT_SECRET.into(),
    }
}

fn reply(status: u16, body: &str) -> ScriptedResponse {
    ScriptedResponse::new(status, body, 2)
}

#[test]
fn client_from_env_file_or_none() {
    let home = TempHome::new();
    let env = client_from(Some(CLIENT_ID.into()), Some(CLIENT_SECRET.into()), &home.0).unwrap();
    assert_eq!(env, client());
    let err = client_from(Some(CLIENT_ID.into()), None, &home.0).unwrap_err();
    assert!(matches!(err, GoogleError::NoClient));
    let text = err.to_string();
    assert!(text.contains("FAST_HARNESS_GOOGLE_CLIENT_ID"));
    assert!(text.contains("accounts google-client --id"));
    let shown = format!("{:?}", client());
    assert!(!shown.contains(CLIENT_SECRET) && !shown.contains(CLIENT_ID));
}

#[test]
fn saved_client_is_encrypted_and_read_back() {
    let home = TempHome::new();
    save_client(&home.0, CLIENT_ID, &format!(" {CLIENT_SECRET}\n")).unwrap();
    let raw = std::fs::read_to_string(home.0.join("google-client.json")).unwrap();
    assert!(!raw.contains(CLIENT_SECRET));
    let parsed: serde_json::Value = serde_json::from_str(&raw).unwrap();
    assert_eq!(parsed["version"], 1);
    assert_eq!(parsed["id"], CLIENT_ID);
    assert_eq!(client_from(None, None, &home.0).unwrap(), client());
    assert!(save_client(&home.0, CLIENT_ID, "  ").is_err());
    std::fs::write(
        home.0.join("google-client.json"),
        r#"{"version":2,"id":"a","secret":"b"}"#,
    )
    .unwrap();
    let err = client_from(None, None, &home.0).unwrap_err();
    assert!(err.to_string().contains("version 2"));
}

#[test]
fn authorize_url_has_antigravity_parameters() {
    let flow = begin_login(&api(), &client()).unwrap();
    let url = flow.url();
    assert!(url
        .starts_with("https://auth.example/o/oauth2/v2/auth?client_id=test-client.apps.example&"));
    assert!(flow.redirect_uri().starts_with("http://localhost:"));
    assert!(flow.redirect_uri().ends_with("/oauth-callback"));
    for part in [
        "response_type=code",
        "redirect_uri=http%3A%2F%2Flocalhost%3A",
        "scope=https%3A%2F%2Fwww.googleapis.com%2Fauth%2Fcloud-platform+https",
        "auth%2Fcclog+https%3A%2F%2Fwww.googleapis.com%2Fauth%2Fexperimentsandconfigs",
        "code_challenge_method=S256",
        "access_type=offline",
        "prompt=consent+select_account",
        &format!("state={}", flow.state()),
    ] {
        assert!(url.contains(part), "{part} missing");
    }
    assert!(!url.contains(CLIENT_SECRET));
}

#[test]
fn exchange_posts_the_form_and_reads_tokens() {
    let flow = begin_login(&api(), &client()).unwrap();
    let body = r#"{"access_token":"fake-access","refresh_token":"fake-refresh","expires_in":3599}"#;
    let http = ScriptedTransport::new(vec![reply(200, body)]);
    let tokens = api()
        .exchange_code(&http, &client(), &flow, "fake/code")
        .unwrap();
    assert_eq!(tokens.access, "fake-access");
    assert_eq!(tokens.refresh, "fake-refresh");
    assert!(tokens.expires_at > fast_harness_core::accounts::model::now_unix_ms());
    let request = &http.requests()[0];
    assert_eq!(request.method, "POST");
    assert_eq!(request.url, "https://token.example/token");
    let form = &request.body;
    assert!(
        form.starts_with("client_id=test-client.apps.example&client_secret=fake-client-secret-123")
    );
    assert!(form.contains(
        "&code=fake%2Fcode&grant_type=authorization_code&redirect_uri=http%3A%2F%2Flocalhost"
    ));
    assert!(form.contains("&code_verifier="));
    let content = request.headers.iter().find(|(k, _)| k == "content-type");
    assert_eq!(content.unwrap().1, "application/x-www-form-urlencoded");
    assert!(!format!("{tokens:?}").contains("fake-access"));
}

#[test]
fn refresh_keeps_the_old_refresh_token() {
    let http = ScriptedTransport::new(vec![reply(200, r#"{"access_token":"new-access"}"#)]);
    let tokens = api().refresh(&http, &client(), "old-refresh").unwrap();
    assert_eq!(tokens.refresh, "old-refresh");
    assert_eq!(tokens.access, "new-access");
    let form = &http.requests()[0].body;
    assert!(form.starts_with("grant_type=refresh_token&refresh_token=old-refresh&client_id="));
}

#[test]
fn refused_refresh_is_dead_or_refused_without_secrets() {
    let dead =
        r#"{"error":"invalid_grant","error_description":"Token has been expired or revoked."}"#;
    let http = ScriptedTransport::new(vec![reply(400, dead)]);
    let err = api().refresh(&http, &client(), "old-refresh").unwrap_err();
    assert!(
        matches!(&err, GoogleError::Dead(reason) if reason.starts_with("invalid_grant: Token"))
    );
    let http = ScriptedTransport::new(vec![reply(500, "oops")]);
    let err = api().refresh(&http, &client(), "old-refresh").unwrap_err();
    assert!(matches!(
        err,
        GoogleError::Refused {
            status: 500,
            reason: None
        }
    ));
    let http = ScriptedTransport::new(vec![reply(200, r#"{"token_type":"Bearer"}"#)]);
    let err = api().refresh(&http, &client(), "old-refresh").unwrap_err();
    assert!(!err.to_string().contains("old-refresh"));
    assert!(!err.to_string().contains(CLIENT_SECRET));
}

#[test]
fn userinfo_reads_id_email_and_name() {
    let body = r#"{"id":"123456789","email":"ada@example.com","name":"Ada","picture":"x"}"#;
    let http = ScriptedTransport::new(vec![reply(200, body)]);
    let user = api().userinfo(&http, "fake-access").unwrap();
    assert_eq!(user.id, "123456789");
    assert_eq!(user.email.as_deref(), Some("ada@example.com"));
    assert_eq!(user.name.as_deref(), Some("Ada"));
    let request = &http.requests()[0];
    assert_eq!(request.url, "https://userinfo.example/v1");
    assert!(request
        .headers
        .contains(&("authorization".into(), "Bearer fake-access".into())));
    let http = ScriptedTransport::new(vec![reply(401, r#"{"error":"unauthorized"}"#)]);
    let err = api().userinfo(&http, "fake-access").unwrap_err();
    assert!(matches!(err, GoogleError::Refused { status: 401, .. }));
    let http = ScriptedTransport::new(vec![reply(200, r#"{"email":"a@example.com"}"#)]);
    assert!(matches!(
        api().userinfo(&http, "x"),
        Err(GoogleError::Invalid(_))
    ));
}
