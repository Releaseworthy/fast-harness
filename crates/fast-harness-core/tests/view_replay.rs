use fast_harness_core::llm::Timing;
use fast_harness_core::trace::TraceEvent;
use fast_harness_core::types::{Block, Message, Role, StopReason, Usage};
use fast_harness_core::view::{Item, NoteKind, State, Transcript};
use serde_json::json;

fn assistant() -> Message {
    Message {
        role: Role::Assistant,
        content: vec![
            Block::Thinking {
                text: "hmm".into(),
                signature: Some("sig".into()),
            },
            Block::Text {
                text: "Look".into(),
            },
            Block::ToolCall {
                id: "c1".into(),
                name: "glob".into(),
                arguments: r#"{"pattern":"*.rs"}"#.into(),
                signature: None,
            },
        ],
        model: None,
    }
}

fn results() -> Message {
    Message {
        role: Role::Tool,
        content: vec![Block::ToolResult {
            call_id: "c1".into(),
            content: "a.rs".into(),
            is_error: false,
        }],
        model: None,
    }
}

fn final_answer() -> Message {
    Message {
        role: Role::Assistant,
        content: vec![Block::Text {
            text: "Done".into(),
        }],
        model: None,
    }
}

fn items(duration_ms: Option<u64>) -> Vec<Item> {
    vec![
        Item::User { text: "hi".into() },
        Item::Assistant {
            turn: 1,
            step: 1,
            text: "Look".into(),
            thinking: "hmm".into(),
        },
        Item::Tool {
            id: "c1".into(),
            name: "glob".into(),
            arguments: r#"{"pattern":"*.rs"}"#.into(),
            summary: "*.rs".into(),
            output: Some("a.rs".into()),
            is_error: false,
            duration_ms,
        },
        Item::Assistant {
            turn: 1,
            step: 2,
            text: "Done".into(),
            thinking: String::new(),
        },
    ]
}

#[test]
fn from_messages_joins_calls_and_results() {
    let messages = [Message::user("hi"), assistant(), results(), final_answer()];
    let view = Transcript::from_messages(&messages);
    assert_eq!(view.items, items(None));
    assert_eq!(view.state, State::Idle);
    assert_eq!((view.turn, view.step), (1, 2));
    assert!(view.revision > 0);
}

fn event(seq: u64, kind: &str, data: serde_json::Value) -> TraceEvent {
    TraceEvent {
        kind: kind.into(),
        seq,
        time: 1000 + seq,
        data,
    }
}

fn usage() -> Usage {
    Usage {
        input: 4,
        output: 9,
        cache_read: 100,
        cache_write: 20,
        cache_write_1h: 0,
    }
}

fn log() -> Vec<TraceEvent> {
    let timing = json!({"ttfbMs":5,"firstTokenMs":6,"totalMs":70,"chunks":3,"bytes":99});
    vec![
        event(0, "turn/start", json!({"turn":1})),
        event(
            1,
            "user/message",
            json!({"turn":1,"message":Message::user("hi")}),
        ),
        event(2, "step/start", json!({"turn":1,"step":1})),
        event(
            3,
            "request/context",
            json!({"turn":1,"step":1,"attempt":1,
            "provider":"anthropic","model":"m1","account":"a1","effort":null,"wire":"t1-s1-a1"}),
        ),
        event(
            4,
            "llm/retry",
            json!({"turn":1,"step":1,"attempt":1,"delayMs":2000,"error":"Overloaded"}),
        ),
        event(
            5,
            "account/switch",
            json!({"from":"a1","to":"a2","reason":"RateLimit","holdUntil":null}),
        ),
        event(
            6,
            "assistant/message",
            json!({"turn":1,"step":1,"message":assistant(),
            "usage":usage(),"stop":"tool_use","timing":timing,"account":"a2"}),
        ),
        event(
            7,
            "tool/call",
            json!({"turn":1,"step":1,"callId":"c1","name":"glob","arguments":"{}"}),
        ),
        event(
            8,
            "tool/result",
            json!({"turn":1,"step":1,"message":results(),"durationMs":12}),
        ),
        event(9, "step/end", json!({"turn":1,"step":1,"durationMs":90})),
        event(10, "step/start", json!({"turn":1,"step":2})),
        event(
            11,
            "assistant/message",
            json!({"turn":1,"step":2,"message":final_answer(),
            "usage":usage(),"stop":"end","timing":timing,"account":"a2"}),
        ),
        event(12, "step/end", json!({"turn":1,"step":2,"durationMs":10})),
        event(
            13,
            "turn/end",
            json!({"turn":1,"steps":2,"usage":{},"durationMs":200,"outcome":"cancelled"}),
        ),
        event(14, "future/kind", json!(["ignored"])),
    ]
}

fn footer(step: u32, stop: StopReason) -> Item {
    Item::StepFooter {
        turn: 1,
        step,
        model: "m1".into(),
        account: "a2".into(),
        usage: usage(),
        timing: Timing {
            ttfb_ms: 5,
            first_token_ms: 6,
            total_ms: 70,
            chunks: 3,
            bytes: 99,
        },
        stop,
    }
}

#[test]
fn from_trace_adds_footers_notes_and_durations() {
    let view = Transcript::from_trace(&log());
    let mut want = items(Some(12));
    want.insert(
        1,
        Item::Note {
            kind: NoteKind::Retry,
            text: "Attempt 1 failed, retrying in 2000 ms: Overloaded".into(),
        },
    );
    want.insert(
        2,
        Item::Note {
            kind: NoteKind::AccountSwitch,
            text: "Account a1 -> a2 (RateLimit)".into(),
        },
    );
    want.insert(5, footer(1, StopReason::ToolUse));
    want.push(footer(2, StopReason::End));
    want.push(Item::Note {
        kind: NoteKind::Info,
        text: "Turn ended: cancelled".into(),
    });
    assert_eq!(view.items, want);
    let mut totals = usage();
    totals.add(&usage());
    assert_eq!(view.totals, totals);
}

#[test]
fn from_trace_survives_malformed_data() {
    let events = vec![
        event(0, "user/message", json!("nope")),
        event(
            1,
            "assistant/message",
            json!({"turn":"x","message":7,"usage":[1]}),
        ),
        event(2, "tool/result", json!({"message":{"role":"tool"}})),
        event(3, "llm/retry", json!(null)),
        event(4, "turn/end", json!({"outcome":5})),
    ];
    let view = Transcript::from_trace(&events);
    assert_eq!(view.items.len(), 2);
    assert!(matches!(
        &view.items[0],
        Item::Note {
            kind: NoteKind::Retry,
            ..
        }
    ));
    assert!(matches!(
        &view.items[1],
        Item::StepFooter {
            stop: StopReason::Other,
            turn: 0,
            ..
        }
    ));
}
