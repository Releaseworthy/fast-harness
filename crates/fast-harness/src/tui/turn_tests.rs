//! A TUI turn through the service (PLAN.md D59): a turn whose thread panics
//! leaves "running", shows the error, and the next prompt runs.

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::mpsc::{self, Receiver};
use std::sync::Arc;
use std::time::Duration;

use fast_harness_core::agent::{Agent, AgentParts, Conversation};
use fast_harness_core::app::{Runtime, RuntimeHost};
use fast_harness_core::auth::StaticAuth;
use fast_harness_core::config::Config;
use fast_harness_core::http::{
    Flow, HttpError, HttpHead, HttpRequest, ScriptedResponse, ScriptedTransport, Transport,
};
use fast_harness_core::llm::anthropic::Anthropic;
use fast_harness_core::llm::Auth;
use fast_harness_core::service::Host;
use fast_harness_core::tools::Registry;
use fast_harness_core::view::{Item, NoteKind, State};
use ratatui::backend::TestBackend;
use ratatui::Terminal;
use serde_json::{json, Value};

use super::app::{Msg, Tui};
use super::draw::{self, View};
use super::transcript::Cache;
use super::ui::Action;

/// The scripted model, except that its first request panics.
struct FirstPanics {
    script: ScriptedTransport,
    calls: AtomicUsize,
}

impl Transport for FirstPanics {
    fn send(
        &self,
        req: &HttpRequest<'_>,
        on_chunk: &mut dyn FnMut(&HttpHead, &[u8]) -> Flow,
    ) -> Result<HttpHead, HttpError> {
        if self.calls.fetch_add(1, Ordering::SeqCst) == 0 {
            panic!("boom transport");
        }
        self.script.send(req, on_chunk)
    }
}

struct ScriptHost {
    home: PathBuf,
    transport: Arc<FirstPanics>,
}

impl Host for ScriptHost {
    fn home(&self) -> PathBuf {
        self.home.clone()
    }

    fn config(&self, _cwd: &Path) -> Result<Config, String> {
        Ok(Config::default())
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
            config: c.clone(),
            cwd: cwd.to_path_buf(),
            system: "test system".into(),
        };
        Ok((Agent::new(parts, Arc::new(Registry::new())), Vec::new()))
    }
}

fn text_reply(text: &str) -> ScriptedResponse {
    let events: [Value; 6] = [
        json!({"type": "message_start", "message": {"id": "m1", "type": "message",
            "role": "assistant", "content": [], "model": "claude-x",
            "usage": {"input_tokens": 10, "output_tokens": 1}}}),
        json!({"type": "content_block_start", "index": 0,
            "content_block": {"type": "text", "text": ""}}),
        json!({"type": "content_block_delta", "index": 0,
            "delta": {"type": "text_delta", "text": text}}),
        json!({"type": "content_block_stop", "index": 0}),
        json!({"type": "message_delta", "delta": {"stop_reason": "end_turn"},
            "usage": {"output_tokens": 5}}),
        json!({"type": "message_stop"}),
    ];
    let body: String = (events.iter())
        .map(|e| format!("event: {}\ndata: {e}\n\n", e["type"].as_str().unwrap_or("")))
        .collect();
    ScriptedResponse::new(200, &body, 7)
}

/// Applies messages until the running turn ended.
fn pump(tui: &mut Tui, rx: &Receiver<Msg>) {
    while tui.ui.running {
        let msg = rx.recv_timeout(Duration::from_secs(10));
        tui.handle(msg.expect("the turn ended within 10 s"));
    }
}

fn header(tui: &mut Tui) -> String {
    let mut terminal = Terminal::new(TestBackend::new(80, 12)).unwrap();
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
    (0..80)
        .map(|x| buffer[(x, 0)].symbol().to_string())
        .collect()
}

fn run(root: &Path) {
    let (home, cwd) = (root.join("home"), root.join("work"));
    std::fs::create_dir_all(&home).unwrap();
    std::fs::create_dir_all(&cwd).unwrap();
    let conv = Conversation::create(&home, &cwd, None).unwrap();
    let runtime = Arc::new(RuntimeHost::at_project(
        Runtime::load_at(home.clone()).unwrap(),
        &cwd,
    ));
    let script = ScriptedTransport::new(vec![text_reply("fine")]);
    let calls = AtomicUsize::new(0);
    let transport = Arc::new(FirstPanics { script, calls });
    let turns = Arc::new(ScriptHost { home, transport });
    let (tx, rx) = mpsc::channel();
    let mut tui = Tui::with_host(runtime, turns, cwd, conv, tx).unwrap();
    tui.act(Action::Send("hi".into()));
    assert!(tui.ui.running);
    pump(&mut tui, &rx);
    assert_eq!(tui.transcript.state, State::Idle);
    assert!(header(&mut tui).contains("idle"));
    let error = |item: &Item| matches!(item, Item::Note { kind: NoteKind::Error, text } if text.contains("boom transport"));
    assert!(
        tui.transcript.items.iter().any(error),
        "{:#?}",
        tui.transcript.items
    );
    tui.act(Action::Send("again".into()));
    assert!(tui.ui.running, "the next prompt starts");
    pump(&mut tui, &rx);
    let answer = |item: &Item| matches!(item, Item::Assistant { text, .. } if text == "fine");
    assert!(tui.transcript.items.iter().any(answer));
}

#[test]
fn a_turn_that_panics_ends_and_the_next_prompt_runs() {
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| d.subsec_nanos());
    let root = std::env::temp_dir().join(format!("fh-tui-panic-{}-{nanos}", std::process::id()));
    run(&root);
    let _ = std::fs::remove_dir_all(&root);
}
