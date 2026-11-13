//! Typed records write the bytes the untyped writer wrote (PLAN.md D61):
//! every event type through `TraceWriter`'s typed methods and through the
//! data the functions at 25a8652 built (kept here as `old`), compared line
//! by line with `time` masked; unknown lines round-trip unchanged; and every
//! log under `FAST_HARNESS_ROUNDTRIP_DIR` (unset: skipped) reads and writes
//! back byte for byte.

use fast_harness_core::llm::{Timing, ToolSpec};
use fast_harness_core::trace::record::{self, Record};
use fast_harness_core::trace::{kind, RequestContext, SessionHeader, TraceEvent, TraceWriter};
use fast_harness_core::types::{Block, Message, Role, StopReason, Usage};
use serde_json::{json, Value};
use std::path::{Path, PathBuf};

fn temp(name: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("fh-record-{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

fn writer(path: &Path) -> TraceWriter {
    TraceWriter::create(path, &SessionHeader::new("s", "F:\\w", None)).unwrap()
}

/// The lines after the header with `"time":<n>` replaced by `"time":0`.
fn masked(path: &Path) -> Vec<String> {
    let text = std::fs::read_to_string(path).unwrap();
    let mask = |line: &str| {
        let start = line.find("\"time\":").unwrap() + 7;
        let end = start + line[start..].find(',').unwrap();
        format!("{}0{}", &line[..start], &line[end..])
    };
    text.lines().skip(1).map(mask).collect()
}

struct Fixture {
    message: Message,
    result: Message,
    usage: Usage,
    timing: Timing,
    tool: ToolSpec,
}

fn fixture() -> Fixture {
    let mut message = Message::new(
        Role::Assistant,
        vec![Block::text("hi"), Block::text("there")],
    );
    message.model = Some("claude-opus-5-5".into());
    Fixture {
        message,
        result: Message::new(Role::Tool, vec![Block::tool_result("c1", "out", false)]),
        usage: Usage::new(4, 363, 2, 24114).with_cache_write_1h(7),
        timing: Timing::new(812, 1190, 5100),
        tool: ToolSpec::new("read", "Read a file", json!({"type": "object"})),
    }
}

/// The data of every event as the functions at 25a8652 built it.
mod old {
    use super::*;

    fn v<T: serde::Serialize>(value: &T) -> Value {
        serde_json::to_value(value).unwrap()
    }

    pub fn lines(f: &Fixture) -> Vec<(&'static str, Value)> {
        let tools = json!([{ "name": f.tool.name, "description": f.tool.description, "schema": f.tool.schema }]);
        vec![
            (kind::TURN_START, json!({ "turn": 1 })),
            (
                kind::USER_MESSAGE,
                json!({ "turn": 1, "message": v(&f.message) }),
            ),
            (
                kind::USER_MESSAGE,
                json!({ "turn": 1, "message": v(&f.message), "kind": "steer" }),
            ),
            (kind::TURN_START, json!({ "turn": 2, "resume": true })),
            (kind::STEP_START, json!({ "turn": 1, "step": 1 })),
            (
                kind::REQUEST_HEADER,
                json!({ "system": "sys", "tools": tools }),
            ),
            (
                kind::REQUEST_HEADER,
                json!({ "system": "sys", "tools": tools, "context": {"date": "d"} }),
            ),
            (
                kind::REQUEST_CONTEXT,
                json!({ "turn": 1, "step": 1, "attempt": 1, "provider": "anthropic",
                "model": "m", "account": "a", "effort": "high", "wire": "t1-s1-a1" }),
            ),
            (
                kind::REQUEST_CONTEXT,
                json!({ "turn": 1, "step": 1, "attempt": 2, "provider": "anthropic",
                "model": "m", "account": "a", "effort": null }),
            ),
            (
                kind::ASSISTANT_ATTEMPT,
                json!({ "turn": 1, "step": 1, "attempt": 1, "error": "e", "timing": v(&f.timing) }),
            ),
            (
                kind::LLM_RETRY,
                json!({ "turn": 1, "step": 1, "attempt": 1, "delayMs": 2000, "error": "e" }),
            ),
            (
                kind::ACCOUNT_SWITCH,
                json!({ "from": "a", "to": "b", "reason": "RateLimit", "holdUntil": 9 }),
            ),
            (
                kind::ACCOUNT_SWITCH,
                json!({ "from": "a", "to": "b", "reason": "Manual", "holdUntil": null }),
            ),
            (
                kind::ASSISTANT_MESSAGE,
                json!({ "turn": 1, "step": 1, "message": v(&f.message), "usage": v(&f.usage),
                "context": f.usage.context(), "window": 1000000, "stop": "tool_use", "timing": v(&f.timing), "account": "a" }),
            ),
            (
                kind::REQUEST_TRANSFORMATIONS,
                json!({ "turn": 1, "step": 1,
                "transformations": [{"type": "prefix_binding_mismatch"}], "prefixBindingMismatch": 1 }),
            ),
            (
                kind::TOOL_CALL,
                json!({ "turn": 1, "step": 1, "callId": "c1", "name": "read", "arguments": "{}" }),
            ),
            (
                kind::TOOL_RESULT,
                json!({ "turn": 1, "step": 1, "message": v(&f.result), "durationMs": 3, "bytes": 3 }),
            ),
            (
                kind::TOOL_RESULT,
                json!({ "turn": 1, "step": 1, "message": v(&f.result), "durationMs": 3,
                "bytes": 9000, "spilled": "x.out/c1.txt" }),
            ),
            (
                kind::COMPACTION,
                json!({ "turn": 1, "step": 2, "summary": "s", "facts": "f", "loaded": ["read"],
                "kept": [], "callId": "compact_1_2", "mid": true, "before": 10, "after": 5, "usage": v(&f.usage),
                "auto": true, "model": "m", "instructions": null }),
            ),
            (
                kind::STEP_END,
                json!({ "turn": 1, "step": 1, "durationMs": 5120 }),
            ),
            (
                kind::TURN_END,
                json!({ "turn": 1, "steps": 3, "usage": v(&f.usage), "durationMs": 14200, "outcome": "done" }),
            ),
            (
                kind::HOOK,
                json!({"turn": 1, "event": "PreToolUse", "command": "c", "ms": 4, "decision": "none"}),
            ),
            (
                kind::HOOK_ERROR,
                json!({"turn": 1, "event": "Stop", "command": "c", "ms": 4, "error": "boom"}),
            ),
            (
                kind::TOOL_APPROVAL,
                json!({"turn": 1, "step": 1, "callId": "c1", "decision": "once"}),
            ),
        ]
    }
}

#[path = "trace_record/typed.rs"]
mod typed;

#[test]
fn every_event_type_writes_the_old_bytes() {
    let dir = temp("bytes");
    let f = fixture();
    let (old_path, new_path) = (dir.join("old.jsonl"), dir.join("new.jsonl"));
    let mut w = writer(&old_path);
    for (kind, data) in old::lines(&f) {
        w.emit(kind, data).unwrap();
    }
    typed::write_all(&mut writer(&new_path), &f);
    let (old_lines, new_lines) = (masked(&old_path), masked(&new_path));
    assert_eq!(old_lines.len(), 24);
    for (old, new) in old_lines.iter().zip(&new_lines) {
        assert_eq!(new, old);
    }
    assert_eq!(new_lines.len(), old_lines.len());
    let _ = std::fs::remove_dir_all(dir);
}

#[test]
fn every_written_line_reads_back_typed_and_unknown_lines_stay_raw() {
    let f = fixture();
    for (kind, data) in old::lines(&f) {
        let record = Record::parse(kind, &data);
        assert!(!matches!(record, Record::Raw { .. }), "{kind} is typed");
        assert_eq!(record.data().unwrap(), data, "{kind}");
    }
    let unknown = [
        ("x/new", json!({"a": 1})),
        (
            kind::STEP_END,
            json!({"turn": 1, "step": 1, "durationMs": 2, "extra": true}),
        ),
        (kind::TURN_END, json!({"turn": 1})),
        (kind::STEP_START, json!([2])),
    ];
    for (kind, data) in unknown {
        let record = Record::parse(kind, &data);
        assert!(matches!(record, Record::Raw { .. }), "{kind} {data} is raw");
        assert_eq!((record.kind(), record.data().unwrap()), (kind, data));
    }
    let stop = Record::parse(kind::ASSISTANT_MESSAGE, &old::lines(&f)[13].1);
    let Record::AssistantMessage(message) = stop else {
        panic!("typed")
    };
    assert_eq!(message.stop_reason(), StopReason::ToolUse);
}

#[path = "trace_record/real_logs.rs"]
mod real_logs;

#[allow(dead_code)]
fn _uses(_: record::Compaction, _: RequestContext<'_>, _: TraceEvent) {}
