use fast_harness_core::events::{AgentEvent, TurnOutcome};
use fast_harness_core::llm::Timing;
use fast_harness_core::types::{StopReason, Usage};
use fast_harness_core::view::{Item, NoteKind, State, Transcript};

fn usage(input: u64, output: u64) -> Usage {
    Usage {
        input,
        output,
        cache_read: 0,
        cache_write: 0,
        cache_write_1h: 0,
    }
}

fn timing(total_ms: u64) -> Timing {
    Timing {
        total_ms,
        ..Timing::default()
    }
}

fn tool(id: &str, name: &str, args: &str, summary: &str, out: &str, err: bool, ms: u64) -> Item {
    Item::Tool {
        id: id.into(),
        name: name.into(),
        arguments: args.into(),
        summary: summary.into(),
        output: Some(out.into()),
        is_error: err,
        duration_ms: Some(ms),
    }
}

fn request(account: &str, attempt: u32) -> AgentEvent {
    AgentEvent::Request {
        model: "m1".into(),
        account: account.into(),
        attempt,
    }
}

fn step_one() -> Vec<AgentEvent> {
    let read_args = r#"{"path":"a.rs"}"#;
    let grep_args = r#"{"pattern":"fn","path":"src"}"#;
    vec![
        AgentEvent::TurnStart { turn: 1 },
        AgentEvent::StepStart { turn: 1, step: 1 },
        request("a1", 1),
        AgentEvent::Retry {
            attempt: 1,
            delay_ms: 2000,
            error: "Overloaded".into(),
        },
        AgentEvent::AccountSwitch {
            from: "a1".into(),
            to: "a2".into(),
            reason: "RateLimit".into(),
        },
        request("a2", 2),
        AgentEvent::ThinkingDelta("plan".into()),
        AgentEvent::ThinkingDelta(" more".into()),
        AgentEvent::TextDelta("Reading".into()),
        AgentEvent::ToolCallStart {
            id: "c1".into(),
            name: "read".into(),
        },
        AgentEvent::ToolArgsDelta {
            id: "c1".into(),
            chunk: r#"{"path":"#.into(),
        },
        AgentEvent::ToolArgsDelta {
            id: "c1".into(),
            chunk: r#""a.rs"}"#.into(),
        },
        AgentEvent::ToolCallStart {
            id: "c2".into(),
            name: "grep".into(),
        },
        AgentEvent::ToolCall {
            id: "c1".into(),
            name: "read".into(),
            arguments: read_args.into(),
        },
        AgentEvent::ToolCall {
            id: "c2".into(),
            name: "grep".into(),
            arguments: grep_args.into(),
        },
        AgentEvent::ToolResult {
            id: "c2".into(),
            content: "src/x.rs:1".into(),
            is_error: false,
            duration_ms: 7,
        },
        AgentEvent::ToolResult {
            id: "c1".into(),
            content: "missing".into(),
            is_error: true,
            duration_ms: 3,
        },
        AgentEvent::StepEnd {
            turn: 1,
            step: 1,
            usage: usage(10, 20),
            context: 30,
            window: 1_000_000,
            timing: timing(500),
            stop: StopReason::ToolUse,
        },
    ]
}

fn step_two() -> Vec<AgentEvent> {
    vec![
        AgentEvent::StepStart { turn: 1, step: 2 },
        request("a2", 1),
        AgentEvent::TextDelta("Do".into()),
        AgentEvent::TextDelta("ne".into()),
        AgentEvent::StepEnd {
            turn: 1,
            step: 2,
            usage: usage(1, 2),
            context: 410_000,
            window: 1_000_000,
            timing: timing(90),
            stop: StopReason::End,
        },
        AgentEvent::TurnEnd {
            turn: 1,
            usage: usage(11, 22),
            duration_ms: 700,
            outcome: TurnOutcome::Done,
        },
    ]
}

fn footer(step: u32, usage: Usage, timing: Timing, stop: StopReason) -> Item {
    Item::StepFooter {
        turn: 1,
        step,
        model: "m1".into(),
        account: "a2".into(),
        usage,
        timing,
        stop,
    }
}

fn expected() -> Vec<Item> {
    vec![
        Item::User {
            text: "fix it".into(),
        },
        Item::Note {
            kind: NoteKind::Retry,
            text: "Attempt 1 failed, retrying in 2000 ms: Overloaded".into(),
        },
        Item::Note {
            kind: NoteKind::AccountSwitch,
            text: "Account a1 -> a2 (RateLimit)".into(),
        },
        Item::Assistant {
            turn: 1,
            step: 1,
            text: "Reading".into(),
            thinking: "plan more".into(),
        },
        tool(
            "c1",
            "read",
            r#"{"path":"a.rs"}"#,
            "a.rs",
            "missing",
            true,
            3,
        ),
        tool(
            "c2",
            "grep",
            r#"{"pattern":"fn","path":"src"}"#,
            "fn in src",
            "src/x.rs:1",
            false,
            7,
        ),
        footer(1, usage(10, 20), timing(500), StopReason::ToolUse),
        Item::Assistant {
            turn: 1,
            step: 2,
            text: "Done".into(),
            thinking: String::new(),
        },
        footer(2, usage(1, 2), timing(90), StopReason::End),
    ]
}

#[test]
fn scripted_turn_produces_exact_items_and_revisions() {
    let mut view = Transcript::new();
    view.push_user("fix it");
    assert_eq!(view.revision, 1);
    let events: Vec<AgentEvent> = step_one().into_iter().chain(step_two()).collect();
    for event in &events {
        let before = view.revision;
        view.apply_at(event, 1_000);
        assert_ne!(view.revision, before, "{event:?}");
    }
    assert_eq!(view.items, expected());
    assert_eq!(view.state, State::Idle);
    assert_eq!(view.totals, usage(11, 22));
    assert_eq!((view.turn, view.step), (1, 2));
    assert_eq!(view.account, "a2");
    // The last step's context against its window; no cache reads this turn.
    let status = view.context_status();
    assert_eq!(status.as_deref(), Some("context 41% of 1M · cache 0%"));
    view.turn_usage.cache_read = 33;
    let status = view.context_status();
    assert_eq!(status.as_deref(), Some("context 41% of 1M · cache 75%"));
    assert_eq!(Transcript::new().context_status(), None);
    assert_eq!(
        [1_048_576, 1_500_000, 200_000, 999].map(fast_harness_core::view::short_tokens),
        ["1M", "1.5M", "200K", "999"]
    );
}

#[test]
fn states_follow_the_stream() {
    let mut view = Transcript::new();
    let mut states = Vec::new();
    for event in step_one() {
        view.apply_at(&event, 1_000);
        states.push(view.state);
    }
    assert_eq!(states[2], State::Waiting);
    assert_eq!(states[3], State::Retrying { until_ms: 3_000 });
    assert_eq!(states[6], State::Thinking);
    assert_eq!(states[8], State::Responding);
    assert_eq!(states[13], State::RunningTools { running: 2 });
    assert_eq!(states[15], State::RunningTools { running: 1 });
    assert_eq!(states[16], State::Waiting);
}

#[test]
fn deltas_accumulate_and_args_stream() {
    let mut view = Transcript::new();
    view.apply(&AgentEvent::StepStart { turn: 2, step: 3 });
    for chunk in ["a", "b", "c"] {
        view.apply(&AgentEvent::TextDelta(chunk.into()));
    }
    view.apply(&AgentEvent::ToolCallStart {
        id: "t".into(),
        name: "shell".into(),
    });
    view.apply(&AgentEvent::ToolArgsDelta {
        id: "t".into(),
        chunk: "{\"comm".into(),
    });
    view.apply(&AgentEvent::TextDelta("d".into()));
    assert_eq!(view.items.len(), 3);
    assert!(matches!(&view.items[0],
        Item::Assistant { turn: 2, step: 3, text, .. } if text == "abc"));
    assert!(matches!(&view.items[1],
        Item::Tool { arguments, output: None, .. } if arguments == "{\"comm"));
    assert!(matches!(&view.items[2], Item::Assistant { text, .. } if text == "d"));
}

#[test]
fn notes_for_errors_subagents_and_unfinished_turns() {
    let mut view = Transcript::new();
    view.apply(&AgentEvent::Error("boom".into()));
    view.apply(&AgentEvent::UsageUpdate {
        account: "a".into(),
    });
    view.apply(&AgentEvent::Subagent {
        session: "s9".into(),
        running: true,
    });
    view.apply(&AgentEvent::TurnEnd {
        turn: 1,
        usage: Usage::default(),
        duration_ms: 1,
        outcome: TurnOutcome::Cancelled,
    });
    let notes: Vec<(NoteKind, &str)> = view
        .items
        .iter()
        .filter_map(|item| match item {
            Item::Note { kind, text } => Some((*kind, text.as_str())),
            _ => None,
        })
        .collect();
    assert_eq!(
        notes,
        vec![
            (NoteKind::Error, "boom"),
            (NoteKind::Info, "Subagent s9 started"),
            (NoteKind::Info, "Turn ended: cancelled"),
        ]
    );
    assert_eq!(view.revision, 4);
}
