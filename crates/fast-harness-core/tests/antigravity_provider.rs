//! The Antigravity provider: request headers, host fallback, error mapping,
//! cancel and credentials, over a scripted transport.

mod antigravity_support;

use antigravity_support::*;
use fast_harness_core::accounts::google::{is_disabled, is_validation, validation_url};
use fast_harness_core::http::ScriptedResponse;
use fast_harness_core::llm::{Auth, ErrorKind, Provider};
use fast_harness_core::types::StopReason;
use serde_json::{json, Value};

fn done() -> ScriptedResponse {
    sse(&[event(json!([{"text": "ok"}]), Some("STOP"))])
}

fn header<'a>(headers: &'a [(String, String)], name: &str) -> Option<&'a str> {
    headers
        .iter()
        .find(|(key, _)| key.eq_ignore_ascii_case(name))
        .map(|(_, value)| value.as_str())
}

#[test]
fn request_carries_headers_user_agent_and_envelope() {
    let out = call(vec![manifest(), done()]);
    assert_eq!(out.ok().stop, StopReason::End);
    assert_eq!(out.requests.len(), 2);
    assert_eq!(out.requests[0].url, MANIFEST);
    let sent = &out.requests[1];
    assert_eq!(
        (sent.method.as_str(), sent.url.clone()),
        ("POST", format!("{DAILY}{PATH}"))
    );
    let bearer = format!("Bearer {TOKEN}");
    assert_eq!(
        header(&sent.headers, "authorization"),
        Some(bearer.as_str())
    );
    assert_eq!(
        header(&sent.headers, "content-type"),
        Some("application/json")
    );
    assert_eq!(header(&sent.headers, "accept"), Some("text/event-stream"));
    let agent = header(&sent.headers, "user-agent").unwrap_or("");
    assert!(
        agent.starts_with("antigravity/hub/2.30.0 (aidev_client;"),
        "{agent}"
    );
    let body: Value = serde_json::from_str(&sent.body).unwrap();
    assert_eq!(body["model"], "gemini-test");
    assert_eq!(body["project"], "fake-project");
    let session = body["request"]["sessionId"].as_str().unwrap_or("");
    assert!(session.starts_with('-') && session.len() > 2, "{session}");
}

#[test]
fn provider_id_and_stable_session() {
    let provider = provider();
    assert_eq!(provider.id(), "antigravity");
    assert_eq!(provider.session_id(), provider.session_id());
    assert_ne!(
        provider.session_id(),
        antigravity_support::provider().session_id()
    );
}

#[test]
fn rate_limit_carries_retry_delay() {
    let error = json!({"code": 429, "status": "RESOURCE_EXHAUSTED", "message": "Resource has been exhausted",
    "details": [
        {"@type": "type.googleapis.com/google.rpc.ErrorInfo", "reason": "RATE_LIMIT_EXCEEDED"},
        {"@type": "type.googleapis.com/google.rpc.RetryInfo", "retryDelay": "12.5s"}
    ]});
    let out = call(vec![manifest(), google_error(429, error)]);
    let err = out.err();
    assert_eq!((err.kind, err.status), (ErrorKind::RateLimit, Some(429)));
    assert_eq!(err.retry_after_ms(), Some(12_500));
    assert!(
        err.message.contains("RATE_LIMIT_EXCEEDED"),
        "{}",
        err.message
    );
    assert!(!err.retryable());
    assert_eq!(out.requests.len(), 2, "a 429 never moves to the next host");
}

#[test]
fn verification_request_is_detected_with_its_url() {
    let url = "https://accounts.google.com/signin/continue?sarp=1&scc=1";
    let error = json!({"code": 403, "status": "PERMISSION_DENIED",
        "message": "Verify your account to continue.",
        "details": [{"@type": "type.googleapis.com/google.rpc.ErrorInfo",
            "reason": "VALIDATION_REQUIRED", "metadata": {"validation_url": url}}]});
    let out = call(vec![manifest(), google_error(403, error)]);
    let err = out.err();
    assert_eq!(err.kind, ErrorKind::Auth);
    assert!(is_validation(&err.message), "{}", err.message);
    assert_eq!(validation_url(&err.message).as_deref(), Some(url));
    assert!(err.message.contains("Verify your account"));
}

#[test]
fn terms_of_service_ban_is_a_permanent_auth_error() {
    // The live body of 2026-11-12 (docs/perf.md, Experiments 2026-11-12).
    let error = json!({"code": 403, "status": "PERMISSION_DENIED",
        "message": "This service has been disabled in this account for violation of Terms of Service.",
        "details": [{"@type": "type.googleapis.com/google.rpc.ErrorInfo",
            "reason": "TOS_VIOLATION", "domain": "cloudcode-pa.googleapis.com"}]});
    let out = call(vec![manifest(), google_error(403, error)]);
    let err = out.err();
    assert_eq!(err.kind, ErrorKind::Auth);
    assert!(is_disabled(&err.message), "{}", err.message);
    assert!(!is_validation(&err.message));
    assert!(err.message.contains("Terms of Service"), "{}", err.message);
    // Matched by the words alone too, without the ErrorInfo reason.
    let bare = json!({"code": 403, "status": "PERMISSION_DENIED",
        "message": "This service has been disabled in this account for violation of Terms of Service."});
    assert!(is_disabled(
        &call(vec![manifest(), google_error(403, bare)])
            .err()
            .message
    ));
}

#[test]
fn plain_forbidden_is_auth_without_validation() {
    let error = json!({"code": 403, "status": "PERMISSION_DENIED", "message": "The caller does not have permission"});
    let out = call(vec![manifest(), google_error(403, error)]);
    let err = out.err();
    assert_eq!(err.kind, ErrorKind::Auth);
    assert!(!is_validation(&err.message));
    assert!(!is_disabled(&err.message));
}

#[test]
fn unauthorized_never_echoes_the_token() {
    let message = format!("Invalid token {TOKEN}");
    let error = json!({"code": 401, "status": "UNAUTHENTICATED", "message": message});
    let out = call(vec![manifest(), google_error(401, error)]);
    let err = out.err();
    assert_eq!((err.kind, err.status), (ErrorKind::Auth, Some(401)));
    assert!(err.message.contains("<redacted>"));
    assert_eq!(out.requests.len(), 2);
}

#[test]
fn server_failure_moves_to_the_next_host() {
    let busy = google_error(
        503,
        json!({"code": 503, "status": "UNAVAILABLE", "message": "busy"}),
    );
    let out = call(vec![manifest(), busy, done()]);
    assert_eq!(out.ok().message.text(), "ok");
    let urls: Vec<&str> = out.requests.iter().map(|r| r.url.as_str()).collect();
    let (daily, sandbox) = (format!("{DAILY}{PATH}"), format!("{SANDBOX}{PATH}"));
    assert_eq!(urls, [MANIFEST, daily.as_str(), sandbox.as_str()]);
}

#[test]
fn not_found_on_both_hosts_is_a_bad_request() {
    let missing = || {
        google_error(
            404,
            json!({"code": 404, "status": "NOT_FOUND", "message": "no model"}),
        )
    };
    let out = call(vec![manifest(), missing(), missing()]);
    let err = out.err();
    assert_eq!((err.kind, err.status), (ErrorKind::BadRequest, Some(404)));
    assert!(err.message.contains("no model"), "{}", err.message);
    assert_eq!(out.requests.len(), 3);
}

#[test]
fn last_host_statuses_map_to_kinds() {
    let cases = [
        (503, ErrorKind::Overloaded),
        (529, ErrorKind::Overloaded),
        (500, ErrorKind::Server),
        (400, ErrorKind::BadRequest),
    ];
    for (status, kind) in cases {
        let first = ScriptedResponse::new(500, "<html>oops</html>", 4);
        let second = ScriptedResponse::new(status, "", 4);
        let out = call(vec![manifest(), first, second]);
        assert_eq!(out.err().kind, kind, "{status}");
    }
}

#[test]
fn bad_request_on_the_first_host_stays_there() {
    let bad = google_error(
        400,
        json!({"code": 400, "status": "INVALID_ARGUMENT", "message": "bad schema"}),
    );
    let out = call(vec![manifest(), bad, done()]);
    assert_eq!(out.err().kind, ErrorKind::BadRequest);
    assert_eq!(out.requests.len(), 2);
}

#[test]
fn network_failure_on_every_host() {
    let out = call(vec![manifest()]);
    let err = out.err();
    assert_eq!(err.kind, ErrorKind::Network);
    assert!(err.retryable());
    assert_eq!(out.requests.len(), 3);
}

#[test]
fn cancel_stops_the_transfer() {
    let events: Vec<Value> = (0..20)
        .map(|n| event(json!([{"text": format!("t{n}")}]), None))
        .collect();
    let out = call_with(vec![manifest(), sse(&events)], &google(), true);
    assert_eq!(out.err().kind, ErrorKind::Aborted);
    assert_eq!(out.deltas, [r#"Text("t0")"#]);
}

#[test]
fn foreign_credentials_are_refused() {
    for auth in [
        Auth::OAuth("fake-oauth".into()),
        Auth::ApiKey("fake-key".into()),
    ] {
        let out = call_with(vec![manifest(), done()], &auth, false);
        assert_eq!(out.err().kind, ErrorKind::Auth);
        assert!(out.requests.is_empty());
    }
}

#[test]
fn a_model_without_the_prefix_is_a_bad_request() {
    let http = fast_harness_core::http::ScriptedTransport::new(vec![manifest(), done()]);
    let cancel = std::sync::atomic::AtomicBool::new(false);
    let req = fast_harness_core::llm::ChatRequest {
        model: "gemini-test",
        system: "",
        messages: &[],
        tools: &[],
        max_tokens: 16,
        effort: None,
        cache: Default::default(),
        session: "",
        cancel: &cancel,
    };
    let err = provider()
        .stream(&http, &google(), &req, &mut |_| {})
        .unwrap_err();
    assert_eq!(err.kind, ErrorKind::BadRequest);
    assert!(http.requests().is_empty());
}
