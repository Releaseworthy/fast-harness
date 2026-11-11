use super::*;
use nh_core::accounts::usage::Usage as AccountUsage;
use nh_core::llm::Timing;
use nh_core::types::{StopReason, Usage};

fn tool(output: Option<&str>, is_error: bool) -> Item {
    Item::Tool {
        id: "t1".into(),
        name: "read".into(),
        arguments: "{}".into(),
        summary: "src/main.rs".into(),
        output: output.map(str::to_string),
        is_error,
        duration_ms: None,
    }
}

#[test]
fn user_and_assistant_rows() {
    let user = msg_row(&Item::User { text: "hi".into() });
    assert_eq!((user.kind.as_str(), user.text.as_str()), ("user", "hi"));
    let item = Item::Assistant {
        turn: 1,
        step: 1,
        text: "**bold**".into(),
        thinking: "hmm".into(),
    };
    let row = msg_row(&item);
    assert_eq!(row.kind, "assistant");
    assert_eq!(row.text, "**bold**");
    assert_eq!(row.thinking, "hmm");
    assert!(!row.expanded);
}

#[test]
fn tool_rows_track_running_and_errors() {
    let running = msg_row(&tool(None, false));
    assert!(running.running && !running.error);
    assert_eq!(
        (running.title.as_str(), running.summary.as_str()),
        ("read", "src/main.rs")
    );
    let failed = msg_row(&tool(Some("boom"), true));
    assert!(!failed.running && failed.error);
    assert_eq!(
        (failed.text.as_str(), failed.output.as_str()),
        ("boom", "boom")
    );
    let long = "x".repeat(OUTPUT_CHARS + 10);
    let row = msg_row(&tool(Some(&long), false));
    assert_eq!(row.text.len(), OUTPUT_CHARS + 10);
    assert!(row.output.contains("10 more characters"));
}

#[test]
fn notes_and_footers() {
    let note = msg_row(&Item::Note {
        kind: NoteKind::Error,
        text: "no account".into(),
    });
    assert_eq!((note.kind.as_str(), note.tone.as_str()), ("note", "error"));
    let footer = Item::StepFooter {
        turn: 2,
        step: 3,
        model: "claude-opus-5-5".into(),
        account: "a@b.c".into(),
        usage: Usage {
            input: 1200,
            output: 300,
            cache_read: 10_000,
            cache_write: 0,
        },
        timing: Timing {
            ttfb_ms: 850,
            total_ms: 3240,
            ..Timing::default()
        },
        stop: StopReason::End,
    };
    let row = msg_row(&footer);
    assert_eq!(row.kind, "footer");
    assert_eq!(
        row.text,
        "turn 2 | step 3 | claude-opus-5-5 | a@b.c | TTFB 850 ms | total 3.2 s | in 1.2k out 300 | cache read 10.0k write 0"
    );
}

fn status(kind: AccountKind, usage: Option<AccountUsage>) -> AccountStatus {
    AccountStatus {
        account: Account {
            id: "id-1".into(),
            kind,
            email: None,
            name: Some("Work".into()),
            plan: Some("max".into()),
            added_at: 0,
        },
        active: true,
        usage,
        held_until: Some(30 * 60_000),
        hold_reason: Some("rate limit".into()),
        token_ok: true,
    }
}

#[test]
fn account_rows() {
    let window = |utilization, resets_at| Window {
        utilization,
        resets_at,
        ..Window::default()
    };
    let usage = AccountUsage {
        five_hour: Some(window(42.0, Some(2 * 3_600_000))),
        seven_day: Some(window(80.0, None)),
        ..AccountUsage::default()
    };
    let row = account_row(&status(AccountKind::Linked, Some(usage)), 0);
    assert_eq!(
        (row.name.as_str(), row.kind.as_str(), row.plan.as_str()),
        ("Work", "linked", "max")
    );
    assert!(row.active && row.token_ok);
    assert_eq!(row.five, 0.42);
    assert_eq!(row.five_text, "42%, resets in 2h 00m");
    assert_eq!((row.seven, row.seven_text.as_str()), (0.8, "80%"));
    assert_eq!(row.hold, "On hold (rate limit), 30m left");
    let bare = account_row(&status(AccountKind::ApiKey, None), 40 * 60_000);
    assert_eq!(
        (
            bare.kind.as_str(),
            bare.five_text.as_str(),
            bare.hold.as_str()
        ),
        ("key", "no data", "")
    );
}

#[test]
fn session_and_trace_rows() {
    let summary = SessionSummary {
        id: "s1".into(),
        created_at: 0,
        title: " ".into(),
        turns: 0,
        last_time: 0,
    };
    let row = session_row(&summary, Some("s1"), 120_000);
    assert_eq!(
        (row.title.as_str(), row.time.as_str()),
        ("(no message yet)", "2 min ago")
    );
    assert!(row.current);
    let step = StepRow {
        turn: 1,
        step: 2,
        tools: vec![("read".into(), 5), ("grep".into(), 7)],
        ..StepRow::default()
    };
    let cells = trace_cells(&step);
    assert_eq!(cells.len(), 12);
    assert_eq!(cells[10].text, "read, grep");
}
