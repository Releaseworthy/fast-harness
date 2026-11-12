//! The TUI approval prompt (PLAN.md D64) through a real service session:
//! the prompt shows the whole command, each key gives its answer, Enter
//! and Space never approve, typed text stays, requests queue, an answer
//! from elsewhere closes the prompt, and the footer names the keys.

use std::path::{Path, PathBuf};
use std::sync::mpsc::{self, Receiver};
use std::sync::Arc;
use std::time::Duration;

use fast_harness_core::agent::{Agent, AgentParts, Answer, Conversation};
use fast_harness_core::app::{Runtime, RuntimeHost};
use fast_harness_core::auth::StaticAuth;
use fast_harness_core::config::approvals::Approvals;
use fast_harness_core::config::Config;
use fast_harness_core::events::ApprovalRequest;
use fast_harness_core::http::{ScriptedResponse, ScriptedTransport};
use fast_harness_core::llm::anthropic::Anthropic;
use fast_harness_core::llm::{Auth, ToolSpec};
use fast_harness_core::service::Host;
use fast_harness_core::tools::{Effect, Registry, Tool, ToolCtx, ToolOutput};
use ratatui::backend::TestBackend;
use ratatui::crossterm::event::{Event, KeyCode, KeyEvent, KeyModifiers};
use ratatui::Terminal;
use serde_json::{json, Value};

use super::app::{Msg, Tui};
use super::draw::{self, View};
use super::transcript::Cache;
use super::ui::Action;

/// A `shell` that runs nothing: the gate keys on the name.
struct FakeShell;

impl Tool for FakeShell {
    fn spec(&self) -> ToolSpec {
        ToolSpec::new("shell", "Run", json!({"type": "object"}))
    }

    fn run(&self, args: &Value, _ctx: &ToolCtx<'_>) -> ToolOutput {
        ToolOutput::ok(format!("ran {}", args["command"].as_str().unwrap_or("")))
    }
}

struct AskHost {
    home: PathBuf,
    transport: Arc<ScriptedTransport>,
    approvals: Approvals,
}

impl Host for AskHost {
    fn home(&self) -> PathBuf {
        self.home.clone()
    }

    fn config(&self, _cwd: &Path) -> Result<Config, String> {
        Ok(Config {
            approvals: self.approvals,
            ..Config::default()
        })
    }

    fn agent(
        &self,
        cwd: &Path,
        _: &Conversation,
        c: &Config,
    ) -> Result<(Agent, Vec<String>), String> {
        let parts = AgentParts {
            extensions: Default::default(),
            provider: Arc::new(Anthropic::new("http://scripted.invalid")),
            transport: self.transport.clone(),
            auth: Arc::new(StaticAuth::new("acct-1", "test", Auth::ApiKey("k".into()))),
            // The TUI hands its runtime's settings; the case sets the gate.
            config: Config {
                approvals: self.approvals,
                ..c.clone()
            },
            cwd: cwd.to_path_buf(),
            system: "test system".into(),
        };
        let mut registry = Registry::new();
        registry.add(Box::new(FakeShell));
        Ok((Agent::new(parts, Arc::new(registry)), Vec::new()))
    }
}

fn sse(events: &[Value]) -> ScriptedResponse {
    let body: String = (events.iter())
        .map(|e| format!("event: {}\ndata: {e}\n\n", e["type"].as_str().unwrap_or("")))
        .collect();
    ScriptedResponse::new(200, &body, 9)
}

fn start() -> Value {
    json!({"type": "message_start", "message": {"id": "m1", "type": "message",
        "role": "assistant", "content": [], "model": "claude-x",
        "usage": {"input_tokens": 10, "output_tokens": 1}}})
}

/// One reply calling `shell` with each command in order.
fn shell_calls(commands: &[&str]) -> ScriptedResponse {
    let mut events = vec![start()];
    for (index, command) in commands.iter().enumerate() {
        let args = json!({ "command": command }).to_string();
        events.push(json!({"type": "content_block_start", "index": index,
            "content_block": {"type": "tool_use", "id": format!("c{index}"), "name": "shell", "input": {}}}));
        events.push(json!({"type": "content_block_delta", "index": index,
            "delta": {"type": "input_json_delta", "partial_json": args}}));
        events.push(json!({"type": "content_block_stop", "index": index}));
    }
    events.push(
        json!({"type": "message_delta", "delta": {"stop_reason": "tool_use"},
        "usage": {"output_tokens": 5}}),
    );
    events.push(json!({"type": "message_stop"}));
    sse(&events)
}

fn text_reply(text: &str) -> ScriptedResponse {
    sse(&[
        start(),
        json!({"type": "content_block_start", "index": 0, "content_block": {"type": "text", "text": ""}}),
        json!({"type": "content_block_delta", "index": 0, "delta": {"type": "text_delta", "text": text}}),
        json!({"type": "content_block_stop", "index": 0}),
        json!({"type": "message_delta", "delta": {"stop_reason": "end_turn"}, "usage": {"output_tokens": 5}}),
        json!({"type": "message_stop"}),
    ])
}

struct Case {
    root: PathBuf,
    tui: Tui,
    rx: Receiver<Msg>,
}

impl Drop for Case {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.root);
    }
}

fn case(name: &str, approvals: Approvals, replies: Vec<ScriptedResponse>) -> Case {
    let nanos = (std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH))
        .map_or(0, |d| d.subsec_nanos());
    let root = std::env::temp_dir().join(format!("fh-tui-{name}-{}-{nanos}", std::process::id()));
    let (home, cwd) = (root.join("home"), root.join("work"));
    std::fs::create_dir_all(&home).unwrap();
    std::fs::create_dir_all(&cwd).unwrap();
    let conv = Conversation::create(&home, &cwd, None).unwrap();
    let runtime = Arc::new(RuntimeHost::at_project(
        Runtime::load_at(home.clone()).unwrap(),
        &cwd,
    ));
    let transport = Arc::new(ScriptedTransport::new(replies));
    let turns = Arc::new(AskHost {
        home,
        transport,
        approvals,
    });
    let (tx, rx) = mpsc::channel();
    let tui = Tui::with_host(runtime, turns, cwd, (conv, Vec::new()), tx).unwrap();
    Case { root, tui, rx }
}

impl Case {
    /// Applies messages until `done` holds.
    fn until(&mut self, done: impl Fn(&Tui) -> bool) {
        while !done(&self.tui) {
            let Ok(msg) = self.rx.recv_timeout(Duration::from_secs(10)) else {
                panic!("no message in 10 s: {:#?}", self.tui.transcript.items);
            };
            self.tui.handle(msg);
        }
    }

    fn key(&mut self, code: KeyCode, mods: KeyModifiers) {
        self.tui
            .handle(Msg::Term(Event::Key(KeyEvent::new(code, mods))));
    }

    fn typed(&mut self, text: &str) {
        for c in text.chars() {
            self.key(KeyCode::Char(c), KeyModifiers::NONE);
        }
    }

    fn alt(&mut self, c: char) {
        self.key(KeyCode::Char(c), KeyModifiers::ALT);
    }

    fn waiting(&self) -> Option<String> {
        self.tui.ui.approvals.first().map(|r| r.id.clone())
    }

    fn screen(&mut self) -> String {
        screen(&mut self.tui, 80, 24)
    }
}

fn screen(tui: &mut Tui, width: u16, height: u16) -> String {
    let mut terminal = Terminal::new(TestBackend::new(width, height)).unwrap();
    let mut cache = Cache::default();
    let mut view = View {
        transcript: &tui.transcript,
        cache: &mut cache,
        session: "s-1",
    };
    let ui = &mut tui.ui;
    terminal
        .draw(|frame| draw::draw(frame, ui, &mut view, 0))
        .unwrap();
    let buffer = terminal.backend().buffer();
    (0..height)
        .map(|y| {
            (0..width)
                .map(|x| buffer[(x, y)].symbol())
                .collect::<String>()
                + "\n"
        })
        .collect()
}

const LONG: &str = "Write-Output hello; Get-ChildItem C:/a/very/long/folder/name/that/goes/on/and/on/for/a/while/until/it/wraps | Remove-Item -Recurse END-OF-COMMAND";

#[test]
fn each_key_answers_and_enter_or_space_never_approve() {
    let calls = [LONG, "Write-Output two", "Write-Output three"];
    let mut c = case(
        "ask",
        Approvals::Ask,
        vec![shell_calls(&calls), text_reply("ok")],
    );
    c.tui.act(Action::Send("go".into()));
    c.until(|t| !t.ui.approvals.is_empty());
    let first = c.waiting().unwrap();
    let shown = c.screen();
    assert!(
        shown.contains("Approval needed: shell wants to run a command"),
        "{shown}"
    );
    assert!(
        shown.contains("END-OF-COMMAND"),
        "the end of a long command shows:\n{shown}"
    );
    assert!(
        shown.contains("Alt+Y allow once · Alt+A allow for this session · Alt+N decline"),
        "{shown}"
    );
    assert!(
        shown.contains("waiting for your answer · "),
        "the tool row says so:\n{shown}"
    );
    let footer = super::keys::hint_text(&c.tui.ui);
    assert!(footer.starts_with("Alt+Y allow once"), "{footer}");
    assert!(
        footer.ends_with("Esc cancels the turn"),
        "the footer names Stop: {footer}"
    );
    // Space and Enter type or steer; they never answer.
    c.key(KeyCode::Char(' '), KeyModifiers::NONE);
    c.key(KeyCode::Enter, KeyModifiers::NONE);
    c.typed("keep me");
    assert_eq!(c.tui.current.approvals().len(), 1, "still waiting");
    c.alt('y');
    assert_eq!(
        c.tui.ui.input.text(),
        " keep me",
        "typed text survives the prompt"
    );
    c.until(|t| t.ui.approvals.first().is_some_and(|r| r.id != first));
    assert!(c.screen().contains("Write-Output two"));
    c.alt('a');
    c.until(|t| t.ui.approvals.len() == 1 && t.ui.approvals[0].arguments.contains("three"));
    c.alt('n');
    c.until(|t| !t.ui.running);
    assert!(c.tui.ui.approvals.is_empty(), "the prompt is gone");
    let t = &c.tui.transcript;
    let words: Vec<_> = ["c0", "c1", "c2"]
        .iter()
        .map(|id| t.approvals.word(id))
        .collect();
    assert_eq!(
        words,
        [
            Some("allowed once"),
            Some("allowed for this session"),
            Some("declined")
        ]
    );
    let shown = c.screen();
    assert!(shown.contains("allowed once · "), "{shown}");
    assert!(shown.contains("declined · "), "{shown}");
    assert!(shown.contains("keep me"), "the input box kept the text");
}

#[test]
fn an_answer_from_elsewhere_closes_the_prompt() {
    let mut c = case(
        "elsewhere",
        Approvals::Ask,
        vec![shell_calls(&["Write-Output x"]), text_reply("ok")],
    );
    c.tui.act(Action::Send("go".into()));
    c.until(|t| !t.ui.approvals.is_empty());
    let id = c.waiting().unwrap();
    assert!(
        c.tui.current.answer(&id, Answer::Once),
        "another client answers"
    );
    c.until(|t| t.ui.approvals.is_empty());
    assert!(!c.screen().contains("Approval needed"));
    c.until(|t| !t.ui.running);
    assert_eq!(c.tui.transcript.approvals.word("c0"), Some("allowed once"));
}

#[test]
fn stop_cancels_a_waiting_request() {
    let mut c = case(
        "stop",
        Approvals::Ask,
        vec![shell_calls(&["Write-Output x"]), text_reply("ok")],
    );
    c.tui.act(Action::Send("go".into()));
    c.until(|t| !t.ui.approvals.is_empty());
    c.key(KeyCode::Esc, KeyModifiers::NONE);
    c.until(|t| !t.ui.running);
    assert!(c.tui.ui.approvals.is_empty());
    assert_eq!(c.tui.transcript.approvals.word("c0"), Some("cancelled"));
}

#[test]
fn with_approvals_off_the_keys_type_and_nothing_shows() {
    let mut c = case(
        "off",
        Approvals::Off,
        vec![shell_calls(&["Write-Output x"]), text_reply("ok")],
    );
    c.alt('y');
    assert_eq!(c.tui.ui.input.text(), "y", "Alt+Y types as before");
    c.tui.ui.input.clear();
    c.tui.act(Action::Send("go".into()));
    c.until(|t| !t.ui.running);
    assert!(c.tui.transcript.approvals.is_empty());
    assert!(!c.screen().contains("Approval"));
}

#[test]
fn a_subagent_request_names_the_subagent_and_queues() {
    let mut c = case("child", Approvals::Ask, Vec::new());
    let request = |id: &str| {
        let mut r = ApprovalRequest::default();
        r.id = id.into();
        r.call_id = "k1".into();
        r.tool = "write".into();
        r.effect = Effect::Write;
        r.summary = "write file: C:/w/a.txt".into();
        r.arguments = "{}".into();
        r.agent = Some("helper (s-9)".into());
        r
    };
    c.tui.ui.approvals = vec![request("ap1"), request("ap2")];
    let shown = c.screen();
    assert!(shown.contains("write wants to change files"), "{shown}");
    assert!(shown.contains("write file: C:/w/a.txt"), "{shown}");
    assert!(shown.contains("asked by subagent helper (s-9)"), "{shown}");
    assert!(shown.contains("1 more waiting after this one"), "{shown}");
}
