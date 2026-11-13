//! The Claude Code version of OAuth calls (PLAN.md D71): one value per
//! process, so every step runs in one test, in order.

use std::path::PathBuf;
use std::sync::atomic::AtomicBool;

use fast_harness_core::http::{RecordedRequest, ScriptedResponse, ScriptedTransport};
use fast_harness_core::llm::anthropic::{claude_code, Anthropic};
use fast_harness_core::llm::{Auth, ChatRequest, LlmError, Provider};
use fast_harness_core::types::{Block, Message, Role};

const DONE: &str =
    "event: message_start\ndata: {\"type\":\"message_start\",\"message\":{\"usage\":{}}}\n\n\
event: message_stop\ndata: {\"type\":\"message_stop\"}\n\n";

/// The API's refusal of 2026-11-14 with `required` as the version asked for.
fn refusal(have: &str, required: &str) -> String {
    let message = format!(
        "Claude Code {have} does not support this model; version {required} or newer is \
         required. Run 'claude update', or update the Claude desktop app, then try again."
    );
    serde_json::json!({"type": "error", "error": {"type": "invalid_request_error", "message": message}})
        .to_string()
}

fn call(
    transport: &ScriptedTransport,
    auth: &Auth,
) -> (Result<(), LlmError>, Vec<RecordedRequest>) {
    let cancel = AtomicBool::new(false);
    let messages = vec![Message::new(Role::User, vec![Block::text("hi")])];
    let req = ChatRequest::new(
        "claude-opus-5-5",
        "Be brief.",
        &messages,
        &[],
        1024,
        &cancel,
    );
    let result = Anthropic::new("https://example.test")
        .stream(transport, auth, &req, &mut |_| {})
        .map(|_| ());
    (result, transport.requests())
}

fn agent(req: &RecordedRequest) -> String {
    let found = req.headers.iter().find(|(k, _)| k == "user-agent");
    found.map(|(_, v)| v.clone()).unwrap_or_default()
}

fn temp_home(name: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("fh-cc-{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

/// Only a newer plain release moves the version; a pin holds it.
fn offers_and_pins() {
    assert_eq!(claude_code::version(), claude_code::BUILTIN);
    assert!(!claude_code::offer("2.1.280"));
    assert!(!claude_code::offer("not a version"));
    claude_code::configure("2.1.300");
    assert_eq!(
        (claude_code::version(), claude_code::pinned()),
        ("2.1.300".into(), true)
    );
    assert!(!claude_code::offer("9.9.9"));
    claude_code::configure("auto");
    assert_eq!(claude_code::version(), claude_code::BUILTIN);
    assert!(!claude_code::pinned());
}

/// The bug of 2026-11-14: the refusal now raises the version and the same
/// call goes again; an API key call is not touched.
fn refusal_raises_the_version_and_sends_again() {
    let transport = ScriptedTransport::new(vec![
        ScriptedResponse::new(400, &refusal(claude_code::BUILTIN, "3.0.0"), 7),
        ScriptedResponse::new(200, DONE, 5),
    ]);
    let (result, sent) = call(&transport, &Auth::OAuth("t".into()));
    assert!(result.is_ok(), "{result:?}");
    let first = format!("claude-cli/{}", claude_code::BUILTIN);
    assert_eq!(
        (agent(&sent[0]), agent(&sent[1])),
        (first, "claude-cli/3.0.0".into())
    );
    assert_eq!(claude_code::version(), "3.0.0");
    // Asking for an older version than ours: no second call.
    let transport = ScriptedTransport::new(vec![
        ScriptedResponse::new(400, &refusal("3.0.0", "2.1.290"), 7),
        ScriptedResponse::new(200, DONE, 5),
    ]);
    let (result, sent) = call(&transport, &Auth::OAuth("t".into()));
    assert_eq!((result.unwrap_err().status, sent.len()), (Some(400), 1));
    let transport =
        ScriptedTransport::new(vec![ScriptedResponse::new(400, &refusal("x", "4.0.0"), 7)]);
    let (result, sent) = call(&transport, &Auth::ApiKey("k".into()));
    assert_eq!((result.unwrap_err().status, sent.len()), (Some(400), 1));
    assert_eq!(claude_code::version(), "3.0.0");
}

/// The registry's `latest` raises the version, lands in the cache and the
/// next request; a bad answer changes nothing.
fn update_reads_the_registry_and_caches() {
    let home = temp_home("update");
    let url = "https://registry.example.test/latest";
    let body = r#"{"name":"@anthropic-ai/claude-code","version":"3.2.1"}"#;
    let transport = ScriptedTransport::new(vec![ScriptedResponse::new(200, body, 9)]);
    assert_eq!(
        claude_code::update(&home, &transport, url).unwrap(),
        "3.2.1"
    );
    assert_eq!(transport.requests()[0].url, url);
    let cached = std::fs::read_to_string(home.join(claude_code::CACHE_FILE)).unwrap();
    assert!(cached.contains("\"version\": \"3.2.1\""), "{cached}");
    let (_, sent) = call(
        &ScriptedTransport::new(vec![ScriptedResponse::new(200, DONE, 5)]),
        &Auth::OAuth("t".into()),
    );
    assert_eq!(agent(&sent[0]), "claude-cli/3.2.1");
    for (status, body) in [(500, "oops"), (200, r#"{"version":"3.9.0-beta.1"}"#)] {
        let transport = ScriptedTransport::new(vec![ScriptedResponse::new(status, body, 4)]);
        assert!(claude_code::update(&home, &transport, url).is_err());
    }
    assert_eq!(claude_code::version(), "3.2.1");
    std::fs::write(
        home.join(claude_code::CACHE_FILE),
        r#"{"version":"5.0.0","fetchedAt":1}"#,
    )
    .unwrap();
    claude_code::load_cache(&home);
    assert_eq!(claude_code::version(), "5.0.0");
    std::fs::remove_dir_all(&home).unwrap();
}

#[test]
fn the_claude_code_version_moves_forward_only() {
    offers_and_pins();
    refusal_raises_the_version_and_sends_again();
    update_reads_the_registry_and_caches();
}
