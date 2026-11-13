//! `ask_user_question` through the service (PLAN.md D81): answers flow
//! back, multi select, the custom answer, skip, cancel, the refusal when
//! nobody can answer, approvals never gate it, and the tool reaches the
//! request body only after `find_tools` loads it.

mod agent_support;
mod service_support;

use std::path::{Path, PathBuf};
use std::sync::Arc;

use agent_support::{env, key_auth, parts, text_reply, tool_reply};
use fast_harness_core::agent::{Agent, Conversation};
use fast_harness_core::config::approvals::Approvals;
use fast_harness_core::config::Config;
use fast_harness_core::events::{AgentEvent, QuestionAnswer, QuestionRequest, TurnOutcome};
use fast_harness_core::http::{ScriptedResponse, ScriptedTransport};
use fast_harness_core::service::{Host, Service, Session, Start};
use fast_harness_core::tools::{self, ask};
use serde_json::{json, Value};
use service_support::{watch, Watch};

struct OneHost {
    home: PathBuf,
    transport: Arc<ScriptedTransport>,
    config: Config,
}

impl Host for OneHost {
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
        c: &Config,
    ) -> Result<(Agent, Vec<String>), String> {
        let mut parts = parts(&self.transport, key_auth(), cwd);
        parts.config = c.clone();
        Ok((Agent::new(parts, Arc::new(tools::builtin())), Vec::new()))
    }
}

fn questions() -> String {
    json!({"questions": [
        {"id": "db", "header": "Database", "question": "Which database?",
            "options": [{"label": "MySQL"}, {"label": "Postgres (Recommended)"}]},
        {"id": "os", "question": "Which systems?", "multi_select": true,
            "options": [{"label": "Windows"}, {"label": "Linux"}, {"label": "macOS"}]},
        {"id": "name", "question": "Project name?"}]})
    .to_string()
}

fn script() -> Vec<ScriptedResponse> {
    let find = tool_reply(&[("f1", "find_tools", r#"{"names":["ask_user_question"]}"#)]);
    let call = tool_reply(&[("a1", ask::NAME, &questions())]);
    vec![find, call, text_reply("done")]
}

type Opened = (
    agent_support::Env,
    Service,
    Arc<Session>,
    Arc<ScriptedTransport>,
);

fn open(name: &str, approvals: Approvals, asking: bool) -> Opened {
    let dir = env(name);
    let transport = Arc::new(ScriptedTransport::new(script()));
    let config = Config {
        approvals,
        ..Config::default()
    };
    let host = OneHost {
        home: dir.home.clone(),
        transport: Arc::clone(&transport),
        config,
    };
    let service = Service::new(Arc::new(host));
    let session = service.create(&dir.cwd).unwrap();
    session.set_answering(true);
    session.set_asking(asking);
    (dir, service, session, transport)
}

fn request(turn: &Watch) -> QuestionRequest {
    match turn.until(|e| matches!(e, AgentEvent::QuestionRequest(_))) {
        AgentEvent::QuestionRequest(request) => request,
        _ => unreachable!(),
    }
}

fn result_of(turn: &Watch) -> (String, bool) {
    match turn.until(|e| matches!(e, AgentEvent::ToolResult { id, .. } if id == "a1")) {
        AgentEvent::ToolResult {
            content, is_error, ..
        } => (content, is_error),
        _ => unreachable!(),
    }
}

fn done_word(turn: &Watch) -> String {
    match turn.until(|e| matches!(e, AgentEvent::QuestionDone { .. })) {
        AgentEvent::QuestionDone { answer, .. } => answer,
        _ => unreachable!(),
    }
}

#[test]
fn answers_flow_back_with_multi_select_and_custom_text() {
    // Approvals on: the question call never asks for approval.
    let (_dir, service, session, _) = open("ask-answers", Approvals::Ask, true);
    let turn = watch(&service, &session, Start::Text("go".into()));
    let asked = request(&turn);
    assert_eq!(asked.call_id, "a1");
    assert_eq!(asked.questions.len(), 3);
    assert_eq!(asked.questions[0].options[0].label, "Postgres");
    assert!(asked.questions[0].options[0].recommended);
    assert!(asked.questions[1].multi_select);
    assert_eq!(session.questions(), vec![asked.clone()]);
    assert!(session.approvals().is_empty(), "no approval request");
    let answers = vec![
        QuestionAnswer::new("db", vec!["Postgres".into()], None),
        QuestionAnswer::new("os", vec!["Windows".into(), "Linux".into()], None),
        QuestionAnswer::new("name", Vec::new(), Some("Orbit".into())),
    ];
    assert!(session.reply(&asked.id, Some(answers)));
    assert!(!session.reply(&asked.id, None), "answered already");
    assert_eq!(done_word(&turn), "answered");
    let (text, is_error) = result_of(&turn);
    assert!(!is_error);
    assert!(
        text.contains("- db (Which database?): \"Postgres\""),
        "{text}"
    );
    assert!(text.contains("\"Windows\", \"Linux\""), "{text}");
    assert!(text.contains("custom answer: \"Orbit\""), "{text}");
    assert_eq!(turn.end().outcome, TurnOutcome::Done);
    assert!(session.questions().is_empty());
}

#[test]
fn skip_tells_the_model_to_decide() {
    let (_dir, service, session, _) = open("ask-skip", Approvals::Off, true);
    let turn = watch(&service, &session, Start::Text("go".into()));
    let asked = request(&turn);
    assert!(session.reply(&asked.id, None));
    assert_eq!(done_word(&turn), "skipped");
    assert_eq!(result_of(&turn).0, ask::SKIPPED);
    assert_eq!(turn.end().outcome, TurnOutcome::Done);
}

#[test]
fn cancel_ends_the_wait_with_an_error() {
    let (_dir, service, session, _) = open("ask-cancel", Approvals::Off, true);
    let turn = watch(&service, &session, Start::Text("go".into()));
    request(&turn);
    assert!(session.cancel());
    assert_eq!(done_word(&turn), "cancelled");
    let (text, is_error) = result_of(&turn);
    assert!(is_error && text.contains("cancelled"), "{text}");
    assert_eq!(turn.end().outcome, TurnOutcome::Cancelled);
    assert!(session.questions().is_empty());
}

/// The `ask_user_question` entry of a body's tools.
fn ask_spec(body: &str) -> Value {
    let body: Value = serde_json::from_str(body).unwrap();
    let tools = body["tools"].as_array().cloned().unwrap_or_default();
    let found = tools.iter().find(|t| t["name"] == ask::NAME);
    found.cloned().unwrap_or(Value::Null)
}

/// True when the body's messages load the tool (a find result's reference).
fn loads_ask(body: &str) -> bool {
    let body: Value = serde_json::from_str(body).unwrap();
    let text = body["messages"].to_string();
    text.contains(r#""tool_name":"ask_user_question""#)
}
#[test]
fn nobody_answering_refuses_at_once_and_the_body_loads_the_tool_late() {
    let (_dir, service, session, transport) = open("ask-refuse", Approvals::Off, false);
    let turn = watch(&service, &session, Start::Text("go".into()));
    let (text, is_error) = result_of(&turn);
    assert!(is_error);
    assert_eq!(text, ask::NO_USER);
    assert_eq!(turn.end().outcome, TurnOutcome::Done);
    let bodies: Vec<String> = transport.requests().into_iter().map(|r| r.body).collect();
    assert_eq!(bodies.len(), 3);
    for body in &bodies {
        let spec = ask_spec(body);
        assert_eq!(spec["defer_loading"], json!(true), "{spec}");
        assert!(spec["input_schema"]["properties"]["questions"].is_object());
    }
    assert!(!loads_ask(&bodies[0]), "not loaded before the find");
    assert!(
        loads_ask(&bodies[1]) && loads_ask(&bodies[2]),
        "{}",
        bodies[1]
    );
}
