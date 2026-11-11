//! The Antigravity answer stream: text, thoughts, function calls, finish
//! reasons and usage, over a scripted transport in 3-byte chunks.

mod antigravity_support;

use antigravity_support::*;
use fast_harness_core::http::ScriptedResponse;
use fast_harness_core::llm::antigravity::stream::Assembler;
use fast_harness_core::llm::antigravity::ToolNames;
use fast_harness_core::llm::ErrorKind;
use fast_harness_core::types::{Block, StopReason, Usage};
use serde_json::json;

fn call_block(id: &str, name: &str, args: &str, signature: Option<&str>) -> Block {
    Block::ToolCall {
        id: id.into(),
        name: name.into(),
        arguments: args.into(),
        signature: signature.map(str::to_string),
    }
}

#[test]
fn text_answer() {
    let out = answer(&[
        event(json!([{"text": "Hello "}]), None),
        event(json!([{"text": "wörld"}]), Some("STOP")),
    ]);
    let done = out.ok();
    assert_eq!(out.deltas, [r#"Text("Hello ")"#, r#"Text("wörld")"#]);
    assert_eq!(done.message.text(), "Hello wörld");
    assert_eq!(done.message.model, None);
    assert_eq!(done.stop, StopReason::End);
    assert!(done.timing.chunks > 10 && done.timing.bytes > 0);
}

#[test]
fn thinking_then_text() {
    let out = answer(&[
        event(
            json!([{"text": "Let me ", "thought": true, "thoughtSignature": "sig-a"}]),
            None,
        ),
        event(json!([{"text": "think.", "thought": true}]), None),
        event(json!([{"text": "Done."}]), Some("STOP")),
    ]);
    let done = out.ok();
    assert_eq!(out.deltas[0], r#"Thinking("Let me ")"#);
    assert_eq!(out.deltas[2], r#"Text("Done.")"#);
    let thinking = Block::Thinking {
        text: "Let me think.".into(),
        signature: Some("sig-a".into()),
    };
    let text = Block::Text {
        text: "Done.".into(),
    };
    assert_eq!(done.message.content, [thinking, text]);
}

#[test]
fn function_call_with_signature_and_id() {
    let part = json!({
        "functionCall": {"name": "read", "args": {"path": "a.txt"}, "id": "call-1"},
        "thoughtSignature": "sig-call"
    });
    let out = answer(&[event(json!([part]), Some("STOP"))]);
    let done = out.ok();
    let args = r#"{"path":"a.txt"}"#;
    assert_eq!(
        out.deltas,
        [
            r#"ToolStart { index: 0, id: "call-1", name: "read" }"#.to_string(),
            format!("ToolArgs {{ index: 0, chunk: {args:?} }}"),
        ]
    );
    assert_eq!(
        done.message.content,
        [call_block("call-1", "read", args, Some("sig-call"))]
    );
    assert_eq!(done.stop, StopReason::ToolUse);
}

#[test]
fn parallel_calls() {
    let parts = json!([
        {"text": "Two reads."},
        {"functionCall": {"name": "read", "args": {"path": "a"}, "id": "c1"}, "thoughtSignature": "s1"},
        {"functionCall": {"name": "grep", "args": {}, "id": "c2"}}
    ]);
    let out = answer(&[event(parts, Some("STOP"))]);
    let done = out.ok();
    assert!(out.deltas[3].starts_with("ToolStart { index: 1, id: \"c2\""));
    assert_eq!(done.message.content.len(), 3);
    assert_eq!(
        done.message.content[2],
        call_block("c2", "grep", "{}", None)
    );
    assert_eq!(done.stop, StopReason::ToolUse);
}

#[test]
fn missing_ids_are_generated_and_unique() {
    let parts = json!([
        {"functionCall": {"name": "read", "args": {"path": "a"}}},
        {"functionCall": {"name": "read", "args": {"path": "b"}}}
    ]);
    let out = answer(&[event(parts, Some("STOP"))]);
    let ids: Vec<&str> = out.ok().message.tool_calls().map(|(id, _, _)| id).collect();
    assert_eq!(ids.len(), 2);
    assert!(ids.iter().all(|id| !id.is_empty()));
    assert_ne!(ids[0], ids[1]);
}

#[test]
fn names_map_back_through_tool_names() {
    let mut names = ToolNames::default();
    names.insert("read_file_", "read-file.");
    let mut assembler = Assembler::new(&names);
    let data = event(
        json!([{"functionCall": {"name": "read_file_", "id": "x"}}]),
        Some("OTHER"),
    );
    let mut seen = Vec::new();
    let sent = assembler.handle(&data.to_string(), &mut |d| seen.push(format!("{d:?}")));
    assert_eq!(sent, Ok(true));
    assert_eq!(
        seen[0],
        r#"ToolStart { index: 0, id: "x", name: "read-file." }"#
    );
    let (message, _, stop) = assembler.finish().unwrap();
    assert_eq!(message.content, [call_block("x", "read-file.", "{}", None)]);
    assert_eq!(stop, StopReason::ToolUse);
}

#[test]
fn other_after_a_call_is_tool_use_and_alone_is_other() {
    let call = json!([{"functionCall": {"name": "read", "args": {}, "id": "c"}}]);
    let out = answer(&[event(call, None), event(json!([]), Some("OTHER"))]);
    assert_eq!(out.ok().stop, StopReason::ToolUse);
    let out = answer(&[event(json!([{"text": "hm"}]), Some("OTHER"))]);
    assert_eq!(out.ok().stop, StopReason::Other);
}

#[test]
fn max_tokens_and_safety() {
    let out = answer(&[event(json!([{"text": "cut"}]), Some("MAX_TOKENS"))]);
    assert_eq!(out.ok().stop, StopReason::MaxTokens);
    let out = answer(&[event(json!([{"text": "half"}]), Some("SAFETY"))]);
    assert_eq!(out.ok().stop, StopReason::Refusal);
    // A blocked answer often carries no part at all.
    let out = answer(&[wrap(
        json!({"candidates": [{"finishReason": "RECITATION"}]}),
    )]);
    assert_eq!(out.ok().stop, StopReason::Refusal);
    assert!(out.ok().message.content.is_empty());
    let blocked = wrap(json!({"promptFeedback": {"blockReason": "PROHIBITED_CONTENT"}}));
    assert_eq!(answer(&[blocked]).ok().stop, StopReason::Refusal);
}

#[test]
fn usage_from_the_last_metadata() {
    let first = json!({"promptTokenCount": 1, "candidatesTokenCount": 1});
    let last = json!({
        "promptTokenCount": 1000, "cachedContentTokenCount": 600,
        "candidatesTokenCount": 50, "thoughtsTokenCount": 30,
        "toolUsePromptTokenCount": 5, "totalTokenCount": 1085
    });
    let mut one = event(json!([{"text": "a"}]), None);
    one["response"]["usageMetadata"] = first;
    let mut two = event(json!([{"text": "b"}]), Some("STOP"));
    two["response"]["usageMetadata"] = last;
    let usage = answer(&[one, two]).ok().usage;
    let want = Usage {
        input: 405,
        output: 80,
        cache_read: 600,
        cache_write: 0,
    };
    assert_eq!(usage, want);
}

#[test]
fn array_bodies_and_done_markers() {
    let array = json!([event(json!([{"text": "x"}]), Some("STOP"))]);
    let body = format!("data: {array}\n\ndata: [DONE]\n\n");
    let out = call(vec![manifest(), ScriptedResponse::new(200, &body, 2)]);
    assert_eq!(out.ok().message.text(), "x");
}

#[test]
fn missing_finish_reason_is_a_retryable_protocol_error() {
    let out = answer(&[event(json!([{"text": "partial"}]), None)]);
    let err = out.err();
    assert_eq!(err.kind, ErrorKind::Protocol);
    assert_eq!(err.message, "stream ended without a finish reason");
    assert!(err.retryable());
}

#[test]
fn no_part_at_all_is_a_protocol_error() {
    let out = answer(&[event(json!([]), Some("STOP"))]);
    assert_eq!(out.err().kind, ErrorKind::Protocol);
    let out = answer(&[wrap(json!({"candidates": [{"finishReason": "STOP"}]}))]);
    assert_eq!(out.err().kind, ErrorKind::Protocol);
}

#[test]
fn bad_json_and_error_events() {
    let body = "data: {not json\n\n";
    let out = call(vec![manifest(), ScriptedResponse::new(200, body, 1)]);
    assert_eq!(out.err().kind, ErrorKind::Protocol);
    let error = json!({"error": {"code": 429, "status": "RESOURCE_EXHAUSTED", "message": "slow down",
        "details": [{"@type": "type.googleapis.com/google.rpc.RetryInfo", "retryDelay": "2s"}]}});
    let out = answer(&[event(json!([{"text": "a"}]), None), error]);
    let err = out.err();
    assert_eq!(err.kind, ErrorKind::RateLimit);
    assert_eq!(err.retry_after_ms(), Some(2000));
    assert_eq!(out.deltas, [r#"Text("a")"#]);
}
