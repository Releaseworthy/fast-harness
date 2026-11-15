//! What the session read and wrote survives a reopen (`-c`, a GUI session
//! opened again, PLAN.md D88): `edit` takes a file this session wrote and
//! edited before the restart, and still refuses one changed on disk since.

mod agent_support;

use std::path::Path;
use std::sync::atomic::AtomicBool;
use std::sync::Arc;

use agent_support::{env, key_auth, parts, text_reply, tool_reply, Env};
use fast_harness_core::agent::{Agent, Conversation};
use fast_harness_core::http::{ScriptedResponse, ScriptedTransport};
use fast_harness_core::tools::{self, ToolCtx, ToolOutput, ToolSession};
use serde_json::{json, Value};

fn agent(env: &Env, script: Vec<ScriptedResponse>) -> Agent {
    let transport = Arc::new(ScriptedTransport::new(script));
    let parts = parts(&transport, key_auth(), &env.cwd);
    let mut agent = Agent::new(parts, Arc::new(tools::builtin()));
    agent.preload = vec!["read".into(), "write".into(), "edit".into()];
    agent
}

fn turn(agent: &Agent, conv: &mut Conversation, text: &str) {
    let cancel = AtomicBool::new(false);
    agent.run_turn(conv, text, &cancel, &mut |_| {});
}

fn write_call(id: &str, content: &str) -> ScriptedResponse {
    let args = json!({"path": "s.txt", "content": content}).to_string();
    tool_reply(&[(id, "write", &args)])
}

fn edit_call(id: &str, old: &str, new: &str) -> ScriptedResponse {
    let args = json!({"path": "s.txt", "old_string": old, "new_string": new}).to_string();
    tool_reply(&[(id, "edit", &args)])
}

/// A session that wrote `s.txt` (`v1`), then edited it (`v2`), closed.
fn written_and_edited(env: &Env) -> String {
    let script = vec![
        write_call("c1", "v1"),
        text_reply("ok"),
        edit_call("c2", "v1", "v2"),
        text_reply("ok"),
    ];
    let agent = agent(env, script);
    let mut conv = Conversation::create(&env.home, &env.cwd, None).unwrap();
    turn(&agent, &mut conv, "create");
    turn(&agent, &mut conv, "change");
    assert_eq!(
        std::fs::read_to_string(env.cwd.join("s.txt")).unwrap(),
        "v2"
    );
    conv.id.clone()
}

/// Reopens session `id` and asks for one more edit `v2` -> `v3`.
fn edit_after_reopen(env: &Env, id: &str) {
    let script = vec![edit_call("c3", "v2", "v3"), text_reply("ok")];
    let agent = agent(env, script);
    let (mut conv, _) = Conversation::reopen(&env.home, &env.cwd, id).unwrap();
    turn(&agent, &mut conv, "again");
}

#[test]
fn a_reopened_session_edits_the_file_it_wrote_and_edited() {
    let env = env("fs-resume");
    let id = written_and_edited(&env);
    edit_after_reopen(&env, &id);
    let file = env.cwd.join("s.txt");
    assert_eq!(std::fs::read_to_string(file).unwrap(), "v3");
}

fn call(session: &ToolSession, cwd: &Path, id: &str, tool: &str, args: Value) -> ToolOutput {
    let cancel = AtomicBool::new(false);
    let ctx = ToolCtx::new(cwd, &cancel)
        .with_call_id(id)
        .with_session(session);
    tools::builtin().run(tool, &args.to_string(), &ctx)
}

/// A restored entry knows no earlier content (D87 meets D88): an edit
/// after the reopen is taken, and the read after it returns the whole
/// file, not the lines changed since a read the new process never saw.
#[test]
fn a_read_after_reopen_and_edit_returns_the_whole_file() {
    let env = env("fs-resume-read");
    let out = env.home.join("s.out");
    let text: String = (1..=50).map(|i| format!("line {i}\n")).collect();
    std::fs::write(env.cwd.join("a.txt"), &text).unwrap();
    let read = json!({"path": "a.txt"});
    let before = ToolSession::new(Some(out.clone()));
    assert!(call(&before, &env.cwd, "c1", "read", read.clone())
        .text()
        .starts_with("1: line 1\n"));
    let edit = json!({"path": "a.txt", "old_string": "line 10\n", "new_string": "ten\n"});
    assert!(!call(&before, &env.cwd, "c2", "edit", edit).is_error());
    drop(before);
    let after = ToolSession::new(Some(out));
    let edit = json!({"path": "a.txt", "old_string": "line 40\n", "new_string": "forty\n"});
    let edited = call(&after, &env.cwd, "c3", "edit", edit);
    assert!(!edited.is_error(), "{}", edited.text());
    let reread = call(&after, &env.cwd, "c4", "read", read).into_text();
    assert!(reread.starts_with("1: line 1\n"), "{reread}");
    assert!(reread.contains("\n10: ten\n") && reread.contains("\n40: forty\n"));
}

/// A session from before `<id>.files.json` (the owner's, 2026-11-15): the
/// log alone shows what it wrote and edited (PLAN.md D90).
fn without_saved_state(env: &Env, id: &str) {
    let out = fast_harness_core::session::out_dir(&env.home, &env.cwd, id);
    std::fs::remove_file(out.with_extension("files.json")).unwrap();
}

#[test]
fn a_session_log_without_saved_state_still_knows_the_files_it_changed() {
    let env = env("fs-resume-log");
    let id = written_and_edited(&env);
    without_saved_state(&env, &id);
    edit_after_reopen(&env, &id);
    let file = env.cwd.join("s.txt");
    assert_eq!(std::fs::read_to_string(file).unwrap(), "v3");
}

#[test]
fn a_session_log_without_saved_state_refuses_a_file_changed_after_it() {
    let env = env("fs-resume-log-changed");
    let id = written_and_edited(&env);
    without_saved_state(&env, &id);
    let file = env.cwd.join("s.txt");
    std::fs::write(&file, "v2").unwrap();
    let later = std::time::SystemTime::now() + std::time::Duration::from_secs(60);
    let handle = std::fs::File::options().write(true).open(&file).unwrap();
    handle.set_modified(later).unwrap();
    drop(handle);
    edit_after_reopen(&env, &id);
    assert_eq!(std::fs::read_to_string(&file).unwrap(), "v2");
}

#[test]
fn a_reopened_session_refuses_a_file_changed_on_disk_since() {
    let env = env("fs-resume-changed");
    let id = written_and_edited(&env);
    let file = env.cwd.join("s.txt");
    std::fs::write(&file, "v2 changed by someone else").unwrap();
    edit_after_reopen(&env, &id);
    let after = std::fs::read_to_string(&file).unwrap();
    assert_eq!(after, "v2 changed by someone else");
}
