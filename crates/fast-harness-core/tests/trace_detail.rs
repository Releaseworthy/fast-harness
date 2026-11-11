//! The readable trace timeline: `trace::detail::entries` and `format_text`.

use fast_harness_core::llm::{Timing, ToolSpec};
use fast_harness_core::trace::detail::{entries, format_text, Entry, EntryKind};
use fast_harness_core::trace::{self, RequestContext, SessionHeader, TraceEvent, TraceWriter};
use fast_harness_core::types::{Block, Message, Role, StopReason, Usage};
use serde_json::{json, Value};
use std::path::{Path, PathBuf};

struct TempDir(PathBuf);

impl TempDir {
    fn new(name: &str) -> TempDir {
        let nanos = trace::now_ms();
        let path = std::env::temp_dir().join(format!(
            "fast-harness-test-{name}-{}-{nanos}",
            std::process::id()
        ));
        let _ = std::fs::remove_dir_all(&path);
        TempDir(path)
    }
}

impl Drop for TempDir {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

fn ev(seq: u64, kind: &str, data: Value) -> TraceEvent {
    TraceEvent {
        kind: kind.into(),
        seq,
        time: 0,
        data,
    }
}

fn spec(name: &str) -> ToolSpec {
    ToolSpec {
        name: name.into(),
        description: format!("{name} tool"),
        schema: json!({"type": "object"}),
    }
}

fn context<'a>(
    step: u32,
    attempt: u32,
    effort: Option<&'a str>,
    wire: Option<&'a str>,
) -> RequestContext<'a> {
    RequestContext {
        turn: 1,
        step,
        attempt,
        provider: "anthropic",
        model: "claude-opus-5-5",
        account: "acct-a",
        effort,
        wire,
    }
}

fn usage(input: u64, output: u64, read: u64, write: u64) -> Usage {
    Usage {
        input,
        output,
        cache_read: read,
        cache_write: write,
    }
}

fn timing(ttfb_ms: u64, total_ms: u64) -> Timing {
    Timing {
        ttfb_ms,
        total_ms,
        ..Timing::default()
    }
}

fn thinking_answer() -> Message {
    Message {
        role: Role::Assistant,
        content: vec![
            Block::Thinking {
                text: "I should read a.txt".into(),
                signature: Some("sig".into()),
            },
            Block::RedactedThinking { data: "xx".into() },
            Block::Text {
                text: String::new(),
            },
            Block::Text {
                text: "Reading it.".into(),
            },
            Block::ToolCall {
                id: "c1".into(),
                name: "read".into(),
                arguments: r#"{"path":"a.txt"}"#.into(),
                signature: None,
            },
        ],
        model: None,
    }
}

fn tool_results() -> Message {
    Message {
        role: Role::Tool,
        content: vec![
            Block::ToolResult {
                call_id: "c1".into(),
                content: "line one".into(),
                is_error: false,
            },
            Block::ToolResult {
                call_id: "c7".into(),
                content: "no such call".into(),
                is_error: true,
            },
        ],
        model: None,
    }
}

fn first_step(w: &mut TraceWriter) {
    w.step_start(1, 1).unwrap();
    w.request_header(
        "You are fast-harness.\nBe brief.",
        &[spec("read"), spec("shell")],
    )
    .unwrap();
    w.request_context(&context(1, 1, None, None)).unwrap();
    w.assistant_attempt(1, 1, 1, "Overloaded", &timing(10, 40))
        .unwrap();
    w.llm_retry(1, 1, 1, 2000, "Overloaded").unwrap();
    w.account_switch("acct-a", "acct-b", "RateLimit", None)
        .unwrap();
    w.request_context(&context(1, 2, Some("high"), Some("t1-s1-a2")))
        .unwrap();
    let answer = thinking_answer();
    let stop = StopReason::ToolUse;
    w.assistant_message(
        1,
        1,
        &answer,
        &usage(4, 9, 100, 20),
        stop,
        &timing(812, 2300),
        "acct-b",
    )
    .unwrap();
    w.tool_call(1, 1, "c1", "read", r#"{"path":"a.txt"}"#)
        .unwrap();
    let results = tool_results();
    w.tool_result(1, 1, &results, 3).unwrap();
    w.step_end(1, 1, 2400).unwrap();
}

fn write_turn(path: &Path) -> Vec<TraceEvent> {
    let header = SessionHeader::new("d", "F:\\work", None);
    let mut w = TraceWriter::create(path, &header).unwrap();
    w.turn_start(1).unwrap();
    w.user_message(1, &Message::user("what is in a.txt?"))
        .unwrap();
    first_step(&mut w);
    w.step_start(1, 2).unwrap();
    w.request_context(&context(2, 1, Some("high"), None))
        .unwrap();
    let done = Message {
        role: Role::Assistant,
        content: vec![Block::Text {
            text: "It says line one.".into(),
        }],
        model: None,
    };
    w.assistant_message(
        1,
        2,
        &done,
        &usage(1, 5, 120, 0),
        StopReason::End,
        &timing(300, 900),
        "acct-b",
    )
    .unwrap();
    w.step_end(1, 2, 950).unwrap();
    w.turn_end(1, 2, &usage(5, 14, 220, 20), 3400, "done")
        .unwrap();
    drop(w);
    trace::read(path).unwrap().1
}

const TITLES: [(EntryKind, u32, &str); 16] = [
    (EntryKind::Turn, 0, "Turn 1"),
    (EntryKind::User, 0, "User"),
    (EntryKind::Header, 1, "Request header · system prompt 31 chars · 2 tools: read, shell"),
    (EntryKind::Request, 1, "Step 1 request · claude-opus-5-5 · effort default · account acct-a · attempt 1"),
    (EntryKind::Failure, 1, "Attempt 1 failed after 40 ms"),
    (EntryKind::Retry, 1, "Retry 1 in 2000 ms"),
    (EntryKind::Switch, 1, "Account switch acct-a → acct-b · RateLimit"),
    (EntryKind::Request, 1, "Step 1 request · claude-opus-5-5 · effort high · account acct-a · attempt 2 · wire t1-s1-a2"),
    (EntryKind::Thinking, 1, "Thinking"),
    (EntryKind::Thinking, 1, "Thinking (redacted)"),
    (EntryKind::Response, 1, "Response"),
    (EntryKind::ToolCall, 1, "Tool call read"),
    (EntryKind::Answer, 1, "Step 1 answer · stop tool_use · TTFB 812 ms · total 2300 ms · in 4 · out 9 · cache read 100 · cache write 20"),
    (EntryKind::ToolResult, 1, "Tool result read · 3 ms"),
    (EntryKind::ToolResult, 1, "Tool error c7 · 3 ms"),
    (EntryKind::Request, 2, "Step 2 request · claude-opus-5-5 · effort high · account acct-a · attempt 1"),
];

#[test]
fn a_real_turn_reads_as_a_timeline() {
    let tmp = TempDir::new("detail");
    let events = write_turn(&tmp.0.join("s.jsonl"));
    let list = entries(&events);
    assert_eq!(list.len(), TITLES.len() + 3);
    for (entry, (kind, step, title)) in list.iter().zip(TITLES) {
        assert_eq!(
            (entry.kind, entry.turn, entry.step),
            (kind, 1, step),
            "{title}"
        );
        assert_eq!(entry.title, title);
    }
    assert_eq!(list[1].body, "what is in a.txt?");
    assert_eq!(list[2].body, "You are fast-harness.\nBe brief.");
    assert_eq!(
        (list[3].body.as_str(), list[4].body.as_str()),
        ("", "Overloaded")
    );
    assert_eq!(list[5].body, "Overloaded");
    assert_eq!(
        (list[8].body.as_str(), list[9].body.as_str()),
        ("I should read a.txt", "")
    );
    assert_eq!(list[10].body, "Reading it.");
    assert_eq!(list[11].body, "{\n  \"path\": \"a.txt\"\n}");
    assert_eq!(
        (list[13].body.as_str(), list[14].body.as_str()),
        ("line one", "no such call")
    );
    let errors: Vec<usize> = (0..list.len()).filter(|&i| list[i].is_error).collect();
    assert_eq!(errors, vec![4, 14]);
    let tail: Vec<(EntryKind, u32, &str)> = list[16..]
        .iter()
        .map(|e| (e.kind, e.step, e.title.as_str()))
        .collect();
    assert_eq!(
        tail,
        vec![
            (EntryKind::Response, 2, "Response"),
            (EntryKind::Answer, 2, "Step 2 answer · stop end · TTFB 300 ms · total 900 ms · in 1 · out 5 · cache read 120 · cache write 0"),
            (EntryKind::TurnEnd, 0, "Turn 1 done · 2 steps · 3400 ms · in 5 · out 14 · cache read 220 · cache write 20"),
        ]
    );
    assert_eq!(list[16].body, "It says line one.");
}

#[test]
fn tool_call_without_an_assistant_block_and_one_step() {
    let result = json!({"role": "tool", "content": [
        {"type": "tool_result", "callId": "c9", "content": "ok"}]});
    let events = [
        ev(0, "turn/start", json!({"turn": 3})),
        ev(1, "step/start", json!({"turn": 3, "step": 1})),
        ev(
            2,
            "tool/call",
            json!({"turn": 3, "step": 1, "callId": "c9", "name": "shell", "arguments": "not json"}),
        ),
        ev(
            3,
            "tool/result",
            json!({"turn": 3, "step": 1, "message": result, "durationMs": 7}),
        ),
        ev(
            4,
            "step/end",
            json!({"turn": 3, "step": 1, "durationMs": 9}),
        ),
        ev(5, "something/new", json!({"x": 1})),
        ev(
            6,
            "turn/end",
            json!({"turn": 3, "steps": 1, "outcome": "error"}),
        ),
    ];
    let list = entries(&events);
    let titles: Vec<&str> = list.iter().map(|e| e.title.as_str()).collect();
    assert_eq!(
        titles,
        vec![
            "Turn 3",
            "Tool call shell",
            "Tool result shell · 7 ms",
            "Turn 3 error · 1 step · 0 ms · in 0 · out 0 · cache read 0 · cache write 0",
        ]
    );
    assert_eq!((list[1].step, list[1].body.as_str()), (1, "not json"));
    assert!(!list[2].is_error);
}

#[test]
fn missing_keys_give_defaults() {
    let kinds = [
        "turn/start",
        "user/message",
        "step/start",
        "request/header",
        "request/context",
        "assistant/attempt",
        "llm/retry",
        "account/switch",
        "assistant/message",
        "tool/call",
        "tool/result",
        "step/end",
        "turn/end",
    ];
    let events: Vec<TraceEvent> = (0..kinds.len())
        .map(|i| {
            ev(
                i as u64,
                kinds[i],
                if i % 2 == 0 { json!({}) } else { json!(null) },
            )
        })
        .collect();
    let list = entries(&events);
    let titles: Vec<&str> = list.iter().map(|e| e.title.as_str()).collect();
    assert_eq!(
        titles,
        vec![
            "Turn 0",
            "User",
            "Request header · system prompt 0 chars · 0 tools: ",
            "Step 0 request ·  · effort default · account  · attempt 0",
            "Attempt 0 failed after 0 ms",
            "Retry 0 in 0 ms",
            "Account switch  →  · ",
            "Step 0 answer · stop other · TTFB 0 ms · total 0 ms · in 0 · out 0 · cache read 0 · cache write 0",
            "Tool call ",
            "Turn 0  · 0 steps · 0 ms · in 0 · out 0 · cache read 0 · cache write 0",
        ]
    );
}

fn entry(kind: EntryKind, title: &str, body: &str) -> Entry {
    Entry {
        kind,
        turn: 1,
        step: 0,
        title: title.into(),
        body: body.into(),
        is_error: false,
    }
}

#[test]
fn format_text_indents_bodies_and_separates_turns() {
    let list = [
        entry(EntryKind::Turn, "Turn 1", ""),
        entry(EntryKind::User, "User", "hello\n\nthere"),
        entry(EntryKind::TurnEnd, "Turn 1 done", ""),
        entry(EntryKind::Turn, "Turn 2", ""),
        entry(EntryKind::Response, "Response", "ok"),
    ];
    assert_eq!(
        format_text(&list),
        "Turn 1\nUser\n    hello\n\n    there\nTurn 1 done\n\nTurn 2\nResponse\n    ok\n"
    );
    assert_eq!(format_text(&[]), "");
}
