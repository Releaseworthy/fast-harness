use fast_harness_core::llm::{Timing, ToolSpec};
use fast_harness_core::trace::{self, RequestContext, SessionHeader, TraceWriter};
use fast_harness_core::types::{Block, Message, Role, StopReason, Usage};
use serde_json::json;
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

fn header(id: &str) -> SessionHeader {
    SessionHeader {
        version: 1,
        id: id.into(),
        created_at: 1_794_400_565_123,
        cwd: "F:\\work".into(),
        harness: "fast-harness 0.1.0".into(),
        parent: None,
        depth: 0,
    }
}

fn keys(value: &serde_json::Value) -> Vec<String> {
    let mut keys: Vec<String> = value.as_object().unwrap().keys().cloned().collect();
    keys.sort();
    keys
}

fn sorted(list: &[&str]) -> Vec<String> {
    let mut keys: Vec<String> = list.iter().map(|s| s.to_string()).collect();
    keys.sort();
    keys
}

#[test]
fn writer_is_send() {
    fn assert_send<T: Send>() {}
    assert_send::<TraceWriter>();
}

#[test]
fn header_line_is_exactly_the_plan_format() {
    let tmp = TempDir::new("header");
    let path = tmp.0.join("a").join("b").join("s.jsonl");
    TraceWriter::create(&path, &header("x")).unwrap();
    let text = std::fs::read_to_string(&path).unwrap();
    assert_eq!(
        text,
        "{\"type\":\"session\",\"version\":1,\"id\":\"x\",\"createdAt\":1794400565123,\
         \"cwd\":\"F:\\\\work\",\"harness\":\"fast-harness 0.1.0\",\"parent\":null}\n"
    );
    assert!(
        TraceWriter::create(&path, &header("x")).is_err(),
        "never overwrites"
    );
    let made = SessionHeader::new("y", "C:\\", Some("x".into()));
    assert_eq!(
        made.harness,
        format!("fast-harness {}", env!("CARGO_PKG_VERSION"))
    );
    assert_eq!((made.version, made.parent.as_deref()), (10, Some("x")));
    assert_eq!((trace::VERSION, trace::MIN_VERSION), (10, 1));
}

#[test]
fn emit_round_trips_and_lines_have_key_order() {
    let tmp = TempDir::new("roundtrip");
    let path = tmp.0.join("s.jsonl");
    let mut writer = TraceWriter::create(&path, &header("r")).unwrap();
    assert_eq!(writer.emit("x/one", json!({"a": 1})).unwrap(), 0);
    assert_eq!(writer.emit("x/two", json!([2])).unwrap(), 1);
    let text = std::fs::read_to_string(&path).unwrap();
    let second = text.lines().nth(1).unwrap();
    assert!(second.starts_with("{\"type\":\"x/one\",\"seq\":0,\"time\":"));
    assert!(second.ends_with(",\"data\":{\"a\":1}}"));
    let (head, events) = trace::read(&path).unwrap();
    assert_eq!(head, header("r"));
    assert_eq!(events.len(), 2);
    assert_eq!((events[1].kind.as_str(), events[1].seq), ("x/two", 1));
    assert_eq!(events[1].data, json!([2]));
    assert!(events[0].time >= 1_700_000_000_000);
}

fn fixtures() -> (Usage, Timing, ToolSpec, Message) {
    let usage = Usage::new(4, 363, 0, 24114);
    let mut timing = Timing::new(812, 1190, 5100);
    (timing.chunks, timing.bytes) = (96, 18233);
    let tool = ToolSpec::new("read", "Read a file", json!({"type": "object"}));
    let result = Message::new(Role::Tool, vec![Block::tool_result("c1", "ok", false)]);
    (usage, timing, tool, result)
}

fn write_every_event(path: &Path) {
    let mut w = TraceWriter::create(path, &header("k")).unwrap();
    let (usage, timing, tool, result) = fixtures();
    let context = RequestContext {
        turn: 1,
        step: 1,
        attempt: 1,
        provider: "anthropic",
        model: "claude-opus-5-5",
        account: "u-1f3c",
        effort: Some("high"),
        wire: Some("t1-s1-a1"),
    };
    let (answer, stop) = (Message::user("x"), StopReason::ToolUse);
    w.turn_start(1).unwrap();
    w.user_message(1, &Message::user("hi")).unwrap();
    w.step_start(1, 1).unwrap();
    w.request_header("sys", &[tool]).unwrap();
    w.request_context(&context).unwrap();
    w.assistant_attempt(1, 1, 1, "Overloaded", &timing).unwrap();
    w.llm_retry(1, 1, 1, 2000, "Overloaded").unwrap();
    w.account_switch("u-1f3c", "u-77ab", "RateLimit", Some(9))
        .unwrap();
    w.assistant_message(1, 1, &answer, &usage, stop, &timing, "u-1f3c", 1_000_000)
        .unwrap();
    w.tool_call(1, 1, "c1", "read", "{}").unwrap();
    w.tool_result(1, 1, &result, 3).unwrap();
    w.step_end(1, 1, 5120).unwrap();
    w.turn_end(1, 1, &usage, 14200, "done").unwrap();
}

/// Event type and its data keys, as PLAN.md shows them.
const EXPECTED: [(&str, &str); 13] = [
    ("turn/start", "turn"),
    ("user/message", "turn message"),
    ("step/start", "turn step"),
    ("request/header", "system tools"),
    (
        "request/context",
        "turn step attempt provider model account effort wire",
    ),
    ("assistant/attempt", "turn step attempt error timing"),
    ("llm/retry", "turn step attempt delayMs error"),
    ("account/switch", "from to reason holdUntil"),
    (
        "assistant/message",
        "turn step message usage context window stop timing account",
    ),
    ("tool/call", "turn step callId name arguments"),
    ("tool/result", "turn step message durationMs bytes"),
    ("step/end", "turn step durationMs"),
    ("turn/end", "turn steps usage durationMs outcome"),
];

#[test]
fn every_typed_event_has_the_plan_keys() {
    let tmp = TempDir::new("keys");
    let path = tmp.0.join("s.jsonl");
    write_every_event(&path);
    let (_, events) = trace::read(&path).unwrap();
    assert_eq!(events.len(), EXPECTED.len());
    for (seq, (event, (kind, expected))) in events.iter().zip(EXPECTED).enumerate() {
        assert_eq!((event.kind.as_str(), event.seq), (kind, seq as u64));
        let expected: Vec<&str> = expected.split_whitespace().collect();
        assert_eq!(keys(&event.data), sorted(&expected), "{kind}");
    }
    let answer = &events[8].data;
    assert_eq!(answer["stop"], json!("tool_use"));
    assert_eq!(answer["usage"]["cacheWriteTokens"], json!(24114));
    assert_eq!(answer["timing"]["ttfbMs"], json!(812));
    assert_eq!(answer["window"], json!(1_000_000));
    let context = answer["usage"]["inputTokens"].as_u64().unwrap()
        + answer["usage"]["outputTokens"].as_u64().unwrap()
        + answer["usage"]["cacheReadTokens"].as_u64().unwrap()
        + 24114;
    assert_eq!(answer["context"], json!(context));
    assert!(
        answer["usage"].get("cacheWrite1hTokens").is_none(),
        "0 is left out"
    );
    assert_eq!(events[3].data["tools"][0]["name"], json!("read"));
    assert_eq!(events[4].data["effort"], json!("high"));
    assert_eq!(events[7].data["holdUntil"], json!(9));
    let content = &events[10].data["message"]["content"][0];
    assert_eq!(content["callId"], json!("c1"));
}

#[test]
fn torn_last_line_is_skipped_and_other_bad_lines_fail() {
    let tmp = TempDir::new("torn");
    let path = tmp.0.join("s.jsonl");
    let mut writer = TraceWriter::create(&path, &header("t")).unwrap();
    writer.turn_start(1).unwrap();
    writer.step_start(1, 1).unwrap();
    drop(writer);
    let good = std::fs::read_to_string(&path).unwrap();
    std::fs::write(&path, format!("{good}{{\"type\":\"step/e")).unwrap();
    let (_, events) = trace::read(&path).unwrap();
    assert_eq!(events.len(), 2);
    let mut lines: Vec<&str> = good.lines().collect();
    lines.insert(2, "not json");
    std::fs::write(&path, lines.join("\n") + "\n").unwrap();
    let err = trace::read(&path).unwrap_err();
    assert_eq!(err.kind(), std::io::ErrorKind::InvalidData);
    assert!(err.to_string().contains("line 3"), "{err}");
    std::fs::write(&path, "{\"type\":\"turn/start\"}\n").unwrap();
    assert!(trace::read(&path)
        .unwrap_err()
        .to_string()
        .contains("line 1"));
}

#[test]
fn open_append_continues_seq_and_cuts_a_torn_line() {
    let tmp = TempDir::new("append");
    let path = tmp.0.join("s.jsonl");
    let mut writer = TraceWriter::create(&path, &header("a")).unwrap();
    writer.turn_start(1).unwrap();
    writer.step_start(1, 1).unwrap();
    drop(writer);
    let good = std::fs::read_to_string(&path).unwrap();
    std::fs::write(&path, format!("{good}{{\"type\":\"st")).unwrap();
    let mut writer = TraceWriter::open_append(&path).unwrap();
    assert_eq!(writer.next_seq(), 2);
    assert_eq!(writer.step_end(1, 1, 7).unwrap(), 2);
    drop(writer);
    let (_, events) = trace::read(&path).unwrap();
    let seqs: Vec<u64> = events.iter().map(|e| e.seq).collect();
    assert_eq!(seqs, vec![0, 1, 2]);
    assert_eq!(events[2].kind, "step/end");
    let fresh = tmp.0.join("empty.jsonl");
    TraceWriter::create(&fresh, &header("e")).unwrap();
    assert_eq!(TraceWriter::open_append(&fresh).unwrap().next_seq(), 0);
}

#[test]
fn reader_accepts_versions_1_to_10_only() {
    let tmp = TempDir::new("versions");
    for version in [1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 0] {
        let path = tmp.0.join(format!("v{version}.jsonl"));
        let mut head = header("v");
        head.version = version;
        let mut writer = TraceWriter::create(&path, &head).unwrap();
        writer.turn_start(1).unwrap();
        drop(writer);
        match trace::read(&path) {
            Ok((read, events)) => {
                assert!((1..=10).contains(&version), "read version {version}");
                assert_eq!((read.version, events.len()), (version, 1));
            }
            Err(err) => {
                assert!(version == 11 || version == 0, "{err}");
                assert!(err.to_string().contains("unsupported trace version"));
            }
        }
    }
}

#[test]
fn request_context_without_wire_has_no_wire_key() {
    let tmp = TempDir::new("nowire");
    let path = tmp.0.join("s.jsonl");
    let mut writer = TraceWriter::create(&path, &header("w")).unwrap();
    let context = RequestContext {
        turn: 1,
        step: 1,
        attempt: 1,
        provider: "anthropic",
        model: "m1",
        account: "a1",
        effort: None,
        wire: None,
    };
    writer.request_context(&context).unwrap();
    drop(writer);
    let (_, events) = trace::read(&path).unwrap();
    assert!(events[0].data.get("wire").is_none());
    assert_eq!(events[0].data["effort"], json!(null));
}
