//! Phase 20 (PLAN.md D50): the memory block and its breakpoint, the stable
//! block shared across sessions, compaction facts, the 2 KB index cap, the
//! free stats signals, the reflect digest and the selfImprove modes.

use std::fs;
use std::path::{Path, PathBuf};
use std::sync::atomic::AtomicBool;

use fast_harness_core::agent::memory::{self, Memory, MAX_INDEX_BYTES};
use fast_harness_core::agent::{facts, Context};
use fast_harness_core::app::reflect::{finish, Apply};
use fast_harness_core::llm::anthropic::request_body;
use fast_harness_core::llm::{split_memory, Cache, ChatRequest, ToolSpec};
use fast_harness_core::reflect::{self, Mode};
use fast_harness_core::stats;
use fast_harness_core::trace::{kind, TraceEvent};
use fast_harness_core::types::Message;
use serde_json::{json, Value};

fn temp(name: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("fh-mem-{name}-{}", std::process::id()));
    let _ = fs::remove_dir_all(&dir);
    fs::create_dir_all(&dir).unwrap();
    dir
}

fn context(home: &Path, cwd: &Path) -> Context {
    Context::read_at(home, cwd, "claude-opus-5-5", "2026-11-12").0
}

fn write_index(home: &Path, cwd: &Path, text: &str) {
    let dir = memory::dir(home, cwd);
    fs::create_dir_all(&dir).unwrap();
    fs::write(dir.join(memory::INDEX), text).unwrap();
}

fn body(system: &str, oauth: bool) -> Value {
    let cancel = AtomicBool::new(false);
    let messages = vec![Message::user("hi")];
    let schema = json!({"type": "object", "properties": {}});
    let tools = vec![ToolSpec::new("read", "Read a file", schema)];
    let req = ChatRequest::new("claude-opus-5-5", system, &messages, &tools, 4096, &cancel)
        .with_cache(Cache::new(true, false));
    serde_json::from_str(&request_body(&req, oauth)).unwrap()
}

fn breakpoints(value: &Value) -> usize {
    match value {
        Value::Object(map) => {
            usize::from(map.contains_key("cache_control"))
                + map.values().map(breakpoints).sum::<usize>()
        }
        Value::Array(items) => items.iter().map(breakpoints).sum(),
        _ => 0,
    }
}

#[test]
fn memory_block_has_its_own_breakpoint_and_four_at_most() {
    let (home, cwd) = (temp("golden-home"), temp("golden-cwd"));
    write_index(&home, &cwd, "- notes/a.md: alpha\n");
    let system = context(&home, &cwd).system();
    let oauth = body(&system, true);
    let blocks = oauth["system"].as_array().unwrap();
    assert_eq!(blocks.len(), 3);
    assert!(
        blocks[0].get("cache_control").is_none(),
        "identity has none"
    );
    assert_eq!(
        blocks[1]["cache_control"],
        json!({"type": "ephemeral", "ttl": "1h"})
    );
    let memory_text = blocks[2]["text"].as_str().unwrap();
    assert!(memory_text.starts_with("<memory_index>\nMemory folder of this workspace: "));
    assert!(
        memory_text.contains("- notes/a.md: alpha") && memory_text.ends_with("</memory_index>")
    );
    assert_eq!(
        blocks[2]["cache_control"],
        json!({"type": "ephemeral", "ttl": "1h"})
    );
    assert_eq!(breakpoints(&oauth), 4);
    let key = body(&system, false);
    assert_eq!(key["system"].as_array().unwrap().len(), 2);
    assert_eq!(breakpoints(&key), 4);
    // Without a memory block the layout is the one before phase 20.
    let mut plain = context(&home, &cwd);
    plain.memory = None;
    let old = body(&plain.system(), true);
    assert_eq!(old["system"].as_array().unwrap().len(), 2);
    assert_eq!(breakpoints(&old), 4);
    let _ = (fs::remove_dir_all(&home), fs::remove_dir_all(&cwd));
}

#[test]
fn stable_block_is_byte_identical_across_sessions_with_other_memory() {
    let cwd = temp("stable-cwd");
    let (one, two) = (temp("stable-a"), temp("stable-b"));
    write_index(&one, &cwd, "- notes/a.md: alpha\n");
    write_index(&two, &cwd, "- notes/b.md: beta\n- notes/c.md: gamma\n");
    let (a, b) = (context(&one, &cwd), context(&two, &cwd));
    let (body_a, body_b) = (body(&a.system(), true), body(&b.system(), true));
    assert_eq!(body_a["system"][1], body_b["system"][1]);
    assert_ne!(body_a["system"][2], body_b["system"][2]);
    assert_eq!(
        body_a["system"][1]["text"].as_str().unwrap(),
        a.stable_system()
    );
    let system = a.system();
    let (stable, memory) = split_memory(&system);
    assert_eq!(stable, a.stable_system());
    assert!(memory.unwrap().starts_with("<memory_index>"));
    // Other providers get the whole text: the stable part, then the block.
    assert!(system.starts_with(&a.stable_system()) && system.ends_with("</memory_index>"));
    for dir in [cwd, one, two] {
        let _ = fs::remove_dir_all(dir);
    }
}

#[test]
fn memory_is_frozen_per_session_and_changes_come_as_updates() {
    let (home, cwd) = (temp("frozen-home"), temp("frozen-cwd"));
    write_index(&home, &cwd, "- notes/a.md: alpha\n");
    let first = context(&home, &cwd);
    let frozen = first.system();
    let stamp = first.stamp();
    write_index(&home, &cwd, "- notes/a.md: alpha\n- notes/b.md: beta\n");
    let now = context(&home, &cwd);
    assert_ne!(now.system(), frozen, "a new session sees the new index");
    let note = now.update(&stamp).expect("a context update");
    assert!(note.starts_with("[context update] The memory index "));
    assert!(note.contains("- notes/b.md: beta"));
    assert!(now.update(&now.stamp()).is_none());
    // A session stamped before phase 20 gets no memory note.
    let mut old = stamp.clone();
    old.as_object_mut().unwrap().remove("memory");
    assert!(now.update(&old).is_none());
    let _ = (fs::remove_dir_all(&home), fs::remove_dir_all(&cwd));
}

#[test]
fn compaction_facts_append_to_a_note_with_one_index_line() {
    let dir = temp("facts");
    let summary = "## Task: x\n## Durable facts\n- the shell is pwsh\n## Next: y";
    let found = facts(summary);
    assert!(found.contains("the shell is pwsh"));
    assert!(memory::append_facts(&dir, &found, "2026-11-12").unwrap());
    assert!(memory::append_facts(&dir, "- builds need MSVC", "2026-11-13").unwrap());
    assert!(!memory::append_facts(&dir, "none", "2026-11-13").unwrap());
    let note = fs::read_to_string(dir.join(memory::FACTS_NOTE)).unwrap();
    assert!(note.contains("## 2026-11-12") && note.contains("the shell is pwsh"));
    assert!(note.contains("## 2026-11-13") && note.contains("builds need MSVC"));
    let index = fs::read_to_string(dir.join(memory::INDEX)).unwrap();
    assert_eq!(index.lines().count(), 1);
    assert!(index.contains("notes/compaction-facts.md") && index.contains("2026-11-13"));
    let _ = fs::remove_dir_all(&dir);
}

#[test]
fn index_over_2_kb_rolls_oldest_lines_into_a_note() {
    let dir = temp("cap");
    let lines: Vec<String> = (0..40)
        .map(|i| format!("- notes/n{i:02}.md: {}", "x".repeat(60)))
        .collect();
    fs::write(dir.join(memory::INDEX), lines.join("\n")).unwrap();
    memory::set_line(&dir, "notes/new.md", "- notes/new.md: newest").unwrap();
    let index = fs::read_to_string(dir.join(memory::INDEX)).unwrap();
    assert!(index.len() <= MAX_INDEX_BYTES, "{} bytes", index.len());
    assert!(index.lines().next().unwrap().contains(memory::ARCHIVE_NOTE));
    assert_eq!(index.lines().last().unwrap(), "- notes/new.md: newest");
    let archive = fs::read_to_string(dir.join(memory::ARCHIVE_NOTE)).unwrap();
    assert!(archive.starts_with("- notes/n00.md"));
    assert!(!index.contains("notes/n00.md"));
    let _ = fs::remove_dir_all(&dir);
}

fn event(seq: u64, kind: &str, data: Value) -> TraceEvent {
    TraceEvent {
        kind: kind.into(),
        seq,
        time: seq * 1000,
        data,
    }
}

fn result(id: &str, content: &str, error: bool) -> Value {
    json!({"turn": 1, "step": 1, "message": {"role": "tool", "content": [
        {"type": "tool_result", "callId": id, "content": content, "isError": error}]}})
}

fn call(id: &str, name: &str, args: Value) -> Value {
    json!({"turn": 1, "step": 1, "callId": id, "name": name, "arguments": args.to_string()})
}

fn answer(read: u64, write: u64) -> Value {
    json!({"turn": 1, "message": {"role": "assistant", "content": [{"type": "text", "text": "SECRET ANSWER"}]},
        "usage": {"cacheReadTokens": read, "cacheWriteTokens": write}})
}

fn session_events() -> Vec<TraceEvent> {
    vec![
        event(1, kind::TURN_START, json!({"turn": 1})),
        event(2, kind::STEP_START, json!({"turn": 1, "step": 1})),
        event(3, kind::ASSISTANT_MESSAGE, answer(0, 1000)),
        event(
            4,
            kind::TOOL_CALL,
            call("c1", "read", json!({"path": "src/a.rs"})),
        ),
        event(
            5,
            kind::TOOL_RESULT,
            result("c1", "SECRET FILE TEXT", false),
        ),
        event(
            6,
            kind::TOOL_CALL,
            call("c2", "read", json!({"path": "src/a.rs"})),
        ),
        event(
            7,
            kind::TOOL_RESULT,
            result("c2", "SECRET FILE TEXT", false),
        ),
        event(
            8,
            kind::TOOL_CALL,
            call("c3", "edit", json!({"path": "src/a.rs"})),
        ),
        event(
            9,
            kind::TOOL_RESULT,
            result("c3", "old_string not found", true),
        ),
        event(
            10,
            kind::TOOL_CALL,
            call("c4", "shell", json!({"command": "cargo test -q"})),
        ),
        event(
            11,
            kind::TOOL_RESULT,
            result("c4", "SECRET OUTPUT\n[exit code: 101]", false),
        ),
        event(12, kind::TOOL_CALL, call("c5", "web", json!({}))),
        event(
            13,
            kind::TOOL_RESULT,
            result("c5", "tool `web` is not loaded yet: call find_tools", true),
        ),
        event(14, kind::STEP_START, json!({"turn": 1, "step": 2})),
        event(15, kind::ASSISTANT_MESSAGE, answer(100, 2000)),
    ]
}

#[test]
fn stats_count_the_free_signals() {
    let numbers = stats::compute("s1", &session_events());
    let g = &numbers.signals;
    assert_eq!(g.repeated_reads, 1);
    assert_eq!(g.repeated_files[0].name, "src/a.rs");
    assert_eq!((g.failed_edits, g.not_loaded, g.shell_failures), (1, 1, 1));
    assert_eq!(g.failed_commands[0].name, "cargo");
    assert_eq!((g.max_turn_steps, g.cache_rewrites), (2, 1));
    let text = stats::format_text(&numbers);
    assert!(text
        .contains("repeated reads 1, not loaded yet 1, failed edits 1, failed shell commands 1"));
    assert!(text.contains("steps per turn 2.0 (most 2), cache rewrites 1"));
    let mut total = numbers.clone();
    total.merge(numbers);
    assert_eq!(total.signals.repeated_reads, 2);
    assert_eq!(total.signals.failed_commands[0].count, 2);
}

#[test]
fn reflect_digest_keeps_counts_and_errors_not_the_transcript() {
    let digest = reflect::digest(&session_events());
    assert!(digest.contains("turn 1: 2 steps; "));
    assert!(digest.contains("read x2") && digest.contains("edit x1"));
    assert!(digest.contains("error edit: old_string not found"));
    assert!(!digest.contains("SECRET"), "{digest}");
}

const ANSWER: &str = "Two fixes.\n```diff\n--- /dev/null\n+++ b/memory/notes/build.md\n@@ -0,0 +1 @@\n\
+cargo test needs MSVC\n--- a/AGENTS.md\n+++ b/AGENTS.md\n@@ -1 +1,2 @@\n # Rules\n+- run cargo fmt\n```\n";

fn modes_case(mode: Mode, apply: Apply) -> (PathBuf, PathBuf, String) {
    let (home, cwd) = (
        temp(&format!("mode-home-{mode:?}-{apply:?}")),
        temp(&format!("mode-cwd-{mode:?}-{apply:?}")),
    );
    fs::create_dir_all(fast_harness_core::session::dir(&home, &cwd)).unwrap();
    fs::write(cwd.join("AGENTS.md"), "# Rules\n").unwrap();
    let text = finish(&home, &cwd, "s1", ANSWER, mode, apply).unwrap();
    (home, cwd, text)
}

#[test]
fn self_improve_modes_apply_what_they_allow() {
    let note = |home: &Path, cwd: &Path| memory::dir(home, cwd).join("notes/build.md");
    let (home, cwd, text) = modes_case(Mode::Ask, Apply::Mode);
    assert!(text.contains("+cargo test needs MSVC") && text.contains("Nothing applied"));
    assert!(!note(&home, &cwd).exists());
    assert_eq!(
        fs::read_to_string(cwd.join("AGENTS.md")).unwrap(),
        "# Rules\n"
    );
    let (home2, cwd2, _) = modes_case(Mode::Auto, Apply::Mode);
    assert_eq!(
        fs::read_to_string(note(&home2, &cwd2)).unwrap(),
        "cargo test needs MSVC\n"
    );
    assert_eq!(
        fs::read_to_string(cwd2.join("AGENTS.md")).unwrap(),
        "# Rules\n"
    );
    let (home3, cwd3, _) = modes_case(Mode::Ask, Apply::All);
    assert!(note(&home3, &cwd3).exists());
    assert_eq!(
        fs::read_to_string(cwd3.join("AGENTS.md")).unwrap(),
        "# Rules\n- run cargo fmt\n"
    );
    for dir in [home, cwd, home2, cwd2, home3, cwd3] {
        let _ = fs::remove_dir_all(dir);
    }
}

#[test]
fn reflect_off_refuses_and_the_model_pick_prefers_the_setting() {
    let mut config = fast_harness_core::config::Config::default();
    assert_eq!(Mode::of(&config), Mode::Ask);
    config.self_improve = "off".into();
    assert_eq!(Mode::of(&config), Mode::Off);
    config.reflect.model = Some("claude-haiku-5-5".into());
    assert_eq!(reflect::pick_model(&config), "claude-haiku-5-5");
    let memory = Memory {
        dir: PathBuf::from("m"),
        index: "</memory_index> trick".into(),
    };
    assert_eq!(memory.block().matches("</memory_index>").count(), 1);
}
