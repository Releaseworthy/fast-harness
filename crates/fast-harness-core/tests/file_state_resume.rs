//! What the session read and wrote survives a reopen (`-c`, a GUI session
//! opened again, PLAN.md D88): `edit` takes a file this session wrote and
//! edited before the restart, and still refuses one changed on disk since.

mod agent_support;

use std::sync::atomic::AtomicBool;
use std::sync::Arc;

use agent_support::{env, key_auth, parts, text_reply, tool_reply, Env};
use fast_harness_core::agent::{Agent, Conversation};
use fast_harness_core::http::{ScriptedResponse, ScriptedTransport};
use fast_harness_core::tools;
use serde_json::json;

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
