//! The window's session views on `service::Service` (PLAN.md D39, D59):
//! real turns on the service's threads, with a scripted model.

use std::path::Path;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{self, Receiver, Sender};
use std::sync::Mutex;
use std::time::Duration;

use super::*;
use crate::app::state_text;
use crate::host::GuiHost;
use crate::turn::{end_run, start_run};
use fast_harness_core::agent::{Agent, AgentParts, Conversation};
use fast_harness_core::auth::StaticAuth;
use fast_harness_core::events::TurnOutcome;
use fast_harness_core::http::{
    Flow, HttpError, HttpHead, HttpRequest, ScriptedResponse, ScriptedTransport, Transport,
};
use fast_harness_core::llm::anthropic::Anthropic;
use fast_harness_core::llm::Auth;
use fast_harness_core::service::{Done, Finished, Service, Sink, Start};
use fast_harness_core::tools::Registry;
use fast_harness_core::view::{Item, State};
use serde_json::{json, Value};

/// The scripted model: each request waits for the gate (when there is
/// one), and the first one panics when asked to.
struct Model {
    script: ScriptedTransport,
    gate: Option<Mutex<Receiver<()>>>,
    panics: AtomicBool,
}

impl Transport for Model {
    fn send(
        &self,
        req: &HttpRequest<'_>,
        on_chunk: &mut dyn FnMut(&HttpHead, &[u8]) -> Flow,
    ) -> Result<HttpHead, HttpError> {
        if let Some(gate) = &self.gate {
            let gate = gate.lock().unwrap_or_else(|p| p.into_inner());
            let _ = gate.recv_timeout(Duration::from_secs(10));
        }
        if self.panics.swap(false, Ordering::SeqCst) {
            panic!("boom transport");
        }
        self.script.send(req, on_chunk)
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

fn agent(model: Model, cwd: &Path) -> Agent {
    agent_with(model, cwd, Default::default(), Registry::new())
}

fn agent_with(
    model: Model,
    cwd: &Path,
    config: fast_harness_core::config::Config,
    tools: Registry,
) -> Agent {
    let parts = AgentParts {
        catalog: Default::default(),
        extensions: Default::default(),
        provider: Arc::new(Anthropic::new("http://scripted.invalid")),
        transport: Arc::new(model),
        auth: Arc::new(StaticAuth::new("acct-1", "test", Auth::ApiKey("k".into()))),
        config,
        cwd: cwd.to_path_buf(),
        system: "test system".into(),
    };
    Agent::new(parts, Arc::new(tools))
}

/// What the turn threads hand to the "UI thread" (the test).
enum Msg {
    Event(String, AgentEvent),
    Done(String, Box<Finished>),
}

/// A window without a window: the service, its host, the views, and the
/// channel that stands in for `slint::invoke_from_event_loop`.
struct Window {
    dir: PathBuf,
    host: Arc<GuiHost>,
    service: Service,
    runs: Runs,
    tx: Sender<Msg>,
    rx: Receiver<Msg>,
}

impl Window {
    fn new(name: &str) -> Window {
        let dir = std::env::temp_dir().join(format!("fh-gui-runs-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let host = Arc::new(GuiHost::new(dir.clone()));
        let (tx, rx) = mpsc::channel();
        Window {
            service: Service::new(host.clone()),
            runs: Runs::new(dir.clone()),
            host,
            dir,
            tx,
            rx,
        }
    }

    /// A new session, shown; returns its id.
    fn show_new(&mut self) -> String {
        let conv = Conversation::create(&self.dir, &self.dir, None).unwrap();
        let session = self.service.open(conv, &self.dir).unwrap();
        // As the window does: it answers approval requests (D64).
        session.set_answering(true);
        let id = session.id.clone();
        let run = Run::new(self.dir.clone(), Some(session), Transcript::new());
        self.show(&id, run);
        id
    }

    fn show(&mut self, id: &str, run: Run) {
        let dropped = self.runs.show(Some(id.into()), || Ok(run)).unwrap();
        if let Some(old) = dropped {
            self.service.close(&old);
        }
    }

    fn start(&mut self, id: &str, start: Start, model: Model) -> Result<(), String> {
        let agent = agent(model, &self.dir);
        self.start_agent(id, start, agent)
    }

    fn start_agent(&mut self, id: &str, start: Start, agent: Agent) -> Result<(), String> {
        let (events, ended) = (self.tx.clone(), self.tx.clone());
        let tag = id.to_string();
        let sink: Sink = Box::new(move |e| {
            let _ = events.send(Msg::Event(tag.clone(), e));
        });
        let tag = id.to_string();
        let done: Done = Box::new(move |end: &Finished| {
            let _ = ended.send(Msg::Done(tag.clone(), Box::new(end.clone())));
        });
        let built = (agent, Vec::new());
        let wiring = (&self.service, &*self.host);
        start_run(&mut self.runs, wiring, id, built, start, (sink, done))
    }

    /// Applies messages until session `id`'s turn ended; returns its end
    /// and the next queued text the window would start.
    fn pump(&mut self, id: &str) -> (Finished, Option<String>) {
        loop {
            let msg = self.rx.recv_timeout(Duration::from_secs(10));
            match msg.expect("the turn ended within 10 s") {
                Msg::Event(tag, event) => {
                    self.runs.apply(&tag, &event);
                }
                Msg::Done(tag, end) => {
                    let next = end_run(&mut self.runs, &tag, &end);
                    if tag == id {
                        return (*end, next);
                    }
                }
            }
        }
    }

    fn session(&self, id: &str) -> Arc<fast_harness_core::service::Session> {
        self.service.get(id).unwrap()
    }
}

impl Drop for Window {
    fn drop(&mut self) {
        for session in self.service.open_sessions() {
            self.service.close(&session.id);
        }
        let _ = std::fs::remove_dir_all(&self.dir);
    }
}

fn model(replies: &[&str]) -> Model {
    Model {
        script: ScriptedTransport::new(replies.iter().map(|t| text_reply(t)).collect()),
        gate: None,
        panics: AtomicBool::new(false),
    }
}

/// A model whose requests wait until the returned sender sends.
fn gated(replies: &[&str]) -> (Model, Sender<()>) {
    let (open, gate) = mpsc::channel();
    let mut model = model(replies);
    model.gate = Some(Mutex::new(gate));
    (model, open)
}

fn users(run: &Run) -> Vec<String> {
    (run.view.items.iter())
        .filter_map(|item| match item {
            Item::User { text, .. } => Some(text.clone()),
            _ => None,
        })
        .collect()
}

#[path = "runs_turn_tests.rs"]
mod turn_tests;

#[path = "approval_tests.rs"]
mod approval_tests;
