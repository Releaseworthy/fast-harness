//! Session coordination without a daemon (PLAN.md D49): `<id>.lock` while a
//! turn runs, `<id>.inbox.jsonl` read at step boundaries, `send` that runs
//! an idle session's turn itself, `wait` and the `ps` listing.

mod agent_support;
mod service_support;

use std::fs;
use std::sync::atomic::AtomicBool;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use agent_support::{echo_registry, env, key_auth, parts, text_reply, tool_reply, Env};
use fast_harness_core::agent::{Agent, Conversation, Inbox};
use fast_harness_core::events::{AgentEvent, TurnOutcome};
use fast_harness_core::http::ScriptedTransport;
use fast_harness_core::service::{Mode, Sent, Service};
use fast_harness_core::session::coord::{self, Command, FileInbox};
use fast_harness_core::session::lock::{self, Held, Lock};
use serde_json::Value;
use service_support::{Gate, TestHost};

const CALL: &str = r#"{"text":"hi"}"#;

fn agent(transport: &Arc<ScriptedTransport>, env: &Env) -> Agent {
    Agent::new(parts(transport, key_auth(), &env.cwd), echo_registry())
}

#[test]
fn a_lock_is_live_listed_and_removed_on_drop() {
    let env = env("coord-lock");
    let conv = Conversation::create(&env.home, &env.cwd, None).unwrap();
    let cwd = env.cwd.to_string_lossy();
    let mut held = Held::take(&conv.log_path, &conv.id, &cwd, 1).unwrap();
    held.update("running", 1, 3);
    let path = coord::lock_path(&conv.log_path);
    let lock = lock::live_lock(&conv.log_path).expect("own process is live");
    assert_eq!(
        (lock.pid, lock.step, lock.id.as_str()),
        (std::process::id(), 3, conv.id.as_str())
    );
    // `ps` lists it.
    let running = lock::running(&env.home);
    assert_eq!(running.len(), 1);
    assert_eq!(running[0].lock.id, conv.id);
    assert_eq!(
        lock::find(&env.home, &conv.id).as_deref(),
        Some(conv.log_path.as_path())
    );
    drop(held);
    assert!(!path.exists());
    assert!(lock::running(&env.home).is_empty());
}

#[test]
fn a_stale_lock_is_removed_by_the_reader() {
    let env = env("coord-stale");
    let conv = Conversation::create(&env.home, &env.cwd, None).unwrap();
    let held = Held::take(&conv.log_path, &conv.id, "x", 1).unwrap();
    // The same pid with another start time: a reused pid, so stale.
    let stale = Lock {
        started: held.lock().started + 1,
        ..held.lock().clone()
    };
    std::mem::forget(held);
    let path = coord::lock_path(&conv.log_path);
    fs::write(&path, serde_json::to_string(&stale).unwrap()).unwrap();
    assert!(lock::live_lock(&conv.log_path).is_none());
    assert!(!path.exists());
    assert!(lock::running(&env.home).is_empty());
}

#[test]
fn the_file_inbox_reads_whole_new_lines_only() {
    let env = env("coord-inbox");
    let path = env.home.join("x.inbox.jsonl");
    let mut inbox = FileInbox::new(path.clone());
    assert!(inbox.poll().is_empty());
    coord::append(&path, &Command::Steer("one".into())).unwrap();
    fs::OpenOptions::new()
        .append(true)
        .open(&path)
        .and_then(|mut f| std::io::Write::write_all(&mut f, b"{\"type\":\"pa"))
        .unwrap();
    assert_eq!(inbox.poll(), vec![Command::Steer("one".into())]);
    fs::OpenOptions::new()
        .append(true)
        .open(&path)
        .and_then(|mut f| std::io::Write::write_all(&mut f, b"use\"}\nnot json\n"))
        .unwrap();
    assert_eq!(inbox.poll(), vec![Command::Pause]);
    assert!(inbox.poll().is_empty());
    assert!(coord::is_log(&env.home.join("a.jsonl")) && !coord::is_log(&path));
}

/// Runs a turn whose sink appends `command` to the session's inbox file at
/// the first tool call, and checks that the lock exists while it runs.
fn run_with_file(
    env: &Env,
    replies: usize,
    command: Command,
) -> (
    TurnOutcome,
    Vec<AgentEvent>,
    Arc<ScriptedTransport>,
    Conversation,
) {
    let mut scripted = vec![tool_reply(&[("toolu_1", "echo", CALL)]), text_reply("ok")];
    scripted.truncate(replies);
    let transport = Arc::new(ScriptedTransport::new(scripted));
    let mut conv = Conversation::create(&env.home, &env.cwd, None).unwrap();
    let (log, inbox_path) = (conv.log_path.clone(), coord::inbox_path(&conv.log_path));
    let mut pending = Some(command);
    let mut seen = Vec::new();
    let cancel = AtomicBool::new(false);
    let outcome = agent(&transport, env).run_turn_with(
        &mut conv,
        "go",
        &cancel,
        &mut Inbox::empty(),
        &mut |event| {
            if matches!(event, AgentEvent::ToolCall { .. }) {
                assert!(lock::live_lock(&log).is_some(), "locked while running");
                if let Some(command) = pending.take() {
                    coord::append(&inbox_path, &command).unwrap();
                }
            }
            seen.push(event);
        },
    );
    assert!(!coord::lock_path(&log).exists(), "lock removed at the end");
    assert!(!inbox_path.exists(), "inbox removed at the end");
    (outcome, seen, transport, conv)
}

#[test]
fn a_steer_line_lands_after_the_tool_results() {
    let env = env("coord-steer");
    let (outcome, seen, transport, _) = run_with_file(&env, 2, Command::Steer("from afar".into()));
    assert_eq!(outcome, TurnOutcome::Done);
    let steered = AgentEvent::UserInput {
        text: "from afar".into(),
        steer: true,
    };
    assert!(seen.contains(&steered));
    let requests = transport.requests();
    let second: Value = serde_json::from_str(&requests[1].body).unwrap();
    // The last user message (a trailing `role: system` effort entry may follow).
    let messages = second["messages"].as_array().unwrap();
    let last = &messages.iter().rev().find(|m| m["role"] == "user").unwrap()["content"];
    assert_eq!(last[0]["type"], "tool_result");
    assert_eq!(last[1]["text"], "from afar");
}

#[test]
fn a_pause_line_ends_the_turn_after_its_tools() {
    let env = env("coord-pause");
    let (outcome, _, transport, _) = run_with_file(&env, 1, Command::Pause);
    assert_eq!(outcome, TurnOutcome::Paused);
    assert_eq!(transport.requests().len(), 1);
}

#[test]
fn a_stop_line_cancels_at_the_step_boundary() {
    let env = env("coord-stop");
    let (outcome, _, transport, _) = run_with_file(&env, 1, Command::Stop);
    assert_eq!(outcome, TurnOutcome::Cancelled);
    assert_eq!(transport.requests().len(), 1);
}

fn service(env: &Env, replies: Vec<fast_harness_core::http::ScriptedResponse>) -> Service {
    let host = TestHost {
        home: env.home.clone(),
        transport: Arc::new(ScriptedTransport::new(replies)),
        gate: Arc::new(Gate::default()),
    };
    Service::new(Arc::new(host))
}

#[test]
fn send_to_an_idle_session_runs_the_turn_here() {
    let env = env("coord-send-idle");
    let id = Conversation::create(&env.home, &env.cwd, None).unwrap().id;
    let service = service(&env, vec![text_reply("answered")]);
    let text = Arc::new(Mutex::new(String::new()));
    let seen = Arc::clone(&text);
    let sink = Box::new(move |event: AgentEvent| {
        if let AgentEvent::TextDelta(delta) = event {
            seen.lock().unwrap().push_str(&delta);
        }
    });
    match service.send_to(&id, "hello", Mode::Steer, sink).unwrap() {
        Sent::Ran(finished) => assert_eq!(finished.outcome, TurnOutcome::Done),
        other => panic!("expected a run, got {other:?}"),
    }
    assert_eq!(*text.lock().unwrap(), "answered");
    let log = lock::find(&env.home, &id).unwrap();
    assert!(!coord::lock_path(&log).exists());
}

#[test]
fn send_to_a_running_session_appends_to_its_inbox() {
    let env = env("coord-send-live");
    let conv = Conversation::create(&env.home, &env.cwd, None).unwrap();
    let held = Held::take(&conv.log_path, &conv.id, "x", 1).unwrap();
    let service = service(&env, Vec::new());
    let sent = service
        .send_to(&conv.id, "later", Mode::Queue, Box::new(|_| {}))
        .unwrap();
    assert!(matches!(sent, Sent::Queued { pid, .. } if pid == std::process::id()));
    let mut inbox = FileInbox::new(coord::inbox_path(&conv.log_path));
    assert_eq!(inbox.poll(), vec![Command::FollowUp("later".into())]);
    drop(held);
}

/// A process that is not this one and outlives the test's checks.
fn other_process() -> std::process::Child {
    let mut command = if cfg!(windows) {
        let mut ping = std::process::Command::new("ping");
        ping.args(["-n", "30", "127.0.0.1"]);
        ping
    } else {
        let mut sleep = std::process::Command::new("sleep");
        sleep.arg("30");
        sleep
    };
    let quiet = std::process::Stdio::null;
    command.stdout(quiet()).stderr(quiet()).spawn().unwrap()
}

/// Two processes never write one session log: a turn is refused while the
/// live lock of another process is there, and runs once that process is gone.
#[test]
fn a_turn_is_refused_while_another_process_runs_the_session() {
    let env = env("coord-two-writers");
    let transport = Arc::new(ScriptedTransport::new(vec![text_reply("late")]));
    let mut conv = Conversation::create(&env.home, &env.cwd, None).unwrap();
    let mut other = other_process();
    let end = |other: &mut std::process::Child| {
        let _ = other.kill();
        let _ = other.wait();
    };
    let Some(started) = fast_harness_core::platform::process_start(other.id()) else {
        // No process start time on this platform: no lock is ever live.
        return end(&mut other);
    };
    let held = Held::take(&conv.log_path, &conv.id, "x", 1).unwrap();
    let theirs = Lock {
        pid: other.id(),
        started,
        program: "fast-harness-gui.exe".into(),
        ..held.lock().clone()
    };
    std::mem::forget(held);
    let path = coord::lock_path(&conv.log_path);
    fs::write(&path, serde_json::to_string(&theirs).unwrap()).unwrap();
    let before = fs::read(&conv.log_path).unwrap();
    let mut errors = Vec::new();
    let turn = |conv: &mut Conversation, errors: &mut Vec<String>| {
        let mut sink = |event: AgentEvent| {
            if let AgentEvent::Error(text) = event {
                errors.push(text);
            }
        };
        agent(&transport, &env).run_turn(conv, "go", &AtomicBool::new(false), &mut sink)
    };
    assert_eq!(turn(&mut conv, &mut errors), TurnOutcome::Failed);
    assert!(
        errors[0].contains("is running in fast-harness-gui.exe"),
        "{errors:?}"
    );
    assert!(transport.requests().is_empty());
    assert_eq!(fs::read(&conv.log_path).unwrap(), before, "nothing logged");
    assert!(path.exists(), "the other process keeps its lock");
    end(&mut other);
    assert_eq!(turn(&mut conv, &mut errors), TurnOutcome::Done);
    assert_eq!(errors.len(), 1);
}

#[test]
fn resume_refuses_an_id_that_is_a_path() {
    let env = env("coord-resume-id");
    for id in ["..\\..\\accounts", "../x", "a.b", ""] {
        let err = Conversation::resume(&env.home, &env.cwd, id).err();
        let kind = err.map(|err| err.kind());
        assert_eq!(kind, Some(std::io::ErrorKind::InvalidInput), "{id}");
    }
}

#[test]
fn wait_returns_when_the_lock_goes() {
    let env = env("coord-wait");
    let conv = Conversation::create(&env.home, &env.cwd, None).unwrap();
    let every = Duration::from_millis(20);
    assert!(lock::wait(&conv.log_path, every, None));
    let held = Held::take(&conv.log_path, &conv.id, "x", 1).unwrap();
    assert!(!lock::wait(
        &conv.log_path,
        every,
        Some(Duration::from_millis(60))
    ));
    let dropper = std::thread::spawn(move || {
        std::thread::sleep(Duration::from_millis(100));
        drop(held);
    });
    assert!(lock::wait(
        &conv.log_path,
        every,
        Some(Duration::from_secs(10))
    ));
    dropper.join().unwrap();
}
