use fast_harness_core::llm::Timing;
use fast_harness_core::session::{self, SessionSummary};
use fast_harness_core::trace::{self, SessionHeader, TraceWriter};
use fast_harness_core::types::{Block, Message, Role, StopReason, Usage};
use std::path::{Path, PathBuf};

struct TempDir(PathBuf);

impl TempDir {
    fn new(name: &str) -> TempDir {
        let path = std::env::temp_dir().join(format!(
            "fast-harness-test-{name}-{}-{}",
            std::process::id(),
            trace::now_ms()
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

#[test]
fn slug_matches_the_plan_example() {
    assert_eq!(
        session::slug(Path::new("F:\\work\\deepseek-general")),
        "--F-work-deepseek-general--"
    );
    assert_eq!(session::slug(Path::new("/home/me/x")), "---home-me-x--");
    let home = Path::new("H");
    let cwd = Path::new("F:\\w");
    let dir = session::dir(home, cwd);
    assert_eq!(dir, home.join("sessions").join("--F-w--"));
    assert_eq!(session::log_path(home, cwd, "id1"), dir.join("id1.jsonl"));
    assert_eq!(session::wire_dir(home, cwd, "id1"), dir.join("id1.wire"));
}

#[test]
fn utc_stamp_computes_the_calendar() {
    assert_eq!(session::utc_stamp(0), "19700101-000000");
    assert_eq!(session::utc_stamp(951_782_400), "20000229-000000");
    assert_eq!(session::utc_stamp(1_794_400_565), "20261111-123605");
    assert_eq!(session::utc_stamp(4_107_542_399), "21000228-235959");
}

#[test]
fn new_id_has_the_format_and_sorts_by_creation() {
    let ids: Vec<String> = (0..50).map(|_| session::new_id()).collect();
    let pattern = regex::Regex::new(r"^\d{8}-\d{6}-[0-9a-f]{4}$").unwrap();
    for id in &ids {
        assert!(pattern.is_match(id), "{id}");
    }
    let mut sorted = ids.clone();
    sorted.sort();
    sorted.dedup();
    assert_eq!(sorted.len(), ids.len(), "ids are unique");
    let now = session::utc_stamp(trace::now_ms() / 1000);
    assert_eq!(&ids[0][..8], &now[..8]);
}

fn header(id: &str, created_at: u64) -> SessionHeader {
    SessionHeader {
        version: 1,
        id: id.into(),
        created_at,
        cwd: "F:\\p".into(),
        harness: "fast-harness 0.1.0".into(),
        parent: None,
    }
}

fn tool_turn(w: &mut TraceWriter) {
    let call = Message {
        role: Role::Assistant,
        content: vec![Block::ToolCall {
            id: "c1".into(),
            name: "read".into(),
            arguments: "{}".into(),
            signature: None,
        }],
        model: None,
    };
    let result = Message {
        role: Role::Tool,
        content: vec![Block::ToolResult {
            call_id: "c1".into(),
            content: "ok".into(),
            is_error: false,
        }],
        model: None,
    };
    let (usage, timing) = (Usage::default(), Timing::default());
    w.assistant_message(1, 1, &call, &usage, StopReason::ToolUse, &timing, "a")
        .unwrap();
    w.tool_call(1, 1, "c1", "read", "{}").unwrap();
    w.tool_result(1, 1, &result, 2).unwrap();
}

fn write_log(path: &Path, header: &SessionHeader, first: &str, end_turn: bool) -> Vec<Message> {
    let mut w = TraceWriter::create(path, header).unwrap();
    let user = Message::user(first);
    w.turn_start(1).unwrap();
    w.user_message(1, &user).unwrap();
    w.step_start(1, 1).unwrap();
    tool_turn(&mut w);
    if end_turn {
        w.turn_end(1, 2, &Usage::default(), 10, "done").unwrap();
        w.turn_start(2).unwrap();
        w.user_message(2, &Message::user("second")).unwrap();
    }
    let (_, events) = trace::read(path).unwrap();
    let mut messages = vec![user];
    for event in events
        .iter()
        .filter(|e| e.data.get("message").is_some())
        .skip(1)
    {
        messages.push(serde_json::from_value(event.data["message"].clone()).unwrap());
    }
    messages
}

#[test]
fn replay_rebuilds_the_conversation_even_without_turn_end() {
    let tmp = TempDir::new("replay");
    let path = tmp.0.join("s.jsonl");
    let expected = write_log(&path, &header("s", 1), "hello", false);
    let messages = session::replay(&path).unwrap();
    assert_eq!(messages, expected);
    let roles: Vec<Role> = messages.iter().map(|m| m.role).collect();
    assert_eq!(roles, vec![Role::User, Role::Assistant, Role::Tool]);
}

#[test]
fn list_is_newest_first_with_titles_and_turns() {
    let tmp = TempDir::new("list");
    let (home, cwd) = (tmp.0.as_path(), Path::new("F:\\p"));
    assert!(session::list(home, cwd).unwrap().is_empty());
    assert_eq!(session::latest(home, cwd).unwrap(), None);
    let long = "x".repeat(80);
    write_log(
        &session::log_path(home, cwd, "old"),
        &header("old", 100),
        "first one",
        true,
    );
    write_log(
        &session::log_path(home, cwd, "new"),
        &header("new", 300),
        &long,
        false,
    );
    write_log(
        &session::log_path(home, cwd, "mid"),
        &header("mid", 200),
        "middle",
        false,
    );
    std::fs::write(session::dir(home, cwd).join("junk.jsonl"), "garbage\n").unwrap();
    std::fs::create_dir_all(session::wire_dir(home, cwd, "old")).unwrap();
    let list = session::list(home, cwd).unwrap();
    let ids: Vec<&str> = list.iter().map(|s| s.id.as_str()).collect();
    assert_eq!(ids, vec!["new", "mid", "old"]);
    assert_eq!(list[0].title, "x".repeat(60));
    let old: &SessionSummary = &list[2];
    assert_eq!((old.title.as_str(), old.turns), ("first one", 2));
    assert!(old.last_time >= 1_700_000_000_000);
    assert_eq!(session::latest(home, cwd).unwrap().unwrap().id, "new");
}
