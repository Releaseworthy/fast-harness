//! The Antigravity Hub: manifest version, User-Agent and host fallback.

use fast_harness_core::http::{ScriptedResponse, ScriptedTransport};
use fast_harness_core::llm::antigravity::hub::{
    compare_versions, hub_user_agent, next_host_on, parse_manifest_version, Hub, VERSION_FALLBACK,
};
use serde_json::json;
use std::cmp::Ordering;

const MANIFEST: &str = "https://manifest.example/latest-arm64-mac.yml";
const DAILY: &str = "https://daily.example";
const SANDBOX: &str = "https://sandbox.example";

fn hub() -> Hub {
    Hub::new(MANIFEST, &[DAILY, SANDBOX])
}

fn reply(status: u16, body: &str) -> ScriptedResponse {
    ScriptedResponse::new(status, body, 3)
}

fn manifest(version: &str) -> ScriptedResponse {
    reply(200, &format!("version: {version}\npath: Antigravity.zip\n"))
}

#[test]
fn manifest_version_forms() {
    let parse = |body: &str| parse_manifest_version(body);
    assert_eq!(
        parse("files:\nversion: 2.30.1\n").as_deref(),
        Some("2.30.1")
    );
    assert_eq!(parse("version: '2.30.1'").as_deref(), Some("2.30.1"));
    assert_eq!(
        parse("version: \"2.30.1\" # latest").as_deref(),
        Some("2.30.1")
    );
    assert_eq!(parse("version: 2.30"), None);
    assert_eq!(parse("version: v2.30.1"), None);
    assert_eq!(parse("  version: 2.30.1"), None);
    assert_eq!(parse("name: x"), None);
    assert_eq!(compare_versions("2.23.0", "2.22.9"), Ordering::Greater);
    assert_eq!(compare_versions("2.9.0", "2.23.0"), Ordering::Less);
    assert_eq!(compare_versions("2.23.0", "2.23.0"), Ordering::Equal);
}

#[test]
fn user_agent_names_known_changelists_only() {
    assert_eq!(
        hub_user_agent("2.29.0"),
        "antigravity/hub/2.29.0 (aidev_client; os_type=darwin; arch=arm64; cl=1013060670)"
    );
    assert_eq!(
        hub_user_agent("2.31.4"),
        "antigravity/hub/2.31.4 (aidev_client; os_type=darwin; arch=arm64)"
    );
}

#[test]
fn version_is_read_once_and_cached() {
    let http = ScriptedTransport::new(vec![manifest("2.31.4")]);
    let hub = hub();
    assert_eq!(hub.version(&http), "2.31.4");
    assert_eq!(hub.version(&http), "2.31.4");
    let requests = http.requests();
    assert_eq!(requests.len(), 1);
    assert_eq!(requests[0].method, "GET");
    assert_eq!(requests[0].url, MANIFEST);
}

#[test]
fn old_bad_or_missing_manifest_falls_back() {
    for script in [
        vec![manifest("2.22.9")],
        vec![reply(500, "version: 2.31.4")],
        vec![reply(200, "nothing here")],
        vec![],
    ] {
        let http = ScriptedTransport::new(script);
        assert_eq!(hub().version(&http), VERSION_FALLBACK);
    }
}

#[test]
fn control_call_moves_on_only_for_retryable_answers() {
    assert!(next_host_on(404) && next_host_on(408) && next_host_on(503));
    assert!(!next_host_on(400) && !next_host_on(401) && !next_host_on(429));
    for status in [404, 408, 500, 503] {
        let http = ScriptedTransport::new(vec![
            manifest("2.29.0"),
            reply(status, "{}"),
            reply(200, r#"{"ok":true}"#),
        ]);
        let body = json!({"a": 1});
        let answer = hub()
            .control_call(&http, "fake-access", "loadCodeAssist", &body, &[])
            .unwrap();
        assert_eq!(answer.status, 200);
        assert_eq!(answer.json().unwrap()["ok"], true);
        let requests = http.requests();
        assert_eq!(
            requests[1].url,
            format!("{DAILY}/v1internal:loadCodeAssist")
        );
        assert_eq!(
            requests[2].url,
            format!("{SANDBOX}/v1internal:loadCodeAssist")
        );
        assert_eq!(requests[2].body, r#"{"a":1}"#);
    }
}

#[test]
fn control_call_keeps_a_client_error_and_reports_the_last_host() {
    let http = ScriptedTransport::new(vec![manifest("2.29.0"), reply(429, "{}")]);
    let answer = hub()
        .control_call(&http, "fake-access", "m", &json!({}), &[])
        .unwrap();
    assert_eq!(answer.status, 429);
    assert_eq!(http.requests().len(), 2);

    let http = ScriptedTransport::new(vec![manifest("2.29.0"), reply(503, "{}"), reply(502, "x")]);
    let answer = hub()
        .control_call(&http, "fake-access", "m", &json!({}), &[])
        .unwrap();
    assert_eq!(answer.status, 502);
}

#[test]
fn network_error_moves_to_the_sandbox_then_fails() {
    // The scripted transport fails with a connect error once it runs dry.
    let http = ScriptedTransport::new(vec![manifest("2.29.0")]);
    let err = hub()
        .control_call(&http, "fake-access", "m", &json!({}), &[])
        .unwrap_err();
    assert_eq!(err.kind, fast_harness_core::http::HttpErrorKind::Connect);
    assert_eq!(http.requests().len(), 3);
}

#[test]
fn control_call_headers_and_overrides() {
    let http = ScriptedTransport::new(vec![manifest("2.29.0"), reply(200, "{}")]);
    let extra = [
        ("User-Agent", "custom-agent"),
        ("x-goog-api-client", "gl-node/1"),
    ];
    hub()
        .control_call(&http, "fake-access", "onboardUser", &json!({}), &extra)
        .unwrap();
    let headers = &http.requests()[1].headers;
    let get = |name: &str| {
        headers
            .iter()
            .filter(|(k, _)| k.eq_ignore_ascii_case(name))
            .map(|(_, v)| v.as_str())
            .collect::<Vec<_>>()
    };
    assert_eq!(get("authorization"), ["Bearer fake-access"]);
    assert_eq!(get("content-type"), ["application/json"]);
    assert_eq!(get("user-agent"), ["custom-agent"]);
    assert_eq!(get("x-goog-api-client"), ["gl-node/1"]);
}
