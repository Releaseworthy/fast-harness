//! The model's session tools (PLAN.md D49) over a test home, with a
//! launcher that records workers instead of spawning them.

mod agent_support;

use std::sync::atomic::AtomicBool;
use std::sync::{Arc, Mutex};

use agent_support::{env, key_auth, parts, text_reply, Env};
use fast_harness_core::agent::session_tools::{self, LIST, READ, SEND, START, STOP};
use fast_harness_core::agent::worker::{Launch, Launcher};
use fast_harness_core::agent::{Agent, Conversation};
use fast_harness_core::http::ScriptedTransport;
use fast_harness_core::session::coord::{self, Command, FileInbox};
use fast_harness_core::session::lock::Held;
use fast_harness_core::tools::{Registry, Tool, ToolCtx, ToolOutput};
use serde_json::{json, Value};

struct Tools {
    tools: Vec<Box<dyn Tool>>,
    launched: Arc<Mutex<Vec<Launch>>>,
}

fn tools(env: &Env) -> Tools {
    let launched = Arc::new(Mutex::new(Vec::new()));
    let record = Arc::clone(&launched);
    let launcher: Launcher = Arc::new(move |launch: &Launch| {
        record.lock().unwrap().push(launch.clone());
        Ok(4242)
    });
    let tools = session_tools::all(&env.home, launcher);
    Tools { tools, launched }
}

impl Tools {
    fn run(&self, name: &str, args: Value, env: &Env) -> ToolOutput {
        let tool = (self.tools.iter()).find(|t| t.spec().name == name).unwrap();
        let cancel = AtomicBool::new(false);
        tool.run(&args, &ToolCtx::new(&env.cwd, &cancel))
    }
}

fn flag_value<'a>(launch: &'a Launch, flag: &str) -> &'a str {
    let at = launch.args.iter().position(|a| a == flag).unwrap();
    &launch.args[at + 1]
}

#[test]
fn the_worker_command_runs_print_mode_on_the_session() {
    let exe = std::path::Path::new("C:/bin/fast-harness.exe");
    let cwd = std::path::Path::new("C:/work");
    let launch = Launch::worker(exe, "20261112-000000-abcd", cwd, "-do it", Some("m1"));
    let want = [
        "-p",
        "-do it",
        "--resume",
        "20261112-000000-abcd",
        "--cwd",
        "C:/work",
        "--model",
        "m1",
    ];
    assert_eq!(launch.args, want);
    let cmd = launch.command();
    assert_eq!(cmd.get_program(), exe.as_os_str());
    assert_eq!(cmd.get_current_dir(), Some(cwd));
    let bare = Launch::worker(exe, "x", cwd, "p", None);
    assert!(!bare.args.contains(&"--model".to_string()));
}

#[test]
fn the_five_tools_exist_and_are_deferred() {
    let env = env("tools-names");
    let set = tools(&env);
    let names: Vec<String> = set.tools.iter().map(|t| t.spec().name).collect();
    assert_eq!(names, [LIST, START, SEND, READ, STOP]);
    // A top-level agent carries them behind `find_tools` (deferred, D30).
    let transport = Arc::new(ScriptedTransport::new(Vec::new()));
    let parts = parts(&transport, key_auth(), &env.cwd);
    let agent = Agent::with_subagent(parts, &env.home, "parent");
    let specs: Vec<String> = agent.tools.specs().iter().map(|s| s.name.clone()).collect();
    assert_eq!(specs.first().map(String::as_str), Some("find_tools"));
    for name in [LIST, START, SEND, READ, STOP] {
        assert!(specs.iter().any(|s| s == name), "{name} missing: {specs:?}");
    }
}

#[test]
fn session_start_creates_a_session_and_launches_a_worker() {
    let env = env("tools-start");
    let set = tools(&env);
    let out = set.run(START, json!({"prompt": "build it", "model": "m2"}), &env);
    assert!(!out.is_error(), "{}", out.text());
    let launched = set.launched.lock().unwrap();
    assert_eq!(launched.len(), 1);
    let id = flag_value(&launched[0], "--resume");
    assert!(out.text().contains(id) && out.text().contains("4242"));
    assert_eq!(flag_value(&launched[0], "-p"), "build it");
    assert_eq!(flag_value(&launched[0], "--model"), "m2");
    let listed = set.run(LIST, json!({}), &env).into_text();
    assert!(listed.contains(id), "{listed}");
}

#[test]
fn send_and_stop_reach_a_running_session_through_its_inbox() {
    let env = env("tools-send");
    let set = tools(&env);
    let conv = Conversation::create(&env.home, &env.cwd, None).unwrap();
    let held = Held::take(&conv.log_path, &conv.id, &env.cwd.to_string_lossy(), 1).unwrap();
    let listed = set.run(LIST, json!({}), &env).into_text();
    assert!(
        listed.contains(&format!("{}  running", conv.id)),
        "{listed}"
    );
    let sent = set.run(
        SEND,
        json!({"id": conv.id, "text": "look", "mode": "queue"}),
        &env,
    );
    assert!(!sent.is_error(), "{}", sent.text());
    let stop = set.run(STOP, json!({"id": conv.id}), &env);
    assert!(!stop.is_error());
    let mut inbox = FileInbox::new(coord::inbox_path(&conv.log_path));
    assert_eq!(
        inbox.poll(),
        vec![Command::FollowUp("look".into()), Command::Stop]
    );
    assert!(set.launched.lock().unwrap().is_empty());
    drop(held);
    let idle_stop = set.run(STOP, json!({"id": conv.id}), &env);
    assert!(idle_stop.text().contains("not running"));
}

#[test]
fn send_to_an_idle_session_starts_a_worker() {
    let env = env("tools-send-idle");
    let set = tools(&env);
    let conv = Conversation::create(&env.home, &env.cwd, None).unwrap();
    let out = set.run(SEND, json!({"id": conv.id, "text": "go on"}), &env);
    assert!(out.text().contains("was idle"), "{}", out.text());
    let launched = set.launched.lock().unwrap();
    assert_eq!(flag_value(&launched[0], "--resume"), conv.id);
    assert_eq!(flag_value(&launched[0], "-p"), "go on");
    let missing = set.run(SEND, json!({"id": "nope", "text": "x"}), &env);
    assert!(missing.is_error());
}

#[test]
fn session_read_returns_the_last_answer() {
    let env = env("tools-read");
    let set = tools(&env);
    let transport = Arc::new(ScriptedTransport::new(vec![text_reply("the answer")]));
    let agent = Agent::new(
        parts(&transport, key_auth(), &env.cwd),
        Arc::new(Registry::new()),
    );
    let mut conv = Conversation::create(&env.home, &env.cwd, None).unwrap();
    let before = set.run(READ, json!({"id": conv.id}), &env).into_text();
    assert!(before.contains("no answer yet"), "{before}");
    agent.run_turn(&mut conv, "q", &AtomicBool::new(false), &mut |_| {});
    let read = set.run(READ, json!({"id": conv.id}), &env).into_text();
    assert!(
        read.contains("is idle") && read.ends_with("the answer"),
        "{read}"
    );
}

/// A real detached spawn (Windows: DETACHED_PROCESS | CREATE_NEW_PROCESS_GROUP).
#[cfg(windows)]
#[test]
fn spawn_detached_runs_a_process_with_null_stdio() {
    let env = env("tools-spawn");
    let out = env.home.join("spawned.txt");
    let mut cmd = std::process::Command::new("cmd");
    cmd.args(["/c", "echo ok>"]).arg(&out);
    let mut child = fast_harness_core::platform::spawn_detached(&mut cmd).unwrap();
    assert!(child.wait().unwrap().success());
    assert_eq!(std::fs::read_to_string(&out).unwrap().trim(), "ok");
    assert!(fast_harness_core::platform::process_start(std::process::id()).is_some());
    assert!(fast_harness_core::platform::process_start(child.id()).is_none());
}
