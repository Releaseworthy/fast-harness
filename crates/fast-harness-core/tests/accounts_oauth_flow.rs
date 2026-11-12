//! OAuth sign-in, exchange, refresh, profile and usage reads against
//! `ScriptedTransport` and a loopback client thread. No real network.

use fast_harness_core::accounts::model::now_unix_ms;
use fast_harness_core::accounts::oauth::*;
use fast_harness_core::accounts::remote::{fetch_usage, USAGE_URL};
use fast_harness_core::http::{RecordedRequest, ScriptedResponse, ScriptedTransport};
use serde_json::Value;
use std::io::{Read, Write};
use std::net::TcpStream;
use std::sync::atomic::AtomicBool;
use std::thread;
use std::time::Duration;

fn decode(text: &str) -> String {
    let bytes = text.replace('+', " ").into_bytes();
    let mut out = Vec::new();
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'%' {
            let hex = std::str::from_utf8(&bytes[i + 1..i + 3]).unwrap();
            out.push(u8::from_str_radix(hex, 16).unwrap());
            i += 3;
        } else {
            out.push(bytes[i]);
            i += 1;
        }
    }
    String::from_utf8(out).unwrap()
}

fn query(url: &str) -> Vec<(String, String)> {
    let (_, query) = url.split_once('?').unwrap();
    query
        .split('&')
        .map(|pair| {
            let (k, v) = pair.split_once('=').unwrap();
            (decode(k), decode(v))
        })
        .collect()
}

fn param<'a>(params: &'a [(String, String)], key: &str) -> &'a str {
    &params.iter().find(|(k, _)| k == key).unwrap().1
}

fn header<'a>(req: &'a RecordedRequest, name: &str) -> &'a str {
    &req.headers.iter().find(|(k, _)| k == name).unwrap().1
}

/// Sends one GET to the flow's listener and returns the raw answer.
fn get(port: u16, target: &str) -> String {
    let mut stream = TcpStream::connect(("127.0.0.1", port)).unwrap();
    let request = format!("GET {target} HTTP/1.1\r\nHost: localhost:{port}\r\n\r\n");
    stream.write_all(request.as_bytes()).unwrap();
    let mut answer = String::new();
    stream.read_to_string(&mut answer).unwrap();
    answer
}

fn port_of(flow: &LoginFlow) -> u16 {
    let uri = flow.redirect_uri();
    let rest = uri.strip_prefix("http://localhost:").unwrap();
    rest.strip_suffix(CALLBACK_PATH).unwrap().parse().unwrap()
}

#[test]
fn authorize_url_carries_every_claude_switch_parameter() {
    let flow = begin_login().unwrap();
    assert!(flow.url().starts_with(&format!("{AUTHORIZE_URL}?")));
    let params = query(flow.url());
    let keys: Vec<&str> = params.iter().map(|(k, _)| k.as_str()).collect();
    let expected = [
        "code",
        "client_id",
        "response_type",
        "redirect_uri",
        "scope",
        "code_challenge",
        "code_challenge_method",
        "state",
    ];
    assert_eq!(keys, expected);
    assert_eq!(param(&params, "code"), "true");
    assert_eq!(param(&params, "client_id"), CLIENT_ID);
    assert_eq!(param(&params, "response_type"), "code");
    assert_eq!(param(&params, "redirect_uri"), flow.redirect_uri());
    assert_eq!(param(&params, "scope"), SCOPES);
    assert!(SCOPES.contains("user:inference user:sessions:claude_code"));
    assert_eq!(param(&params, "code_challenge").len(), 43);
    assert_eq!(param(&params, "code_challenge_method"), "S256");
    assert_eq!(param(&params, "state"), flow.state());
    assert!(port_of(&flow) > 0);
    let other = begin_login().unwrap();
    assert_ne!(other.state(), flow.state());
}

#[test]
fn callback_with_the_right_state_returns_the_code() {
    let flow = begin_login().unwrap();
    let port = port_of(&flow);
    let target = format!("/callback?code=test-code%2B1&state={}", flow.state());
    let client = thread::spawn(move || (get(port, "/favicon.ico"), get(port, &target)));
    let cancel = AtomicBool::new(false);
    let code = flow.wait(Duration::from_secs(20), &cancel).unwrap();
    let (favicon, page) = client.join().unwrap();
    assert_eq!(code, "test-code+1");
    assert!(favicon.starts_with("HTTP/1.1 404"));
    assert!(page.starts_with("HTTP/1.1 200"));
    assert!(page.contains("You can close this tab"));
}

#[test]
fn callback_with_another_state_is_refused() {
    let flow = begin_login().unwrap();
    let port = port_of(&flow);
    let client = thread::spawn(move || get(port, "/callback?code=test-code&state=other"));
    let cancel = AtomicBool::new(false);
    let err = flow.wait(Duration::from_secs(20), &cancel).unwrap_err();
    assert!(matches!(err, OAuthError::Mismatch));
    assert!(client.join().unwrap().starts_with("HTTP/1.1 400"));
}

#[test]
fn wait_ends_on_timeout_and_on_cancel() {
    let flow = begin_login().unwrap();
    let cancel = AtomicBool::new(false);
    let err = flow.wait(Duration::from_millis(80), &cancel).unwrap_err();
    assert!(matches!(err, OAuthError::Timeout));
    let cancel = AtomicBool::new(true);
    let err = flow.wait(Duration::from_secs(20), &cancel).unwrap_err();
    assert!(matches!(err, OAuthError::Cancelled));
}

#[test]
fn exchange_sends_the_pkce_grant_and_skews_the_expiry() {
    let flow = begin_login().unwrap();
    let body = r#"{"access_token":"test-access","refresh_token":"test-refresh","expires_in":3600}"#;
    let http = ScriptedTransport::new(vec![ScriptedResponse::new(200, body, 7)]);
    let before = now_unix_ms();
    let tokens = exchange_code(&http, &flow, "test-code").unwrap();
    assert_eq!(
        (tokens.access.as_str(), tokens.refresh.as_str()),
        ("test-access", "test-refresh")
    );
    let after = now_unix_ms();
    // The expiry is "now + 3600 s - 300 s skew", taken between `before` and `after`.
    let window = before + 3_290_000..=after + 3_300_000;
    assert!(window.contains(&tokens.expires_at), "{window:?}");
    assert!(!format!("{tokens:?}").contains("test-access"));
    let req = &http.requests()[0];
    assert_eq!((req.method.as_str(), req.url.as_str()), ("POST", TOKEN_URL));
    assert_eq!(header(req, "content-type"), "application/json");
    let sent: Value = serde_json::from_str(&req.body).unwrap();
    assert_eq!(sent["grant_type"], "authorization_code");
    assert_eq!(sent["client_id"], CLIENT_ID);
    assert_eq!(sent["code"], "test-code");
    assert_eq!(sent["state"], flow.state());
    assert_eq!(sent["redirect_uri"], flow.redirect_uri());
    assert_eq!(sent["code_verifier"].as_str().unwrap().len(), 43);
}

#[test]
fn refresh_keeps_the_old_refresh_token_when_none_comes_back() {
    let body = r#"{"access_token":"test-access-2","expires_in":"28800"}"#;
    let http = ScriptedTransport::new(vec![ScriptedResponse::new(200, body, 3)]);
    let tokens = refresh(&http, "test-refresh-1").unwrap();
    assert_eq!(tokens.refresh, "test-refresh-1");
    assert_eq!(tokens.access, "test-access-2");
    let sent: Value = serde_json::from_str(&http.requests()[0].body).unwrap();
    assert_eq!(sent["grant_type"], "refresh_token");
    assert_eq!(sent["refresh_token"], "test-refresh-1");
    assert_eq!(sent["client_id"], CLIENT_ID);
    assert!(sent.get("scope").is_none());
}

#[test]
fn a_refused_refresh_is_dead_only_for_the_dead_codes_on_400() {
    let cases = [
        (
            400,
            r#"{"error":"invalid_grant","error_description":"Refresh token not found"}"#,
            true,
        ),
        (
            400,
            r#"{"error":{"type":"invalid_client","message":"x"}}"#,
            true,
        ),
        (400, r#"{"error":"unauthorized_client"}"#, true),
        (400, r#"{"error":"invalid_grantx"}"#, false),
        (400, r#"{"error":"invalid_request"}"#, false),
        (401, r#"{"error":"invalid_grant"}"#, false),
        (500, "oops", false),
    ];
    for (status, body, dead) in cases {
        let http = ScriptedTransport::new(vec![ScriptedResponse::new(status, body, 5)]);
        let err = refresh(&http, "test-refresh").unwrap_err();
        assert_eq!(matches!(err, OAuthError::Dead(_)), dead, "{status} {body}");
        if !dead {
            assert!(matches!(err, OAuthError::Refused { status: s, .. } if s == status));
        }
    }
    let http = ScriptedTransport::new(vec![ScriptedResponse::new(200, "{}", 5)]);
    assert!(matches!(refresh(&http, "r"), Err(OAuthError::Invalid(_))));
}

#[test]
fn profile_reads_identity_and_plan() {
    let body = r#"{"account":{"uuid":"AAAA-1","email":"a@example.test","full_name":"A B"},
        "organization":{"organization_type":"claude_max","rate_limit_tier":"default_claude_max_20x"}}"#;
    let http = ScriptedTransport::new(vec![
        ScriptedResponse::new(200, body, 9),
        ScriptedResponse::new(200, r#"{"account":{}}"#, 9),
        ScriptedResponse::new(401, "{}", 9),
    ]);
    let profile = profile(&http, "test-access").unwrap();
    assert_eq!(profile.uuid, "AAAA-1");
    assert_eq!(profile.email.as_deref(), Some("a@example.test"));
    assert_eq!(profile.name.as_deref(), Some("A B"));
    assert_eq!(profile.plan.as_deref(), Some("Max 20x"));
    let req = &http.requests()[0];
    assert_eq!(
        (req.method.as_str(), req.url.as_str()),
        ("GET", PROFILE_URL)
    );
    assert_eq!(header(req, "authorization"), "Bearer test-access");
    assert_eq!(header(req, "anthropic-beta"), "oauth-2025-04-20");
    assert_eq!(
        header(req, "user-agent"),
        "claude-cli/2.1.0 (external, cli)"
    );
    assert!(matches!(
        fast_harness_core::accounts::oauth::profile(&http, "x"),
        Err(OAuthError::Invalid(_))
    ));
    assert!(matches!(
        fast_harness_core::accounts::oauth::profile(&http, "x"),
        Err(OAuthError::Refused { status: 401, .. })
    ));
}

#[test]
fn fetch_usage_parses_windows_and_reports_429_with_retry_after() {
    let body = r#"{"five_hour":{"utilization":12,"resets_at":"2030-01-01T00:00:00Z"},
        "seven_day":{"utilization":40.5,"resets_at":"2030-01-03T00:00:00Z"}}"#;
    let limited = ScriptedResponse::new(429, "{}", 4).with_header("retry-after", "30");
    let http = ScriptedTransport::new(vec![ScriptedResponse::new(200, body, 6), limited]);
    let usage = fetch_usage(&http, "test-access").unwrap();
    assert_eq!(usage.five_hour.unwrap().utilization, 12.0);
    assert_eq!(usage.seven_day.unwrap().utilization, 40.5);
    let req = &http.requests()[0];
    assert_eq!(req.url, USAGE_URL);
    assert_eq!(header(req, "anthropic-beta"), OAUTH_BETA);
    assert_eq!(header(req, "user-agent"), USER_AGENT);
    let err = fetch_usage(&http, "test-access").unwrap_err();
    assert!(err.is_rate_limit());
    assert_eq!(err.retry_after_ms(), Some(30_000));
}
