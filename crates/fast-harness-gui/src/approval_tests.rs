//! The approval prompt on real turns (PLAN.md D64): what the prompt shows,
//! the answers, Stop, a parked session, and `approvals` off. Slint focus
//! (that the prompt never takes it from the composer) is not visible here:
//! the real window pass covers it; these tests cover the key table.

use super::*;
use crate::approval::{answer, answer_of, view};
use crate::rows::msg_row_with;
use fast_harness_core::agent::Answer;
use fast_harness_core::config::approvals::Approvals;
use fast_harness_core::config::Config;
use fast_harness_core::events::ApprovalRequest;
use fast_harness_core::keys::{Front, Keymap};
use fast_harness_core::tools;
use slint::platform::Key as SlintKey;

fn tool_reply(id: &str, name: &str, args: &str) -> ScriptedResponse {
    let events: [Value; 6] = [
        json!({"type": "message_start", "message": {"id": "m1", "type": "message",
            "role": "assistant", "content": [], "model": "claude-x",
            "usage": {"input_tokens": 10, "output_tokens": 1}}}),
        json!({"type": "content_block_start", "index": 0,
            "content_block": {"type": "tool_use", "id": id, "name": name, "input": {}}}),
        json!({"type": "content_block_delta", "index": 0,
            "delta": {"type": "input_json_delta", "partial_json": args}}),
        json!({"type": "content_block_stop", "index": 0}),
        json!({"type": "message_delta", "delta": {"stop_reason": "tool_use"},
            "usage": {"output_tokens": 5}}),
        json!({"type": "message_stop"}),
    ];
    let body: String = (events.iter())
        .map(|e| format!("event: {}\ndata: {e}\n\n", e["type"].as_str().unwrap_or("")))
        .collect();
    ScriptedResponse::new(200, &body, 7)
}

/// `find_tools` loads `write`, then one write per call id, then "done".
fn writes(calls: &[(&str, &str)]) -> Vec<ScriptedResponse> {
    let mut script = vec![tool_reply("f1", "find_tools", r#"{"names":["write"]}"#)];
    for (id, path) in calls {
        let args = json!({"path": path, "content": "x"}).to_string();
        script.push(tool_reply(id, "write", &args));
    }
    script.push(text_reply("done"));
    script
}

fn model_of(script: Vec<ScriptedResponse>) -> Model {
    Model {
        script: ScriptedTransport::new(script),
        gate: None,
        panics: AtomicBool::new(false),
    }
}

impl Window {
    /// Starts "go" in `id` with the built-in tools and `approvals` set.
    fn start_tools(&mut self, id: &str, model: Model, ask: bool) {
        let approvals = if ask { Approvals::Ask } else { Approvals::Off };
        let config = Config {
            approvals,
            ..Config::default()
        };
        let agent = agent_with(model, &self.dir, config, tools::builtin());
        self.start_agent(id, Start::Text("go".into()), agent).unwrap();
    }

    /// Applies messages until session `id` sends an approval event; returns
    /// it and whether it reached the shown run.
    fn until_approval(&mut self, id: &str) -> (AgentEvent, bool) {
        loop {
            let msg = self.rx.recv_timeout(Duration::from_secs(10));
            match msg.expect("an approval event within 10 s") {
                Msg::Event(tag, event) => {
                    let shown = self.runs.apply(&tag, &event) == Some(true);
                    let approval = matches!(
                        event,
                        AgentEvent::ApprovalRequest(_) | AgentEvent::ApprovalDone { .. }
                    );
                    if approval && tag == id {
                        return (event, shown);
                    }
                }
                Msg::Done(tag, end) => {
                    end_run(&mut self.runs, &tag, &end);
                }
            }
        }
    }

    fn request(&mut self, id: &str) -> ApprovalRequest {
        match self.until_approval(id).0 {
            AgentEvent::ApprovalRequest(request) => request,
            other => panic!("not a request: {other:?}"),
        }
    }
}

fn keys() -> Keymap {
    Keymap::defaults(Front::Gui)
}

/// The approval word on the tool row of `call`, as the window draws it.
fn word(run: &Run, call: &str) -> String {
    let item = (run.view.items.iter()).find(|i| matches!(i, Item::Tool { id, .. } if id == call));
    let item = item.expect("the tool row");
    msg_row_with(item, &run.view.approvals).approval.to_string()
}

#[test]
fn each_answer_reaches_the_session_and_the_row_says_it() {
    let mut w = Window::new("ask");
    let id = w.show_new();
    let script = writes(&[("c1", "a.txt"), ("c2", "b.txt"), ("c3", "c.txt")]);
    w.start_tools(&id, model_of(script), true);
    let first = w.request(&id);
    let shown = view(&w.runs.shown, &keys());
    assert_eq!(shown.id, first.id.as_str());
    assert_eq!(shown.title, "Approval needed: write wants to change files");
    assert!(shown.text.contains("a.txt"), "{}", shown.text);
    assert_eq!((shown.agent.as_str(), shown.more.as_str()), ("", ""));
    assert_eq!(shown.once_key, "Alt+Y");
    assert_eq!(w.runs.mark(&id, None), "waiting");
    assert_eq!(word(&w.runs.shown, "c1"), "waiting for your answer");
    // Enter and Space name no answer.
    for name in ["send", "steer", "queue", "stop", ""] {
        assert!(!answer(&w.runs.shown, &first.id, name), "{name}");
    }
    assert!(answer(&w.runs.shown, &first.id, "approve_once"));
    w.until_approval(&id);
    assert_eq!(word(&w.runs.shown, "c1"), "allowed once");
    // The second request shows after the first.
    let second = w.request(&id);
    assert_eq!(view(&w.runs.shown, &keys()).id, second.id.as_str());
    assert!(answer(&w.runs.shown, &second.id, "decline"));
    w.until_approval(&id);
    assert_eq!(word(&w.runs.shown, "c2"), "declined");
    // Another client (serve) answers: ApprovalDone closes the prompt.
    let third = w.request(&id);
    assert!(w.session(&id).answer(&third.id, Answer::Session));
    w.until_approval(&id);
    assert_eq!(view(&w.runs.shown, &keys()).id, "");
    assert_eq!(word(&w.runs.shown, "c3"), "allowed for this session");
    let (end, _) = w.pump(&id);
    assert_eq!(end.outcome, TurnOutcome::Done, "{:?}", end.error);
    assert_eq!(w.runs.mark(&id, None), "");
}

#[test]
fn stop_cancels_the_waiting_request() {
    let mut w = Window::new("ask-stop");
    let id = w.show_new();
    w.start_tools(&id, model_of(writes(&[("c1", "a.txt")])), true);
    w.request(&id);
    assert!(w.session(&id).cancel());
    let (end, _) = w.pump(&id);
    assert_eq!(end.outcome, TurnOutcome::Cancelled);
    assert_eq!(word(&w.runs.shown, "c1"), "cancelled");
    assert_eq!(view(&w.runs.shown, &keys()).id, "");
}

#[test]
fn a_parked_sessions_request_marks_its_row_only() {
    let mut w = Window::new("ask-parked");
    let first = w.show_new();
    w.start_tools(&first, model_of(writes(&[("c1", "a.txt")])), true);
    let second = w.show_new();
    assert_eq!(w.runs.parked_ids(), std::slice::from_ref(&first));
    let (event, shown) = w.until_approval(&first);
    assert!(matches!(event, AgentEvent::ApprovalRequest(_)) && !shown);
    assert_eq!(w.runs.mark(&first, None), "waiting");
    assert!(w.runs.is_shown(&second) && w.runs.shown.view.items.is_empty());
    assert_eq!(view(&w.runs.shown, &keys()).id, "");
    // Opening it shows its prompt.
    // The idle second session is dropped and closed, as the window does.
    let dropped = w.runs.show(Some(first.clone()), || Err("parked".into()));
    assert_eq!(dropped, Ok(Some(second.clone())));
    w.service.close(&second);
    assert!(!view(&w.runs.shown, &keys()).id.is_empty());
    assert!(w.session(&first).cancel());
    w.pump(&first);
}

#[test]
fn with_approvals_off_nothing_changes() {
    let mut w = Window::new("ask-off");
    let id = w.show_new();
    w.start_tools(&id, model_of(writes(&[("c1", "a.txt")])), false);
    let (end, _) = w.pump(&id);
    assert_eq!(end.outcome, TurnOutcome::Done, "{:?}", end.error);
    let run = &w.runs.shown;
    assert!(run.view.approvals.is_empty());
    for item in &run.view.items {
        // ModelRc compares by pointer: the printed rows are compared.
        let with = format!("{:?}", msg_row_with(item, &run.view.approvals));
        assert_eq!(with, format!("{:?}", crate::rows::msg_row(item)));
    }
    assert_eq!(word(run, "c1"), "");
    assert_eq!(view(run, &keys()), crate::ApprovalView::default());
}

#[test]
fn only_chords_answer_and_enter_and_space_never_do() {
    let map = keys();
    let none = [false; 4];
    let alt = [false, true, false, false];
    let enter = String::from(char::from(SlintKey::Return));
    for busy in [false, true] {
        assert_eq!(crate::keys::resolve(&map, "approval", &enter, none, busy), None);
        assert_eq!(crate::keys::resolve(&map, "approval", " ", none, busy), None);
        assert_eq!(crate::keys::resolve(&map, "approval", "y", none, busy), None);
    }
    let resolve = |text| crate::keys::resolve(&map, "approval", text, alt, true);
    assert_eq!(resolve("y"), Some("approve_once"));
    assert_eq!(resolve("a"), Some("approve_session"));
    assert_eq!(resolve("n"), Some("decline"));
    // The composer's Enter steers; its name is no answer.
    let steer = crate::keys::resolve(&map, "composer", &enter, none, true);
    assert_eq!(steer, Some("steer"));
    assert!(answer_of("steer").is_none());
    assert!(matches!(answer_of("approve_once"), Some(Answer::Once)));
}
