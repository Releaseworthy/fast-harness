//! Golden request bodies and headers of the Anthropic provider, and the
//! models list.

use std::sync::atomic::AtomicBool;

use fast_harness_core::http::{RecordedRequest, ScriptedResponse, ScriptedTransport};
use fast_harness_core::llm::anthropic::{builtin_models, list_models, Anthropic};
use fast_harness_core::llm::{Auth, ChatRequest, Provider, ToolSpec};
use fast_harness_core::types::{Block, Message, Role};
use serde_json::{json, Value};

const DONE: &str =
    "event: message_start\ndata: {\"type\":\"message_start\",\"message\":{\"usage\":{}}}\n\n\
event: message_stop\ndata: {\"type\":\"message_stop\"}\n\n";

fn tools() -> Vec<ToolSpec> {
    let schema = json!({"type": "object", "properties": {"path": {"type": "string"}},
        "required": ["path"], "additionalProperties": false});
    vec![
        ToolSpec {
            name: "read".into(),
            description: "Read a file".into(),
            schema,
        },
        ToolSpec {
            name: "shell".into(),
            description: "Run".into(),
            schema: json!({}),
        },
    ]
}

fn history() -> Vec<Message> {
    let assistant = vec![
        Block::Thinking {
            text: "plan".into(),
            signature: Some("sig".into()),
        },
        Block::Thinking {
            text: "unsigned".into(),
            signature: None,
        },
        Block::RedactedThinking {
            data: "opaque".into(),
        },
        Block::Text { text: "  ".into() },
        Block::ToolCall {
            id: "call.1".into(),
            name: "read".into(),
            arguments: "{\"path\":\"a\"}".into(),
            signature: None,
        },
        Block::ToolCall {
            id: "c2".into(),
            name: "shell".into(),
            arguments: "not json".into(),
            signature: None,
        },
    ];
    let result = |id: &str, content: &str, is_error| Message {
        role: Role::Tool,
        content: vec![Block::ToolResult {
            call_id: id.into(),
            content: content.into(),
            is_error,
        }],
        model: None,
    };
    vec![
        Message::user("hello"),
        Message::user(" "),
        Message {
            role: Role::Assistant,
            content: assistant,
            model: None,
        },
        result("call.1", "text of a", false),
        result("c2", "boom", true),
    ]
}

fn send(
    model: &str,
    auth: &Auth,
    effort: Option<&str>,
    cache_long: bool,
) -> (RecordedRequest, Value) {
    let transport = ScriptedTransport::new(vec![ScriptedResponse::new(200, DONE, 5)]);
    let cancel = AtomicBool::new(false);
    let (messages, tools) = (history(), tools());
    let req = ChatRequest {
        model,
        system: "Be brief.",
        messages: &messages,
        tools: &tools,
        max_tokens: 4096,
        effort,
        cache_long,
        cancel: &cancel,
    };
    Anthropic::new("https://example.test/")
        .stream(&transport, auth, &req, &mut |_| {})
        .unwrap();
    let recorded = transport.requests().remove(0);
    let body = serde_json::from_str(&recorded.body).unwrap();
    (recorded, body)
}

fn header<'a>(req: &'a RecordedRequest, name: &str) -> Option<&'a str> {
    req.headers
        .iter()
        .find(|(k, _)| k == name)
        .map(|(_, v)| v.as_str())
}

fn expected_messages(cache: &Value, read_name: &str) -> Value {
    json!([
        {"role": "user", "content": [{"type": "text", "text": "hello"}]},
        {"role": "assistant", "content": [
            {"type": "thinking", "thinking": "plan", "signature": "sig"},
            {"type": "text", "text": "unsigned"},
            {"type": "redacted_thinking", "data": "opaque"},
            {"type": "tool_use", "id": "call_1", "name": read_name, "input": {"path": "a"}},
            {"type": "tool_use", "id": "c2", "name": "shell", "input": {}},
        ]},
        {"role": "user", "content": [
            {"type": "tool_result", "tool_use_id": "call_1", "content": "text of a", "is_error": false},
            {"type": "tool_result", "tool_use_id": "c2", "content": "boom", "is_error": true, "cache_control": cache},
        ]},
    ])
}

fn expected_tools(cache: &Value, read_name: &str) -> Value {
    json!([
        {"name": read_name, "description": "Read a file", "eager_input_streaming": true,
         "input_schema": {"type": "object", "properties": {"path": {"type": "string"}}, "required": ["path"]}},
        {"name": "shell", "description": "Run", "eager_input_streaming": true,
         "input_schema": {"type": "object", "properties": {}, "required": []}, "cache_control": cache},
    ])
}

#[test]
fn api_key_golden_request() {
    let (req, body) = send(
        "claude-opus-4-7",
        &Auth::ApiKey("test-key".into()),
        Some("high"),
        true,
    );
    assert_eq!(
        (req.method.as_str(), req.url.as_str()),
        ("POST", "https://example.test/v1/messages?beta=true")
    );
    let headers: Vec<(&str, &str)> = req
        .headers
        .iter()
        .map(|(k, v)| (k.as_str(), v.as_str()))
        .collect();
    let agent = format!("fast-harness/{}", env!("CARGO_PKG_VERSION"));
    assert_eq!(
        headers,
        vec![
            ("accept", "application/json"),
            ("anthropic-dangerous-direct-browser-access", "true"),
            ("user-agent", agent.as_str()),
            ("x-api-key", "test-key"),
            ("anthropic-version", "2023-06-01"),
            ("content-type", "application/json"),
        ]
    );
    let cache = json!({"type": "ephemeral", "ttl": "1h"});
    let want = json!({
        "model": "claude-opus-4-7",
        "max_tokens": 4096,
        "stream": true,
        "system": [{"type": "text", "text": "Be brief.", "cache_control": cache}],
        "messages": expected_messages(&cache, "read"),
        "tools": expected_tools(&cache, "read"),
        "thinking": {"type": "adaptive", "display": "summarized"},
        "output_config": {"effort": "high"},
    });
    assert_eq!(body, want);
}

#[test]
fn oauth_golden_request() {
    let (req, body) = send(
        "claude-opus-5-5",
        &Auth::OAuth("test-token".into()),
        Some("low"),
        false,
    );
    assert_eq!(header(&req, "authorization"), Some("Bearer test-token"));
    assert_eq!(header(&req, "x-api-key"), None);
    assert_eq!(header(&req, "user-agent"), Some("claude-cli/2.1.280"));
    assert_eq!(header(&req, "x-app"), Some("cli"));
    assert_eq!(
        header(&req, "anthropic-beta"),
        Some("claude-code-20250219,oauth-2025-04-20,mid-conversation-output-config-2026-07-01,thinking-binding-controls-2026-08-01")
    );
    let cache = json!({"type": "ephemeral"});
    let mut messages = expected_messages(&cache, "Read");
    messages
        .as_array_mut()
        .unwrap()
        .push(json!({"role": "system", "content": [], "output_config": {"effort": "low"}}));
    let want = json!({
        "model": "claude-opus-5-5",
        "max_tokens": 4096,
        "stream": true,
        "system": [
            {"type": "text", "text": "You are Claude Code, Anthropic's official CLI for Claude.", "cache_control": cache},
            {"type": "text", "text": "Be brief.", "cache_control": cache},
        ],
        "messages": messages,
        "tools": expected_tools(&cache, "Read"),
        "thinking": {"type": "adaptive", "display": "summarized",
                     "block_binding": {"prefix_mismatch_behavior": "drop_block"}},
        "output_config": {"effort": "high"},
    });
    assert_eq!(body, want);
}

#[test]
fn budget_thinking_and_fallback_betas() {
    let (req, body) = send(
        "claude-haiku-4-5",
        &Auth::ApiKey("k".into()),
        Some("medium"),
        false,
    );
    assert_eq!(
        header(&req, "anthropic-beta"),
        Some("interleaved-thinking-2025-05-14")
    );
    assert_eq!(
        body["thinking"],
        json!({"type": "enabled", "budget_tokens": 8192, "display": "summarized"})
    );
    assert_eq!(body["max_tokens"], 4096 + 8192);
    let (req, body) = send(
        "claude-fable-5",
        &Auth::ApiKey("k".into()),
        Some("off"),
        false,
    );
    assert_eq!(
        header(&req, "anthropic-beta"),
        Some("server-side-fallback-2026-07-01")
    );
    assert_eq!(
        body["fallbacks"],
        json!([{"model": "claude-opus-4-8"}, {"model": "claude-opus-5"}])
    );
    assert_eq!(body.get("thinking"), None);
    let (_, body) = send(
        "claude-sonnet-4-6",
        &Auth::ApiKey("k".into()),
        Some("off"),
        false,
    );
    assert_eq!(body["thinking"], json!({"type": "disabled"}));
}

#[test]
fn models_list_merges_with_table() {
    let page = json!({"data": [
        {"type": "model", "id": "claude-opus-5-5", "display_name": "Claude Opus 5.5", "max_input_tokens": null},
        {"type": "model", "id": "claude-new-1", "display_name": "New", "max_input_tokens": 300000,
         "max_tokens": 50000, "capabilities": {"thinking": {"supported": false}}},
    ], "has_more": false, "last_id": "claude-new-1"});
    let transport = ScriptedTransport::new(vec![ScriptedResponse::new(200, &page.to_string(), 7)]);
    let list = list_models(&transport, &Auth::OAuth("t".into())).unwrap();
    assert_eq!(list.len(), 2);
    assert_eq!(
        (
            list[0].context_window,
            list[0].max_output,
            list[0].reasoning
        ),
        (1_000_000, 128_000, true)
    );
    assert_eq!(
        (
            list[1].context_window,
            list[1].max_output,
            list[1].reasoning
        ),
        (300_000, 50_000, false)
    );
    let sent = transport.requests().remove(0);
    assert_eq!(
        (sent.method.as_str(), sent.url.as_str()),
        ("GET", "https://api.anthropic.com/v1/models?limit=1000")
    );
    assert_eq!(
        header(&sent, "anthropic-beta"),
        Some("claude-code-20250219,oauth-2025-04-20")
    );
    let denied = ScriptedResponse::new(
        401,
        r#"{"type":"error","error":{"type":"authentication_error","message":"x"}}"#,
        3,
    );
    let err = list_models(
        &ScriptedTransport::new(vec![denied]),
        &Auth::ApiKey("k".into()),
    )
    .unwrap_err();
    assert_eq!(err.kind, fast_harness_core::llm::ErrorKind::Auth);
    assert!(builtin_models().iter().any(|m| m.id == "claude-opus-5-5"));
}

/// Run by hand with `ANTHROPIC_API_KEY` set:
/// `cargo test -p fast-harness-core --test anthropic_request -- --ignored`.
#[test]
#[ignore]
fn live_models_list() {
    let Ok(key) = std::env::var("ANTHROPIC_API_KEY") else {
        eprintln!("ANTHROPIC_API_KEY not set: skipped");
        return;
    };
    let transport = fast_harness_core::net::UreqTransport::new();
    let list = list_models(&transport, &Auth::ApiKey(key)).unwrap();
    assert!(!list.is_empty());
    assert!(list.iter().all(|m| m.id.starts_with("claude-")));
}

fn signed_turn(model: &str, call_id: &str) -> Vec<Message> {
    let assistant = Message {
        role: Role::Assistant,
        content: vec![
            Block::Thinking {
                text: "plan".into(),
                signature: Some(format!("sig-{call_id}")),
            },
            Block::ToolCall {
                id: call_id.into(),
                name: "read".into(),
                arguments: "{\"path\":\"a\"}".into(),
                signature: Some("gemini-sig".into()),
            },
        ],
        model: Some(model.into()),
    };
    let result = Message {
        role: Role::Tool,
        content: vec![Block::ToolResult {
            call_id: call_id.into(),
            content: "ok".into(),
            is_error: false,
        }],
        model: None,
    };
    vec![assistant, result]
}

#[test]
fn thinking_goes_back_only_to_anthropic_models() {
    let mut messages = vec![Message::user("hello")];
    messages.extend(signed_turn("antigravity/gemini-x", "g1"));
    messages.push(Message::user("next"));
    messages.extend(signed_turn("claude-opus-5-5", "c1"));
    messages.push(Message::user("again"));
    let transport = ScriptedTransport::new(vec![ScriptedResponse::new(200, DONE, 5)]);
    let cancel = AtomicBool::new(false);
    let req = ChatRequest {
        model: "claude-opus-5-5",
        system: "Be brief.",
        messages: &messages,
        tools: &tools(),
        max_tokens: 4096,
        effort: None,
        cache_long: false,
        cancel: &cancel,
    };
    let auth = Auth::ApiKey("k".into());
    let provider = Anthropic::new("https://example.test/");
    provider
        .stream(&transport, &auth, &req, &mut |_| {})
        .unwrap();
    let recorded = transport.requests().remove(0);
    assert!(!recorded.body.contains("gemini-sig"));
    assert!(!recorded.body.contains("sig-g1"));
    let body: Value = serde_json::from_str(&recorded.body).unwrap();
    let gemini = &body["messages"][1]["content"];
    assert_eq!(gemini.as_array().unwrap().len(), 1);
    assert_eq!(gemini[0]["type"], "tool_use");
    assert!(gemini[0].get("signature").is_none());
    let claude = &body["messages"][4]["content"];
    assert_eq!(
        claude[0],
        json!({"type": "thinking", "thinking": "plan", "signature": "sig-c1"})
    );
    assert_eq!(claude[1]["type"], "tool_use");
}
