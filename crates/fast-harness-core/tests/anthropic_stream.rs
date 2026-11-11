//! Stream assembly and error mapping of the Anthropic provider, replayed
//! through `ScriptedTransport` in 1, 3 and 7 byte chunks.

use std::sync::atomic::{AtomicBool, Ordering};

use fast_harness_core::http::{ScriptedResponse, ScriptedTransport};
use fast_harness_core::llm::anthropic::Anthropic;
use fast_harness_core::llm::{
    Auth, ChatRequest, Completion, ErrorKind, LlmError, Provider, ToolSpec,
};
use fast_harness_core::types::{Block, Message, StopReason, Usage};
use serde_json::{json, Value};

const CHUNKS: [usize; 3] = [1, 3, 7];

fn sse(events: &[Value]) -> String {
    let mut out = String::new();
    for event in events {
        let name = event["type"].as_str().unwrap();
        out.push_str(&format!("event: {name}\ndata: {event}\n\n"));
    }
    out
}

fn start(usage: Value) -> Value {
    json!({"type": "message_start", "message": {"id": "msg_1", "type": "message",
        "role": "assistant", "content": [], "model": "claude-opus-4-7", "usage": usage}})
}

fn block_start(index: u64, block: Value) -> Value {
    json!({"type": "content_block_start", "index": index, "content_block": block})
}

fn delta(index: u64, delta: Value) -> Value {
    json!({"type": "content_block_delta", "index": index, "delta": delta})
}

fn stop_block(index: u64) -> Value {
    json!({"type": "content_block_stop", "index": index})
}

fn finish(reason: &str, usage: Value) -> [Value; 2] {
    [
        json!({"type": "message_delta", "delta": {"stop_reason": reason, "stop_sequence": null}, "usage": usage}),
        json!({"type": "message_stop"}),
    ]
}

#[derive(Debug, PartialEq)]
enum Seen {
    Text(String),
    Thinking(String),
    ToolStart(usize, String, String),
    ToolArgs(usize, String),
}

struct Outcome {
    result: Result<Completion, LlmError>,
    seen: Vec<Seen>,
}

fn call(
    response: ScriptedResponse,
    auth: &Auth,
    tools: &[ToolSpec],
    cancel_after: Option<usize>,
) -> Outcome {
    let transport = ScriptedTransport::new(vec![response]);
    let cancel = AtomicBool::new(false);
    let messages = [Message::user("hi")];
    let req = ChatRequest {
        model: "claude-opus-4-7",
        system: "sys",
        messages: &messages,
        tools,
        max_tokens: 1000,
        effort: None,
        cache_long: false,
        cancel: &cancel,
    };
    let mut seen = Vec::new();
    let result = Anthropic::default().stream(&transport, auth, &req, &mut |d| {
        use fast_harness_core::llm::Delta;
        seen.push(match d {
            Delta::Text(t) => Seen::Text(t.into()),
            Delta::Thinking(t) => Seen::Thinking(t.into()),
            Delta::ToolStart { index, id, name } => Seen::ToolStart(index, id.into(), name.into()),
            Delta::ToolArgs { index, chunk } => Seen::ToolArgs(index, chunk.into()),
        });
        if cancel_after.is_some_and(|n| seen.len() >= n) {
            cancel.store(true, Ordering::Relaxed);
        }
    });
    Outcome { result, seen }
}

fn key() -> Auth {
    Auth::ApiKey("test-key".into())
}

fn text_stream() -> String {
    let mut events = vec![
        start(json!({"input_tokens": 25, "output_tokens": 1,
            "cache_read_input_tokens": 100, "cache_creation_input_tokens": 20})),
        block_start(0, json!({"type": "text", "text": ""})),
        json!({"type": "ping"}),
        delta(0, json!({"type": "text_delta", "text": "Hel"})),
        delta(0, json!({"type": "text_delta", "text": "lo é😀"})),
        stop_block(0),
    ];
    events.extend(finish("end_turn", json!({"output_tokens": 15})));
    sse(&events)
}

#[test]
fn text_only_stream() {
    let body = text_stream();
    for chunk in CHUNKS {
        let response = ScriptedResponse::new(200, &body, chunk).with_header("request-id", "req_1");
        let out = call(response, &key(), &[], None);
        let done = out.result.unwrap();
        assert_eq!(done.message.text(), "Hello é😀");
        assert_eq!(done.message.content.len(), 1);
        assert_eq!(
            out.seen,
            vec![Seen::Text("Hel".into()), Seen::Text("lo é😀".into())]
        );
        let usage = Usage {
            input: 25,
            output: 15,
            cache_read: 100,
            cache_write: 20,
        };
        assert_eq!(done.usage, usage);
        assert_eq!(done.stop, StopReason::End);
        assert_eq!(done.timing.bytes, body.len() as u64);
        assert_eq!(done.timing.chunks as usize, body.len().div_ceil(chunk));
        assert!(done.timing.total_ms >= done.timing.first_token_ms);
        assert_eq!(done.head.header("request-id"), Some("req_1"));
    }
}

fn tool_stream() -> String {
    let mut events = vec![
        start(json!({"input_tokens": 3, "output_tokens": 1})),
        block_start(
            0,
            json!({"type": "thinking", "thinking": "", "signature": ""}),
        ),
        delta(0, json!({"type": "thinking_delta", "thinking": "plan "})),
        delta(0, json!({"type": "thinking_delta", "thinking": "it"})),
        delta(0, json!({"type": "signature_delta", "signature": "sig=="})),
        stop_block(0),
        block_start(1, json!({"type": "text", "text": ""})),
        delta(1, json!({"type": "text_delta", "text": "Reading."})),
        stop_block(1),
        block_start(
            2,
            json!({"type": "tool_use", "id": "toolu_1", "name": "Read", "input": {}}),
        ),
        delta(2, json!({"type": "input_json_delta", "partial_json": ""})),
        delta(
            2,
            json!({"type": "input_json_delta", "partial_json": "{\"path\":"}),
        ),
        delta(
            2,
            json!({"type": "input_json_delta", "partial_json": " \"a.rs\"}"}),
        ),
        stop_block(2),
        block_start(
            3,
            json!({"type": "tool_use", "id": "toolu_2", "name": "Shell", "input": {}}),
        ),
        stop_block(3),
    ];
    events.extend(finish("tool_use", json!({"output_tokens": 40})));
    sse(&events)
}

fn expected_tool_blocks() -> Vec<Block> {
    vec![
        Block::Thinking {
            text: "plan it".into(),
            signature: Some("sig==".into()),
        },
        Block::Text {
            text: "Reading.".into(),
        },
        Block::ToolCall {
            id: "toolu_1".into(),
            name: "read".into(),
            arguments: "{\"path\": \"a.rs\"}".into(),
            signature: None,
        },
        Block::ToolCall {
            id: "toolu_2".into(),
            name: "shell".into(),
            arguments: "{}".into(),
            signature: None,
        },
    ]
}

#[test]
fn thinking_signature_and_tool_use_with_oauth_names() {
    let tools = [
        ToolSpec {
            name: "read".into(),
            description: "r".into(),
            schema: json!({}),
        },
        ToolSpec {
            name: "shell".into(),
            description: "s".into(),
            schema: json!({}),
        },
    ];
    let body = tool_stream();
    for chunk in CHUNKS {
        let auth = Auth::OAuth("test-token".into());
        let out = call(
            ScriptedResponse::new(200, &body, chunk),
            &auth,
            &tools,
            None,
        );
        let done = out.result.unwrap();
        assert_eq!(done.stop, StopReason::ToolUse);
        assert_eq!(done.usage.output, 40);
        let want = expected_tool_blocks();
        assert_eq!(done.message.content, want);
        assert_eq!(
            out.seen,
            vec![
                Seen::Thinking("plan ".into()),
                Seen::Thinking("it".into()),
                Seen::Text("Reading.".into()),
                Seen::ToolStart(0, "toolu_1".into(), "read".into()),
                Seen::ToolArgs(0, "{\"path\":".into()),
                Seen::ToolArgs(0, " \"a.rs\"}".into()),
                Seen::ToolStart(1, "toolu_2".into(), "shell".into()),
            ]
        );
    }
}

#[test]
fn redacted_thinking_and_other_stop_reasons() {
    for (reason, stop) in [
        ("refusal", StopReason::Refusal),
        ("max_tokens", StopReason::MaxTokens),
        ("stop_sequence", StopReason::End),
        ("pause_turn", StopReason::Other),
    ] {
        let mut events = vec![
            start(json!({"input_tokens": 1})),
            block_start(0, json!({"type": "redacted_thinking", "data": "opaque"})),
            stop_block(0),
        ];
        events.extend(finish(reason, json!({"output_tokens": 2})));
        for chunk in CHUNKS {
            let out = call(
                ScriptedResponse::new(200, &sse(&events), chunk),
                &key(),
                &[],
                None,
            );
            let done = out.result.unwrap();
            assert_eq!(
                done.message.content,
                vec![Block::RedactedThinking {
                    data: "opaque".into()
                }]
            );
            assert_eq!(done.stop, stop);
            assert!(out.seen.is_empty());
        }
    }
}

#[test]
fn error_events_map_to_kinds() {
    let cases = [
        ("overloaded_error", ErrorKind::Overloaded),
        ("rate_limit_error", ErrorKind::RateLimit),
        ("api_error", ErrorKind::Server),
        ("invalid_request_error", ErrorKind::Protocol),
    ];
    for (kind, want) in cases {
        let events = [
            start(json!({"input_tokens": 1})),
            json!({"type": "error", "error": {"type": kind, "message": "Overloaded"}}),
        ];
        for chunk in CHUNKS {
            let out = call(
                ScriptedResponse::new(200, &sse(&events), chunk),
                &key(),
                &[],
                None,
            );
            let err = out.result.unwrap_err();
            assert_eq!(err.kind, want);
            assert_eq!(err.message, format!("{kind}: Overloaded"));
            assert_eq!(err.head.unwrap().status, 200);
        }
    }
}

fn http_error(status: u16, kind: &str, chunk: usize) -> LlmError {
    let body = json!({"type": "error", "error": {"type": kind, "message": "nope"}}).to_string();
    let response = ScriptedResponse::new(status, &body, chunk).with_header("retry-after", "7");
    call(response, &key(), &[], None).result.unwrap_err()
}

#[test]
fn http_429_with_retry_after() {
    for chunk in CHUNKS {
        let err = http_error(429, "rate_limit_error", chunk);
        assert_eq!((err.kind, err.status), (ErrorKind::RateLimit, Some(429)));
        assert_eq!(err.message, "rate_limit_error: nope");
        assert_eq!(err.retry_after_ms(), Some(7000));
        assert!(!err.retryable());
    }
}

#[test]
fn http_statuses_map_to_kinds() {
    for chunk in CHUNKS {
        assert_eq!(
            http_error(401, "authentication_error", chunk).kind,
            ErrorKind::Auth
        );
        assert_eq!(
            http_error(403, "permission_error", chunk).kind,
            ErrorKind::Auth
        );
        assert_eq!(
            http_error(529, "overloaded_error", chunk).kind,
            ErrorKind::Overloaded
        );
        assert_eq!(http_error(500, "api_error", chunk).kind, ErrorKind::Server);
        assert_eq!(
            http_error(400, "invalid_request_error", chunk).kind,
            ErrorKind::BadRequest
        );
    }
    let raw = call(
        ScriptedResponse::new(502, "Bad Gateway", 3),
        &key(),
        &[],
        None,
    );
    assert_eq!(raw.result.unwrap_err().message, "Bad Gateway");
}

#[test]
fn truncated_stream_is_protocol_error() {
    let body = text_stream();
    let cut = &body[..body.find("event: message_delta").unwrap()];
    for chunk in CHUNKS {
        let err = call(ScriptedResponse::new(200, cut, chunk), &key(), &[], None)
            .result
            .unwrap_err();
        assert_eq!(err.kind, ErrorKind::Protocol);
        assert!(err.retryable());
    }
    let garbage = "event: message_start\ndata: {not json\n\n";
    let err = call(ScriptedResponse::new(200, garbage, 3), &key(), &[], None)
        .result
        .unwrap_err();
    assert_eq!(err.kind, ErrorKind::Protocol);
}

#[test]
fn cancel_stops_the_stream() {
    let body = text_stream();
    for chunk in CHUNKS {
        let out = call(
            ScriptedResponse::new(200, &body, chunk),
            &key(),
            &[],
            Some(1),
        );
        assert_eq!(out.result.unwrap_err().kind, ErrorKind::Aborted);
        assert_eq!(out.seen.len(), 1);
    }
}

#[test]
fn transport_failure_is_network() {
    let transport = ScriptedTransport::new(vec![]);
    let cancel = AtomicBool::new(false);
    let req = ChatRequest {
        model: "claude-opus-4-7",
        system: "",
        messages: &[],
        tools: &[],
        max_tokens: 10,
        effort: None,
        cache_long: false,
        cancel: &cancel,
    };
    let err = Anthropic::default()
        .stream(&transport, &key(), &req, &mut |_| {})
        .unwrap_err();
    assert_eq!(err.kind, ErrorKind::Network);
    cancel.store(true, Ordering::Relaxed);
    let err = Anthropic::default()
        .stream(&transport, &key(), &req, &mut |_| {})
        .unwrap_err();
    assert_eq!(err.kind, ErrorKind::Aborted);
    assert!(transport.requests().len() == 1);
}
