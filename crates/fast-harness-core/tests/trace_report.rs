use fast_harness_core::llm::Timing;
use fast_harness_core::trace::report::{format_table, step_rows, StepRow};
use fast_harness_core::trace::{self, RequestContext, SessionHeader, TraceWriter};
use fast_harness_core::types::{Block, Message, Role, StopReason, Usage};
use std::path::{Path, PathBuf};

struct TempDir(PathBuf);

impl Drop for TempDir {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

fn context<'a>(step: u32, attempt: u32, account: &'a str) -> RequestContext<'a> {
    RequestContext {
        turn: 1,
        step,
        attempt,
        provider: "anthropic",
        model: "claude-opus-5-5",
        account,
        effort: Some("high"),
        wire: Some("t1-s1-a1"),
    }
}

fn result(call_id: &str) -> Message {
    Message {
        role: Role::Tool,
        content: vec![Block::ToolResult {
            call_id: call_id.into(),
            content: "ok".into(),
            is_error: false,
        }],
    }
}

fn timing(ttfb_ms: u64, total_ms: u64) -> Timing {
    Timing {
        ttfb_ms,
        total_ms,
        ..Timing::default()
    }
}

fn usage(input: u64, output: u64, cache_read: u64, cache_write: u64) -> Usage {
    Usage {
        input,
        output,
        cache_read,
        cache_write,
    }
}

fn write_session(path: &Path) {
    let header = SessionHeader::new("r", "F:\\work", None);
    let mut w = TraceWriter::create(path, &header).unwrap();
    let answer = Message::user("answer");
    w.turn_start(1).unwrap();
    w.user_message(1, &Message::user("go")).unwrap();
    w.step_start(1, 1).unwrap();
    w.request_context(&context(1, 1, "u-1f3c")).unwrap();
    w.assistant_attempt(1, 1, 1, "Overloaded", &timing(0, 900))
        .unwrap();
    w.llm_retry(1, 1, 1, 2000, "Overloaded").unwrap();
    w.request_context(&context(1, 2, "u-1f3c")).unwrap();
    let first = usage(4, 363, 0, 24114);
    w.assistant_message(
        1,
        1,
        &answer,
        &first,
        StopReason::ToolUse,
        &timing(812, 5100),
        "u-1f3c",
    )
    .unwrap();
    w.tool_call(1, 1, "c1", "read", "{}").unwrap();
    w.tool_call(1, 1, "c2", "grep", "{}").unwrap();
    w.tool_result(1, 1, &result("c1"), 3).unwrap();
    w.tool_result(1, 1, &result("c2"), 12).unwrap();
    w.step_end(1, 1, 5120).unwrap();
    w.step_start(1, 2).unwrap();
    w.account_switch("u-1f3c", "u-77ab", "RateLimit", None)
        .unwrap();
    w.request_context(&context(2, 1, "u-77ab")).unwrap();
    let second = usage(2, 50, 24114, 0);
    w.assistant_message(
        1,
        2,
        &answer,
        &second,
        StopReason::End,
        &timing(640, 2000),
        "u-77ab",
    )
    .unwrap();
    w.step_end(1, 2, 2010).unwrap();
}

#[test]
fn step_rows_from_a_session_log() {
    let tmp = TempDir(std::env::temp_dir().join(format!(
        "fast-harness-test-report-{}-{}",
        std::process::id(),
        trace::now_ms()
    )));
    let path = tmp.0.join("r.jsonl");
    write_session(&path);
    let (_, events) = trace::read(&path).unwrap();
    let rows = step_rows(&events);
    assert_eq!(rows.len(), 2);
    let expected = StepRow {
        turn: 1,
        step: 1,
        account: "u-1f3c".into(),
        model: "claude-opus-5-5".into(),
        ttfb_ms: 812,
        total_ms: 5100,
        usage: usage(4, 363, 0, 24114),
        tools: vec![("read".into(), 3), ("grep".into(), 12)],
        retries: 1,
    };
    assert_eq!(rows[0], expected);
    assert_eq!((rows[1].account.as_str(), rows[1].retries), ("u-77ab", 0));
    assert_eq!(format_table(&rows), GOLDEN.join("\n") + "\n");
}

const GOLDEN: [&str; 4] = [
    " turn  step  account  model            ttfb ms  total ms  input  output  cache read  cache write  retries  tools",
    "    1     1  u-1f3c   claude-opus-5-5      812      5100      4     363           0        24114        1  read 3ms, grep 12ms",
    "    1     2  u-77ab   claude-opus-5-5      640      2000      2      50       24114            0        0",
    "total     2                                         7100      6     413       24114        24114        1  2 calls 15ms",
];

#[test]
fn empty_table_has_header_and_totals() {
    let table = format_table(&[]);
    assert_eq!(table.lines().count(), 2);
    assert!(table.lines().nth(1).unwrap().starts_with("total"));
}
