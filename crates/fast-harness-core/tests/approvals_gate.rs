//! The approval gate through the service (PLAN.md D64): ask, the three
//! answers, Stop, pause, steer and close during a wait, two sessions, the
//! refusal without an answering front end, and `off`.

mod agent_support;
mod service_support;

use std::path::{Path, PathBuf};
use std::sync::Arc;

use agent_support::{env, key_auth, parts, text_reply, tool_reply, Echo};
use fast_harness_core::agent::approval::{PAUSED, REFUSED};
use fast_harness_core::agent::{Agent, Answer, Conversation};
use fast_harness_core::config::approvals::{Allow, Approvals};
use fast_harness_core::config::Config;
use fast_harness_core::events::{AgentEvent, ApprovalRequest, TurnOutcome};
use fast_harness_core::http::{ScriptedResponse, ScriptedTransport};
use fast_harness_core::llm::ToolSpec;
use fast_harness_core::service::{Host, Service, Session, Start};
use fast_harness_core::tools::{self, Tool, ToolCtx, ToolOutput};
use serde_json::{json, Value};
use service_support::{watch, Client, Watch};

/// A Run tool such as an MCP tool.
struct Probe;

impl Tool for Probe {
    fn spec(&self) -> ToolSpec {
        ToolSpec::new("mcp__x__do", "Do", json!({"type": "object"}))
    }

    fn run(&self, _args: &Value, _ctx: &ToolCtx<'_>) -> ToolOutput {
        ToolOutput::ok("probed".to_string())
    }
}

/// One scripted transport per working directory.
struct AskHost {
    home: PathBuf,
    transports: Vec<(PathBuf, Arc<ScriptedTransport>)>,
    config: Config,
}

impl Host for AskHost {
    fn home(&self) -> PathBuf {
        self.home.clone()
    }

    fn config(&self, _cwd: &Path) -> Result<Config, String> {
        Ok(self.config.clone())
    }

    fn agent(
        &self,
        cwd: &Path,
        _: &Conversation,
        config: &Config,
    ) -> Result<(Agent, Vec<String>), String> {
        let transport = (self.transports.iter())
            .find(|(dir, _)| dir == cwd)
            .unwrap();
        let mut parts = parts(&transport.1, key_auth(), cwd);
        parts.config = config.clone();
        let mut registry = tools::builtin();
        registry.add(Box::new(Echo));
        registry.add(Box::new(Probe));
        Ok((Agent::new(parts, Arc::new(registry)), Vec::new()))
    }
}

fn ask(allow: Allow) -> Config {
    Config {
        approvals: Approvals::Ask,
        allow,
        ..Config::default()
    }
}

/// Loads every tool the tests call (a `find_tools` step never asks).
fn find() -> ScriptedResponse {
    let names = r#"{"names":["write","read","grep","glob","shell","mcp__x__do"]}"#;
    tool_reply(&[("f1", "find_tools", names)])
}

fn write(id: &str, path: &str) -> ScriptedResponse {
    let args = json!({"path": path, "content": "x"}).to_string();
    tool_reply(&[(id, "write", &args)])
}

/// A service over `scripts` (one working directory each) answering requests.
fn open(
    name: &str,
    config: Config,
    scripts: Vec<Vec<ScriptedResponse>>,
) -> (agent_support::Env, Service, Vec<Arc<Session>>) {
    let dir = env(name);
    let cwds: Vec<PathBuf> = (0..scripts.len())
        .map(|i| dir.cwd.join(format!("w{i}")))
        .collect();
    let transports = (cwds.iter().zip(scripts))
        .map(|(cwd, script)| {
            std::fs::create_dir_all(cwd).unwrap();
            (cwd.clone(), Arc::new(ScriptedTransport::new(script)))
        })
        .collect();
    let host = AskHost {
        home: dir.home.clone(),
        transports,
        config,
    };
    let service = Service::new(Arc::new(host));
    let sessions: Vec<Arc<Session>> = cwds
        .iter()
        .map(|cwd| service.create(cwd).unwrap())
        .collect();
    for session in &sessions {
        session.set_answering(true);
    }
    (dir, service, sessions)
}

fn request(turn: &Watch) -> ApprovalRequest {
    match turn.until(|e| matches!(e, AgentEvent::ApprovalRequest(_))) {
        AgentEvent::ApprovalRequest(request) => request,
        _ => unreachable!(),
    }
}

fn result_of(turn: &Watch, call: &str) -> String {
    match turn.until(|e| matches!(e, AgentEvent::ToolResult { id, .. } if id == call)) {
        AgentEvent::ToolResult { content, .. } => content,
        _ => unreachable!(),
    }
}

#[test]
fn ask_waits_and_each_answer_does_what_it_says() {
    let script = vec![
        find(),
        write("c1", "a.txt"),
        write("c2", "a.txt"),
        write("c3", "a.txt"),
        write("c4", "a.txt"),
        write("c5", "b.txt"),
        text_reply("done"),
    ];
    let (dir, service, sessions) = open("approvals-ask", ask(Allow::default()), vec![script]);
    let session = &sessions[0];
    let turn = watch(&service, session, Start::Text("go".into()));
    let first = request(&turn);
    assert_eq!(
        (first.call_id.as_str(), first.tool.as_str()),
        ("c1", "write")
    );
    assert_eq!(first.effect, tools::Effect::Write);
    assert!(
        first.summary.contains("a.txt") && first.agent.is_none(),
        "{first:?}"
    );
    assert_eq!(session.status().waiting, std::slice::from_ref(&first));
    assert!(session.answer(&first.id, Answer::Once));
    assert!(!session.answer(&first.id, Answer::Once), "answered already");
    assert!(!session.answer("nope", Answer::Once), "unknown id");
    result_of(&turn, "c1");
    let file = dir.cwd.join("w0").join("a.txt");
    assert!(file.is_file(), "yes once ran it");
    let second = request(&turn);
    assert_eq!(second.call_id, "c2", "the next write asks again");
    assert!(session.answer(&second.id, Answer::Once));
    let third = request(&turn);
    assert!(session.answer(&third.id, Answer::Session));
    result_of(&turn, "c4");
    let other = request(&turn);
    assert_eq!(
        other.call_id, "c5",
        "c4 ran under the session grant, b.txt asks"
    );
    assert!(session.answer(&other.id, Answer::No(Some("not now".into()))));
    let declined = result_of(&turn, "c5");
    assert!(
        declined.contains("declined") && declined.contains("not now"),
        "{declined}"
    );
    assert_eq!(turn.end().outcome, TurnOutcome::Done, "the next step ran");
    assert!(!dir.cwd.join("w0").join("b.txt").exists());
    assert!(session.status().waiting.is_empty());
}

#[test]
fn reads_never_ask_and_allowed_calls_run_without_a_request() {
    let reads = tool_reply(&[
        ("r1", "read", r#"{"path":"a.txt"}"#),
        ("r2", "grep", r#"{"pattern":"x"}"#),
        ("r3", "glob", r#"{"pattern":"*"}"#),
    ]);
    let allowed = tool_reply(&[
        ("p1", "mcp__x__do", "{}"),
        ("w1", "write", r#"{"path":"ok/a.txt","content":"x"}"#),
    ]);
    let allow = Allow {
        paths: vec!["ok".into()],
        tools: vec!["mcp__x__do".into()],
        ..Allow::default()
    };
    let script = vec![find(), reads, allowed, text_reply("done")];
    let (dir, service, sessions) = open("approvals-reads", ask(allow), vec![script]);
    let turn = watch(&service, &sessions[0], Start::Text("go".into()));
    let mut asked = 0;
    loop {
        match turn
            .events
            .recv_timeout(std::time::Duration::from_secs(10))
            .unwrap()
        {
            AgentEvent::ApprovalRequest(_) => asked += 1,
            AgentEvent::TurnEnd { .. } => break,
            _ => {}
        }
    }
    assert_eq!(asked, 0);
    assert_eq!(turn.end().outcome, TurnOutcome::Done);
    assert!(dir.cwd.join("w0").join("ok").join("a.txt").is_file());
}

#[test]
fn stop_during_a_wait_cancels_the_call() {
    let script = vec![find(), write("c1", "a.txt"), text_reply("never")];
    let (dir, service, sessions) = open("approvals-stop", ask(Allow::default()), vec![script]);
    let turn = watch(&service, &sessions[0], Start::Text("go".into()));
    request(&turn);
    assert!(sessions[0].cancel());
    let done = turn.until(|e| matches!(e, AgentEvent::ApprovalDone { .. }));
    assert!(matches!(done, AgentEvent::ApprovalDone { answer, .. } if answer == "cancelled"));
    assert_eq!(result_of(&turn, "c1"), "cancelled before it ran");
    assert_eq!(turn.end().outcome, TurnOutcome::Cancelled);
    assert!(!dir.cwd.join("w0").join("a.txt").exists());
}

#[test]
fn pause_and_steer_during_a_wait() {
    let script = vec![find(), write("c1", "a.txt"), write("c2", "b.txt")];
    let (_dir, service, sessions) = open("approvals-pause", ask(Allow::default()), vec![script]);
    let session = &sessions[0];
    let turn = watch(&service, session, Start::Text("go".into()));
    let first = request(&turn);
    assert!(session.steer("use b".into()), "a steer goes to the inbox");
    assert_eq!(session.status().waiting.len(), 1, "and does not answer");
    assert!(session.answer(&first.id, Answer::Once));
    result_of(&turn, "c1");
    turn.until(|e| matches!(e, AgentEvent::UserInput { steer: true, .. }));
    request(&turn);
    assert!(session.pause());
    assert_eq!(result_of(&turn, "c2"), PAUSED);
    assert_eq!(turn.end().outcome, TurnOutcome::Paused);
}

#[test]
fn closing_the_session_ends_the_wait() {
    let script = vec![find(), write("c1", "a.txt")];
    let (dir, service, sessions) = open("approvals-close", ask(Allow::default()), vec![script]);
    let turn = watch(&service, &sessions[0], Start::Text("go".into()));
    request(&turn);
    assert!(service.close(&sessions[0].id).is_some());
    assert_eq!(turn.end().outcome, TurnOutcome::Cancelled);
    assert!(!dir.cwd.join("w0").join("a.txt").exists());
}

#[test]
fn two_sessions_waiting_at_once_answer_independently() {
    let script = || vec![find(), write("c1", "a.txt"), text_reply("done")];
    let (dir, service, sessions) = open(
        "approvals-two",
        ask(Allow::default()),
        vec![script(), script()],
    );
    let one = watch(&service, &sessions[0], Start::Text("go".into()));
    let two = watch(&service, &sessions[1], Start::Text("go".into()));
    let (a, b) = (request(&one), request(&two));
    assert_eq!(sessions[0].status().waiting, std::slice::from_ref(&a));
    assert_eq!(sessions[1].status().waiting, std::slice::from_ref(&b));
    assert!(sessions[1].answer(&b.id, Answer::No(None)));
    assert_eq!(two.end().outcome, TurnOutcome::Done);
    assert_eq!(
        sessions[0].status().waiting.len(),
        1,
        "the other still waits"
    );
    assert!(sessions[0].answer(&a.id, Answer::Once));
    assert_eq!(one.end().outcome, TurnOutcome::Done);
    assert!(dir.cwd.join("w0").join("a.txt").is_file());
    assert!(!dir.cwd.join("w1").join("a.txt").exists());
}

#[test]
fn without_an_answering_front_end_a_call_is_refused_at_once() {
    let script = vec![find(), write("c1", "a.txt"), text_reply("ok")];
    let (dir, service, sessions) = open("approvals-refuse", ask(Allow::default()), vec![script]);
    sessions[0].set_answering(false);
    let turn = watch(&service, &sessions[0], Start::Text("go".into()));
    let done = turn.until(|e| matches!(e, AgentEvent::ApprovalDone { .. }));
    assert!(matches!(done, AgentEvent::ApprovalDone { answer, .. } if answer == "refused"));
    assert_eq!(result_of(&turn, "c1"), REFUSED);
    assert_eq!(turn.end().outcome, TurnOutcome::Done);
    assert!(!dir.cwd.join("w0").join("a.txt").exists());
}

#[test]
fn off_asks_nothing() {
    let script = vec![
        find(),
        write("c1", "a.txt"),
        tool_reply(&[("p1", "mcp__x__do", "{}")]),
        text_reply("ok"),
    ];
    let (dir, service, sessions) = open("approvals-off", Config::default(), vec![script]);
    let turn = watch(&service, &sessions[0], Start::Text("go".into()));
    let mut asked = 0;
    loop {
        match turn
            .events
            .recv_timeout(std::time::Duration::from_secs(10))
            .unwrap()
        {
            AgentEvent::ApprovalRequest(_) | AgentEvent::ApprovalDone { .. } => asked += 1,
            AgentEvent::TurnEnd { .. } => break,
            _ => {}
        }
    }
    assert_eq!(asked, 0);
    assert_eq!(turn.end().outcome, TurnOutcome::Done);
    assert!(dir.cwd.join("w0").join("a.txt").is_file());
}

#[test]
fn serve_asks_the_client_and_takes_its_answer() {
    let dir = env("approvals-serve");
    let cwd = dir.cwd.join("w0");
    std::fs::create_dir_all(&cwd).unwrap();
    let script = vec![
        find(),
        write("c1", "a.txt"),
        write("c2", "b.txt"),
        text_reply("done"),
    ];
    let transports = vec![(cwd.clone(), Arc::new(ScriptedTransport::new(script)))];
    let host = AskHost {
        home: dir.home.clone(),
        transports,
        config: ask(Allow::default()),
    };
    let mut client = Client::start(Arc::new(host));
    let sid = client.call("session/new", json!({"cwd": cwd}))["sessionId"]
        .as_str()
        .unwrap()
        .to_string();
    client.request(
        "session/prompt",
        json!({"sessionId": sid, "prompt": [{"type": "text", "text": "go"}]}),
    );
    let first = client.wait(|v| v["method"] == "session/request_permission");
    assert_eq!(first["params"]["subject"]["toolCall"]["toolCallId"], "c1");
    let allow = json!({"outcome": {"outcome": "selected", "optionId": "allow-once"}});
    client.raw(&json!({"jsonrpc": "2.0", "id": first["id"], "result": allow}).to_string());
    let second =
        client.wait(|v| v["method"] == "session/request_permission" && v["id"] != first["id"]);
    let request_id = second["params"]["_meta"]["fh/requestId"].clone();
    let unknown = client.request(
        "_fh/approve",
        json!({"sessionId": sid, "requestId": "nope", "answer": "once"}),
    );
    assert!(
        client.response(unknown).get("error").is_some(),
        "an unknown id"
    );
    let no = json!({"sessionId": sid, "requestId": request_id, "answer": "no", "reason": "later"});
    client.call("_fh/approve", no.clone());
    let again = client.request("_fh/approve", no);
    assert!(
        client.response(again).get("error").is_some(),
        "answered already"
    );
    client.idle(&sid, "end_turn");
    assert!(cwd.join("a.txt").is_file() && !cwd.join("b.txt").exists());
    client.finish();
}
