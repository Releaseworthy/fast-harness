//! The `openai-chat` adapter (PLAN.md D41) against the scripted transport,
//! with bodies split into tiny chunks.

use std::sync::atomic::AtomicBool;

use fast_harness_core::http::{ScriptedResponse, ScriptedTransport};
use fast_harness_core::llm::openai::OpenAiChat;
use fast_harness_core::llm::{custom, Auth, ChatRequest, Delta, ErrorKind, Provider, ToolSpec};
use fast_harness_core::types::{Block, Message, Role, StopReason};
use serde_json::{json, Value};

fn tools() -> Vec<ToolSpec> {
    ["find_tools", "read", "shell"]
        .iter()
        .map(|name| ToolSpec::new(*name, format!("{name} tool"), json!({"type": "object"})))
        .collect()
}

fn register() {
    let file = r#"{"id": "oc", "api": "openai-chat", "baseUrl": "http://local/v1",
        "models": [{"id": "chat-1", "maxOutput": 4096, "efforts": ["low", "high"]}]}"#;
    custom::remember(&[serde_json::from_str(file).unwrap()]);
}

fn sse(chunks: &[Value]) -> String {
    let mut out: String = chunks.iter().map(|c| format!("data: {c}\n\n")).collect();
    out.push_str("data: [DONE]\n\n");
    out
}

struct Call {
    result: Result<fast_harness_core::llm::Completion, fast_harness_core::llm::LlmError>,
    deltas: Vec<String>,
    body: Value,
    headers: Vec<(String, String)>,
    url: String,
}

fn call(response: ScriptedResponse, messages: &[Message]) -> Call {
    register();
    let transport = ScriptedTransport::new(vec![response]);
    let provider = OpenAiChat::new(
        "oc",
        "http://local/v1/",
        vec![("x-extra".into(), "1".into())],
    );
    let (tools, cancel) = (tools(), AtomicBool::new(false));
    let req = ChatRequest::new("oc/chat-1", "be brief", messages, &tools, 32_000, &cancel)
        .with_effort(Some("high"));
    let mut deltas = Vec::new();
    let auth = Auth::ApiKey("test-key".into());
    let result = provider.stream(&transport, &auth, &req, &mut |delta| {
        deltas.push(match delta {
            Delta::Text(t) => format!("text:{t}"),
            Delta::Thinking(t) => format!("think:{t}"),
            Delta::ToolStart { index, id, name } => format!("start:{index}:{id}:{name}"),
            Delta::ToolArgs { index, chunk } => format!("args:{index}:{chunk}"),
            _ => "other".to_string(),
        })
    });
    let recorded = transport.requests().remove(0);
    Call {
        result,
        deltas,
        body: serde_json::from_str(&recorded.body).unwrap(),
        headers: recorded.headers,
        url: recorded.url,
    }
}

#[test]
fn text_and_usage_stream_in_tiny_chunks() {
    let body = sse(&[
        json!({"choices": [{"index": 0, "delta": {"role": "assistant", "content": "Hel"}}]}),
        json!({"choices": [{"index": 0, "delta": {"content": "lo"}, "finish_reason": "stop"}]}),
        json!({"choices": [], "usage": {"prompt_tokens": 100, "completion_tokens": 5,
            "prompt_tokens_details": {"cached_tokens": 60}}}),
    ]);
    let out = call(ScriptedResponse::new(200, &body, 3), &[Message::user("hi")]);
    let done = out.result.unwrap();
    assert_eq!(done.message.text(), "Hello");
    assert_eq!(done.stop, StopReason::End);
    let u = done.usage;
    assert_eq!(
        (u.input, u.cache_read, u.output, u.cache_write),
        (40, 60, 5, 0)
    );
    assert_eq!(out.url, "http://local/v1/chat/completions");
    assert_eq!(out.body["model"], "chat-1");
    assert_eq!(out.body["stream_options"]["include_usage"], true);
    assert_eq!(out.body["max_tokens"], 4096);
    assert_eq!(out.body["reasoning_effort"], "high");
    assert_eq!(
        out.body["messages"][0],
        json!({"role": "system", "content": "be brief"})
    );
    // Deferred loading: only find_tools before a find result.
    let names: Vec<&str> = (out.body["tools"].as_array().unwrap().iter())
        .map(|t| t["function"]["name"].as_str().unwrap())
        .collect();
    assert_eq!(names, ["find_tools"]);
    let header = |name: &str| {
        out.headers
            .iter()
            .find(|(k, _)| k == name)
            .map(|(_, v)| v.clone())
    };
    assert_eq!(header("authorization").as_deref(), Some("Bearer test-key"));
    assert_eq!(header("x-extra").as_deref(), Some("1"));
    assert_eq!(out.deltas, ["text:Hel", "text:lo"]);
}

fn found_read() -> Vec<Message> {
    let call = Block::ToolCall {
        id: "f1".into(),
        name: "find_tools".into(),
        arguments: r#"{"names":["read"]}"#.into(),
        signature: None,
    };
    let result = Block::ToolResult {
        call_id: "f1".into(),
        content: "Loaded: read".into(),
        is_error: false,
    };
    vec![
        Message::user("read a"),
        Message::new(Role::Assistant, vec![call]),
        Message::new(Role::Tool, vec![result]),
    ]
}

#[test]
fn tool_calls_stream_their_arguments_and_found_tools_join() {
    let tool = |args: Value| json!({"choices": [{"index": 0, "delta": {"tool_calls": [args]}}]});
    let body = sse(&[
        json!({"choices": [{"index": 0, "delta": {"reasoning_content": "think"}}]}),
        tool(json!({"index": 0, "id": "call_9", "type": "function",
            "function": {"name": "read", "arguments": "{\"pa"}})),
        tool(json!({"index": 0, "function": {"arguments": "th\":\"a\"}"}})),
        json!({"choices": [{"index": 0, "delta": {}, "finish_reason": "tool_calls"}]}),
    ]);
    let out = call(ScriptedResponse::new(200, &body, 7), &found_read());
    let done = out.result.unwrap();
    assert_eq!(done.stop, StopReason::ToolUse);
    let calls: Vec<_> = done.message.tool_calls().collect();
    assert_eq!(calls, [("call_9", "read", r#"{"path":"a"}"#)]);
    assert!(matches!(&done.message.content[0], Block::Thinking { text, .. } if text == "think"));
    assert_eq!(out.deltas[1], "start:0:call_9:read");
    let names: Vec<&str> = (out.body["tools"].as_array().unwrap().iter())
        .map(|t| t["function"]["name"].as_str().unwrap())
        .collect();
    assert_eq!(names, ["find_tools", "read"]);
    let messages = out.body["messages"].as_array().unwrap();
    assert_eq!(
        messages[2]["tool_calls"][0]["function"]["name"],
        "find_tools"
    );
    assert_eq!(
        messages[3],
        json!({"role": "tool", "tool_call_id": "f1", "content": "Loaded: read"})
    );
}

#[test]
fn errors_map_to_their_kinds() {
    let overflow = r#"{"error": {"message": "This model's maximum context length is 8192 tokens",
        "type": "invalid_request_error", "code": "context_length_exceeded"}}"#;
    let cases = [
        (400, overflow, ErrorKind::ContextOverflow),
        (401, r#"{"error": {"message": "bad key"}}"#, ErrorKind::Auth),
        (
            429,
            r#"{"error": {"message": "slow down"}}"#,
            ErrorKind::RateLimit,
        ),
        (500, "oops", ErrorKind::Server),
        (
            400,
            r#"{"error": {"message": "bad field"}}"#,
            ErrorKind::BadRequest,
        ),
    ];
    for (status, body, kind) in cases {
        let out = call(
            ScriptedResponse::new(status, body, 5),
            &[Message::user("hi")],
        );
        let err = out.result.unwrap_err();
        assert_eq!((err.kind, err.status), (kind, Some(status)), "{err}");
    }
    let event = "data: {\"error\": {\"message\": \"upstream failed\"}}\n\n";
    let err = call(ScriptedResponse::new(200, event, 4), &[Message::user("hi")])
        .result
        .unwrap_err();
    assert_eq!(err.kind, ErrorKind::Server);
    let cut = "data: {\"choices\": [{\"index\": 0, \"delta\": {\"content\": \"x\"}}]}\n\n";
    let err = call(ScriptedResponse::new(200, cut, 4), &[Message::user("hi")])
        .result
        .unwrap_err();
    assert_eq!(err.kind, ErrorKind::Protocol);
}
