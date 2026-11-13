//! The session index, `latest` and the one-pass resume (PLAN.md D60, G10).

use fast_harness_core::agent::{repair, Conversation};
use fast_harness_core::llm::Timing;
use fast_harness_core::session::{self, index::INDEX_FILE, SessionSummary};
use fast_harness_core::store;
use fast_harness_core::trace::{self, kind, SessionHeader, TraceWriter};
use fast_harness_core::types::{Block, Message, Role, StopReason, Usage};
use serde_json::json;
use std::path::{Path, PathBuf};

struct Env {
    home: PathBuf,
    cwd: PathBuf,
}

impl Drop for Env {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.home);
    }
}

fn env(name: &str) -> Env {
    let home = std::env::temp_dir().join(format!(
        "fast-harness-test-index-{name}-{}-{}",
        std::process::id(),
        trace::now_ms()
    ));
    let _ = std::fs::remove_dir_all(&home);
    Env {
        cwd: home.join("work"),
        home,
    }
}

fn path(env: &Env, id: &str) -> PathBuf {
    session::log_path(&env.home, &env.cwd, id)
}

fn writer(env: &Env, id: &str) -> TraceWriter {
    let header = SessionHeader::new(id, &env.cwd.to_string_lossy(), None);
    TraceWriter::create(&path(env, id), &header).unwrap()
}

fn answer(w: &mut TraceWriter, turn: u32, message: &Message) {
    let (usage, timing) = (Usage::new(10, 2, 0, 0), Timing::default());
    let stop = StopReason::End;
    (w.assistant_message(turn, 1, message, &usage, stop, &timing, "a", 0)).unwrap();
}

fn call_message() -> Message {
    let call = Block::ToolCall {
        id: "c1".into(),
        name: "echo".into(),
        arguments: "{}".into(),
        signature: None,
    };
    Message::new(Role::Assistant, vec![call])
}

/// Finished, unfinished, compacted, empty and damaged (last and middle) logs
/// plus a foreign file; returns the log ids.
fn seed(env: &Env) -> Vec<&'static str> {
    let mut w = writer(env, "20261113-100000-0001");
    w.turn_start(1).unwrap();
    w.user_message(1, &Message::user("finished one\nsecond line"))
        .unwrap();
    let header = json!({"system": "sys", "tools": [{"name": "echo"}]});
    w.emit("request/header", header).unwrap();
    answer(&mut w, 1, &Message::assistant("done"));
    w.turn_end(1, 1, &Usage::default(), 5, "done").unwrap();
    let mut w = writer(env, "20261113-100001-0002");
    w.turn_start(1).unwrap();
    w.user_message(1, &Message::user("crashed")).unwrap();
    answer(&mut w, 1, &call_message());
    let mut w = writer(env, "20261113-100002-0003");
    w.turn_start(1).unwrap();
    w.user_message(1, &Message::user("compacted")).unwrap();
    answer(&mut w, 1, &Message::assistant("long"));
    w.emit(
        "compaction",
        json!({"summary": "s", "after": 7, "mid": true}),
    )
    .unwrap();
    w.turn_end(1, 1, &Usage::default(), 5, "paused").unwrap();
    drop(writer(env, "20261113-100003-0004"));
    let mut w = writer(env, "20261113-100004-0005");
    w.turn_start(1).unwrap();
    w.user_message(1, &Message::user("torn")).unwrap();
    append(&path(env, "20261113-100004-0005"), "{\"type\":\"turn/e");
    drop(w);
    let mut w = writer(env, "20261113-100005-0006");
    w.turn_start(1).unwrap();
    w.user_message(1, &Message::user("middle")).unwrap();
    drop(w);
    let middle = path(env, "20261113-100005-0006");
    let text = std::fs::read_to_string(&middle).unwrap();
    let mut lines: Vec<&str> = text.lines().collect();
    lines.insert(2, "{broken}");
    std::fs::write(&middle, lines.join("\n") + "\n").unwrap();
    let folder = session::dir(&env.home, &env.cwd);
    std::fs::write(folder.join("notes.txt"), "not a log").unwrap();
    std::fs::write(folder.join("junk.jsonl"), "garbage\n").unwrap();
    vec![
        "20261113-100000-0001",
        "20261113-100001-0002",
        "20261113-100002-0003",
        "20261113-100003-0004",
        "20261113-100004-0005",
        "20261113-100005-0006",
    ]
}

fn append(path: &Path, text: &str) {
    use std::io::Write;
    let mut file = std::fs::OpenOptions::new().append(true).open(path).unwrap();
    file.write_all(text.as_bytes()).unwrap();
}

/// The list as it was before the index: every log parsed in full.
fn full_parse(env: &Env) -> Vec<SessionSummary> {
    let folder = session::dir(&env.home, &env.cwd);
    let mut out: Vec<SessionSummary> = std::fs::read_dir(folder)
        .unwrap()
        .map(|e| e.unwrap().path())
        .filter(|p| session::coord::is_log(p))
        .filter_map(|p| session::summary_of(&p))
        .collect();
    out.sort_by(|a, b| (b.created_at, &b.id).cmp(&(a.created_at, &a.id)));
    out
}

fn index_text(env: &Env) -> String {
    std::fs::read_to_string(session::dir(&env.home, &env.cwd).join(INDEX_FILE)).unwrap()
}

#[test]
fn a_cold_and_a_warm_index_list_what_a_full_parse_lists() {
    let env = env("cold");
    let ids = seed(&env);
    let reference = full_parse(&env);
    assert_eq!(reference.len(), ids.len(), "{reference:?}");
    let cold = session::list_counted(&env.home, &env.cwd).unwrap();
    assert_eq!(cold.sessions, reference);
    assert_eq!((cold.parsed, cold.cached, cold.written), (7, 0, true));
    let warm = session::list_counted(&env.home, &env.cwd).unwrap();
    assert_eq!(warm.sessions, reference);
    // A warm list reads no log: every one is answered from the index.
    assert_eq!((warm.parsed, warm.cached, warm.written), (0, 7, false));
    let states: Vec<_> = reference.iter().map(|s| s.unfinished).collect();
    assert!(states.contains(&Some(session::Unfinished::Interrupted)));
    assert!(states.contains(&Some(session::Unfinished::Paused)));
    assert!(reference.iter().any(|s| s.title == "finished one"));
}

#[test]
fn a_grown_shrunk_deleted_or_replaced_log_is_seen() {
    let env = env("stale");
    let ids = seed(&env);
    session::list(&env.home, &env.cwd).unwrap();
    let mut w = TraceWriter::open_append(&path(&env, ids[0])).unwrap();
    w.turn_start(2).unwrap();
    drop(w);
    let text = std::fs::read_to_string(path(&env, ids[1])).unwrap();
    let shorter: Vec<&str> = text.lines().take(3).collect();
    std::fs::write(path(&env, ids[1]), shorter.join("\n") + "\n").unwrap();
    std::fs::remove_file(path(&env, ids[2])).unwrap();
    std::fs::remove_file(path(&env, ids[3])).unwrap();
    let mut w = writer(&env, ids[3]);
    w.turn_start(1).unwrap();
    w.user_message(1, &Message::user("replaced")).unwrap();
    drop(w);
    let listing = session::list_counted(&env.home, &env.cwd).unwrap();
    assert_eq!(listing.sessions, full_parse(&env));
    assert_eq!((listing.parsed, listing.cached), (3, 3));
    assert!(!index_text(&env).contains(ids[2]));
    let first = listing.sessions.iter().find(|s| s.id == ids[0]).unwrap();
    assert_eq!(first.turns, 2);
}

#[test]
fn a_damaged_or_old_index_is_rebuilt_and_an_unwritable_one_still_lists() {
    let env = env("damaged");
    seed(&env);
    let index = session::dir(&env.home, &env.cwd).join(INDEX_FILE);
    for bad in [
        "{\"version\":1,\"logs\":{\"x",
        "{\"version\":0,\"logs\":{}}",
    ] {
        std::fs::write(&index, bad).unwrap();
        let listing = session::list_counted(&env.home, &env.cwd).unwrap();
        assert_eq!((listing.parsed, listing.written), (7, true));
        assert_eq!(listing.sessions, full_parse(&env));
    }
    // A read-only index cannot be replaced on Windows: every list parses and
    // answers. Elsewhere a rename replaces a read-only file, so the first
    // list rebuilds the index (an unwritable folder has no test there).
    std::fs::write(&index, "{}").unwrap();
    let mut perms = std::fs::metadata(&index).unwrap().permissions();
    perms.set_readonly(true);
    std::fs::set_permissions(&index, perms.clone()).unwrap();
    for round in 0..2 {
        let listing = session::list_counted(&env.home, &env.cwd).unwrap();
        match cfg!(windows) {
            true => assert_eq!((listing.parsed, listing.written), (7, false)),
            false => assert_eq!(listing.written, round == 0),
        }
        assert_eq!(listing.sessions, full_parse(&env));
    }
    #[allow(clippy::permissions_set_readonly_false)] // the test's own file
    perms.set_readonly(false);
    std::fs::set_permissions(&index, perms).unwrap();
}

#[test]
fn threads_listing_and_appending_leave_a_whole_index() {
    let env = env("threads");
    let ids = seed(&env);
    let log = path(&env, ids[0]);
    std::thread::scope(|scope| {
        scope.spawn(|| {
            let mut w = TraceWriter::open_append(&log).unwrap();
            for turn in 2..60 {
                w.turn_start(turn).unwrap();
            }
        });
        scope.spawn(|| {
            for n in 0..20 {
                let id = format!("20261113-110000-{n:04x}");
                writer(&env, &id).turn_start(1).unwrap();
            }
        });
        for _ in 0..3 {
            scope.spawn(|| {
                for _ in 0..20 {
                    session::list(&env.home, &env.cwd).unwrap();
                    let index = session::dir(&env.home, &env.cwd).join(INDEX_FILE);
                    if let Ok(text) = store::read_to_string(index) {
                        serde_json::from_str::<serde_json::Value>(&text).unwrap();
                    }
                }
            });
        }
    });
    let after = session::list_counted(&env.home, &env.cwd).unwrap();
    assert_eq!(after.sessions, full_parse(&env));
    assert_eq!(after.sessions.len(), ids.len() + 20);
    let warm = session::list_counted(&env.home, &env.cwd).unwrap();
    assert_eq!((warm.parsed, warm.cached), (0, 27));
    assert!(std::fs::read_dir(session::dir(&env.home, &env.cwd))
        .unwrap()
        .all(|e| !e.unwrap().file_name().to_string_lossy().ends_with(".tmp")));
}

#[test]
fn latest_takes_the_highest_id_in_one_second_and_skips_foreign_files() {
    let env = env("latest");
    assert_eq!(session::latest(&env.home, &env.cwd).unwrap(), None);
    for id in [
        "20261113-120000-00ff",
        "20261113-120000-0a00",
        "20261113-115959-ffff",
    ] {
        writer(&env, id).turn_start(1).unwrap();
    }
    // A foreign log with a later header and a broken log with a higher id.
    let foreign = SessionHeader::new("zzz", "x", None);
    TraceWriter::create(&path(&env, "zzz"), &foreign).unwrap();
    std::fs::write(path(&env, "20261113-120001-0000"), "garbage\n").unwrap();
    let latest = session::latest(&env.home, &env.cwd).unwrap().unwrap();
    assert_eq!(latest.id, "20261113-120000-0a00");
    assert_eq!(latest.turns, 1);
}

/// Messages, turn, system, unfinished state and next seq of a resume.
type Resumed = (
    Vec<Message>,
    u32,
    Option<String>,
    Option<session::Unfinished>,
    u64,
);

/// The resume of a log before G10: three parses.
fn three_parses(log: &Path) -> Resumed {
    let mut messages = session::replay(log).unwrap();
    repair(&mut messages);
    let (_, events) = trace::read(log).unwrap();
    let turn = (events.iter())
        .filter(|e| e.kind == kind::TURN_START)
        .filter_map(|e| e.data["turn"].as_u64())
        .max()
        .unwrap_or(0) as u32;
    let system = (events.iter())
        .filter(|e| e.kind == kind::REQUEST_HEADER)
        .max_by_key(|e| e.seq)
        .and_then(|e| e.data["system"].as_str().map(str::to_string));
    let unfinished = session::unfinished(&events);
    let seq = TraceWriter::open_append(log).unwrap().next_seq();
    (messages, turn, system, unfinished, seq)
}

#[test]
fn a_one_pass_resume_equals_the_three_parses() {
    let env = env("resume");
    for id in seed(&env) {
        let reference = three_parses(&path(&env, id));
        let (conv, events) = Conversation::reopen(&env.home, &env.cwd, id).unwrap();
        let got = (conv.messages.clone(), conv.turn, conv.system.clone());
        let got = (got.0, got.1, got.2, conv.unfinished, conv.next_seq());
        assert_eq!(got, reference, "{id}");
        assert_eq!(events, trace::read(&path(&env, id)).unwrap().1, "{id}");
        assert_eq!(conv.take_notes().len(), usize::from(id.ends_with("0006")));
    }
}
