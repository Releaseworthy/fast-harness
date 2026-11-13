//! Undo of a turn's file changes (PLAN.md D69): restore, created files,
//! conflicts and force, caps, refusal during a turn, the note to the model
//! behind an unchanged prefix, replay after reopen, undo turn by turn.

mod agent_support;
mod service_support;

use std::path::Path;
use std::sync::atomic::AtomicBool;
use std::sync::Arc;

use agent_support::{env, key_auth, parts, text_reply, tool_reply, Env};
use fast_harness_core::agent::{Agent, Conversation};
use fast_harness_core::events::AgentEvent;
use fast_harness_core::http::{ScriptedResponse, ScriptedTransport};
use fast_harness_core::service::undo::{command, undo_conv};
use fast_harness_core::service::{Service, ServiceError, Start};
use fast_harness_core::tools::{self, undo::Caps};
use fast_harness_core::trace::{self, Record};
use fast_harness_core::view::Transcript;
use serde_json::{json, Value};
use service_support::{watch, Gate, TestHost};

fn agent(env: &Env, script: Vec<ScriptedResponse>) -> (Agent, Arc<ScriptedTransport>) {
    let transport = Arc::new(ScriptedTransport::new(script));
    let parts = parts(&transport, key_auth(), &env.cwd);
    let mut agent = Agent::new(parts, Arc::new(tools::builtin()));
    agent.preload = vec!["write".into(), "edit".into()];
    (agent, transport)
}

fn turn(agent: &Agent, conv: &mut Conversation, text: &str) {
    let cancel = AtomicBool::new(false);
    agent.run_turn(conv, text, &cancel, &mut |_| {});
}

fn write_call(id: &str, path: &str, content: &str) -> ScriptedResponse {
    let args = json!({"path": path, "content": content}).to_string();
    tool_reply(&[(id, "write", &args)])
}

fn edit_call(id: &str, path: &str, old: &str, new: &str) -> ScriptedResponse {
    let args = json!({"path": path, "old_string": old, "new_string": new}).to_string();
    tool_reply(&[(id, "edit", &args)])
}

fn read(path: &Path) -> String {
    std::fs::read_to_string(path).unwrap()
}

#[test]
fn write_then_edit_then_undo_restores_the_bytes() {
    let env = env("undo-restore");
    let file = env.cwd.join("a.txt");
    std::fs::write(&file, "one\r\n").unwrap();
    let script = vec![
        write_call("c1", "a.txt", "two\n"),
        edit_call("c2", "a.txt", "two", "three"),
        text_reply("done"),
    ];
    let (agent, _) = agent(&env, script);
    let mut conv = Conversation::create(&env.home, &env.cwd, None).unwrap();
    turn(&agent, &mut conv, "change it");
    assert_eq!(read(&file), "three\n");
    let info = conv.tool_session.undo().last().unwrap();
    assert_eq!((info.turn, info.files, info.not_undoable), (1, 1, None));
    let outcome = undo_conv(&mut conv, false).unwrap();
    assert_eq!(outcome.refused, None, "{}", outcome.text());
    assert_eq!(outcome.restored, std::slice::from_ref(&file));
    assert_eq!(std::fs::read(&file).unwrap(), b"one\r\n");
    assert!(outcome.text().contains("shell commands, background jobs"));
    assert!(conv.tool_session.undo().last().is_none());
}

#[test]
fn a_created_file_is_removed() {
    let env = env("undo-created");
    let script = vec![write_call("c1", "sub/new.txt", "hello"), text_reply("ok")];
    let (agent, _) = agent(&env, script);
    let mut conv = Conversation::create(&env.home, &env.cwd, None).unwrap();
    turn(&agent, &mut conv, "make it");
    let file = env.cwd.join("sub/new.txt");
    assert!(file.exists());
    let outcome = undo_conv(&mut conv, false).unwrap();
    assert_eq!(outcome.removed, std::slice::from_ref(&file));
    assert!(!file.exists());
}

#[test]
fn one_file_named_two_ways_in_a_turn_is_one_file() {
    let env = env("undo-alias");
    let file = env.cwd.join("a.txt");
    std::fs::write(&file, "zero").unwrap();
    std::fs::create_dir_all(env.cwd.join("sub")).unwrap();
    let script = vec![
        write_call("c1", "a.txt", "one"),
        write_call("c2", "sub/../a.txt", "two"),
        text_reply("ok"),
    ];
    let (agent, _) = agent(&env, script);
    let mut conv = Conversation::create(&env.home, &env.cwd, None).unwrap();
    turn(&agent, &mut conv, "twice");
    assert_eq!(read(&file), "two");
    assert_eq!(conv.tool_session.undo().last().unwrap().files, 1);
    let outcome = undo_conv(&mut conv, false).unwrap();
    assert_eq!(outcome.refused, None, "{}", outcome.text());
    assert_eq!(read(&file), "zero");
}

#[test]
fn a_file_that_cannot_be_put_back_keeps_its_copy() {
    let env = env("undo-keep");
    let dir = env.cwd.join("d");
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(dir.join("x.txt"), "zero").unwrap();
    let script = vec![write_call("c1", "d/x.txt", "one"), text_reply("ok")];
    let (agent, _) = agent(&env, script);
    let mut conv = Conversation::create(&env.home, &env.cwd, None).unwrap();
    turn(&agent, &mut conv, "change");
    // The folder became a file: the forced undo cannot write x.txt back.
    std::fs::remove_dir_all(&dir).unwrap();
    std::fs::write(&dir, "a file now").unwrap();
    let outcome = undo_conv(&mut conv, true).unwrap();
    assert_eq!(outcome.errors.len(), 2, "{}", outcome.text());
    let kept = outcome.errors[1].split("stay in ").nth(1).unwrap();
    let copy = std::fs::read_dir(kept.trim_end_matches('.')).unwrap();
    let blobs: Vec<_> = copy
        .flatten()
        .filter(|e| e.path().extension().is_some_and(|x| x == "bin"))
        .collect();
    assert_eq!(blobs.len(), 1, "the earlier content is not lost");
    assert_eq!(std::fs::read(blobs[0].path()).unwrap(), b"zero");
}

#[test]
fn a_conflict_changes_nothing_and_force_overwrites() {
    let env = env("undo-conflict");
    let (a, b) = (env.cwd.join("a.txt"), env.cwd.join("b.txt"));
    std::fs::write(&a, "a0").unwrap();
    std::fs::write(&b, "b0").unwrap();
    let script = vec![
        write_call("c1", "a.txt", "a1"),
        write_call("c2", "b.txt", "b1"),
        text_reply("ok"),
    ];
    let (agent, _) = agent(&env, script);
    let mut conv = Conversation::create(&env.home, &env.cwd, None).unwrap();
    turn(&agent, &mut conv, "both");
    std::fs::write(&b, "user edit").unwrap();
    let outcome = undo_conv(&mut conv, false).unwrap();
    assert!(outcome.refused.as_deref().unwrap().contains("force"));
    assert_eq!(outcome.conflicts, std::slice::from_ref(&b));
    assert!(outcome.restored.is_empty());
    assert_eq!((read(&a), read(&b)), ("a1".into(), "user edit".into()));
    assert!(trace_undos(&conv).is_empty(), "a refused undo logs nothing");
    let forced = undo_conv(&mut conv, true).unwrap();
    assert!(forced.forced && forced.refused.is_none());
    assert_eq!((read(&a), read(&b)), ("a0".into(), "b0".into()));
}

#[test]
fn over_the_cap_a_turn_is_not_undoable() {
    let env = env("undo-cap");
    let file = env.cwd.join("big.txt");
    std::fs::write(&file, "0123456789").unwrap();
    let script = vec![write_call("c1", "big.txt", "small"), text_reply("ok")];
    let (agent, _) = agent(&env, script);
    let mut conv = Conversation::create(&env.home, &env.cwd, None).unwrap();
    let caps = Caps {
        turn: 4,
        session: 1000,
    };
    conv.tool_session.undo().set_caps(caps);
    turn(&agent, &mut conv, "shrink");
    let info = conv.tool_session.undo().last().unwrap();
    assert!(info
        .not_undoable
        .as_deref()
        .unwrap()
        .contains("4 bytes cap per turn"));
    let outcome = undo_conv(&mut conv, false).unwrap();
    assert!(outcome.refused.as_deref().unwrap().contains("not undoable"));
    assert_eq!(read(&file), "small");
}

#[test]
fn undo_is_refused_while_a_turn_runs() {
    let env = env("undo-busy");
    let slow = tool_reply(&[("c1", "echo", r#"{"text":"one","sleepMs":300}"#)]);
    let service = Service::new(Arc::new(TestHost {
        home: env.home.clone(),
        transport: Arc::new(ScriptedTransport::new(vec![slow, text_reply("ok")])),
        gate: Arc::new(Gate::default()),
    }));
    let session = service.create(&env.cwd).unwrap();
    let running = watch(&service, &session, Start::Text("hi".into()));
    running.until(|e| matches!(e, AgentEvent::ToolCall { .. }));
    assert!(matches!(
        session.undo_last_turn(false),
        Err(ServiceError::Busy(_))
    ));
    running.end();
    let idle = session.undo_last_turn(false).unwrap();
    assert!(idle
        .refused
        .unwrap()
        .contains("no turn of this session changed files"));
    assert_eq!(
        (command("/undo"), command("/undo force")),
        (Some(false), Some(true))
    );
    assert_eq!((command("/undone"), command("undo")), (None, None));
}

/// Request bodies without `cache_control`.
fn bodies(transport: &ScriptedTransport) -> Vec<Value> {
    fn strip(value: &Value) -> Value {
        match value {
            Value::Object(map) => (map.iter())
                .filter(|(key, _)| key.as_str() != "cache_control")
                .map(|(key, item)| (key.clone(), strip(item)))
                .collect(),
            Value::Array(items) => (items.iter())
                .filter(|item| item["role"] != "system")
                .map(strip)
                .collect(),
            other => other.clone(),
        }
    }
    (transport.requests().iter())
        .map(|r| strip(&serde_json::from_str(&r.body).unwrap()))
        .collect()
}

#[test]
fn the_note_reaches_the_next_request_behind_an_unchanged_prefix() {
    let env = env("undo-note");
    let script = vec![
        write_call("c1", "n.txt", "x"),
        text_reply("ok"),
        text_reply("noted"),
    ];
    let (agent, transport) = agent(&env, script);
    let mut conv = Conversation::create(&env.home, &env.cwd, None).unwrap();
    turn(&agent, &mut conv, "make n");
    undo_conv(&mut conv, false).unwrap();
    turn(&agent, &mut conv, "go on");
    let sent = bodies(&transport);
    let (before, after) = (
        sent[1]["messages"].as_array().unwrap(),
        sent[2]["messages"].as_array().unwrap(),
    );
    assert_eq!(
        after[..before.len()],
        before[..],
        "earlier messages unchanged"
    );
    let last = after.last().unwrap()["content"].to_string();
    assert!(
        last.contains("[undo] The user undid the file changes made with write and edit in turn 1."),
        "{last}"
    );
    assert!(
        last.contains("Removed (that turn created them)") && last.contains("go on"),
        "{last}"
    );
    assert_eq!(sent[1]["system"], sent[2]["system"]);
    assert_eq!(sent[1]["tools"], sent[2]["tools"]);
}

fn trace_undos(conv: &Conversation) -> Vec<trace::record::Undo> {
    (trace::read(&conv.log_path).unwrap().1.iter())
        .filter_map(|e| match e.record() {
            Record::Undo(undo) => Some(undo),
            _ => None,
        })
        .collect()
}

#[test]
fn replay_after_reopen_shows_the_undo_and_a_second_undo_takes_the_earlier_turn() {
    let env = env("undo-reopen");
    let file = env.cwd.join("r.txt");
    let script = vec![
        write_call("c1", "r.txt", "v1"),
        text_reply("ok"),
        edit_call("c2", "r.txt", "v1", "v2"),
        text_reply("ok"),
    ];
    let (agent, _) = agent(&env, script);
    let mut conv = Conversation::create(&env.home, &env.cwd, None).unwrap();
    turn(&agent, &mut conv, "create");
    turn(&agent, &mut conv, "change");
    let first = undo_conv(&mut conv, false).unwrap();
    assert_eq!((first.turn, read(&file)), (Some(2), "v1".to_string()));
    let id = conv.id.clone();
    drop(conv);
    let (mut conv, events) = Conversation::reopen(&env.home, &env.cwd, &id).unwrap();
    let view = Transcript::opened(&events, &conv.messages, conv.take_notes());
    let shown = format!("{:?}", view.items);
    assert!(
        shown.contains("undid the file changes of turn 2: restored"),
        "{shown}"
    );
    assert_eq!(conv.tool_session.undo().last().map(|i| i.turn), Some(1));
    let second = undo_conv(&mut conv, false).unwrap();
    assert_eq!(second.turn, Some(1));
    assert!(!file.exists());
    assert_eq!(trace_undos(&conv).len(), 2);
    assert!(undo_conv(&mut conv, false).unwrap().refused.is_some());
}
