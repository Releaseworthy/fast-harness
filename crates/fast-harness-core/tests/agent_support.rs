//! Shared helpers of the agent tests (included with `mod agent_support;`).
#![allow(dead_code)]

use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use fast_harness_core::agent::AgentParts;
use fast_harness_core::auth::{AuthError, AuthSource, Failover, Lease, StaticAuth};
use fast_harness_core::config::Config;
use fast_harness_core::http::{HttpHead, ScriptedResponse, ScriptedTransport, Transport};
use fast_harness_core::llm::anthropic::Anthropic;
use fast_harness_core::llm::{Auth, LlmError, ToolSpec};
use fast_harness_core::tools::{Registry, Tool, ToolCtx, ToolOutput};
use fast_harness_core::trace::{self, TraceEvent};
use serde_json::{json, Value};

/// A fresh folder under the temp dir, removed on drop.
pub struct TempDir(pub PathBuf);

impl TempDir {
    pub fn new(name: &str) -> TempDir {
        let nanos = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .subsec_nanos();
        let dir = format!("fast-harness-agent-{name}-{}-{nanos}", std::process::id());
        let path = std::env::temp_dir().join(dir);
        std::fs::create_dir_all(&path).unwrap();
        TempDir(path)
    }
}

impl Drop for TempDir {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

/// A temp root with `home` (FAST_HARNESS_HOME) and `work` (cwd) inside.
pub struct Env {
    pub root: TempDir,
    pub home: PathBuf,
    pub cwd: PathBuf,
}

pub fn env(name: &str) -> Env {
    let root = TempDir::new(name);
    let (home, cwd) = (root.0.join("home"), root.0.join("work"));
    std::fs::create_dir_all(&home).unwrap();
    std::fs::create_dir_all(&cwd).unwrap();
    Env { root, home, cwd }
}

fn sse(events: &[Value]) -> String {
    let mut out = String::new();
    for event in events {
        let name = event["type"].as_str().unwrap();
        out.push_str(&format!("event: {name}\ndata: {event}\n\n"));
    }
    out
}

fn start() -> Value {
    json!({"type": "message_start", "message": {"id": "msg_1", "type": "message",
        "role": "assistant", "content": [], "model": "claude-opus-5-5",
        "usage": {"input_tokens": 10, "output_tokens": 1}}})
}

fn finish(events: &mut Vec<Value>, reason: &str) {
    events.push(json!({"type": "message_delta",
        "delta": {"stop_reason": reason, "stop_sequence": null}, "usage": {"output_tokens": 5}}));
    events.push(json!({"type": "message_stop"}));
}

/// A streamed text answer, delivered in 7 byte chunks.
pub fn text_reply(text: &str) -> ScriptedResponse {
    let mut events = vec![
        start(),
        json!({"type": "content_block_start", "index": 0, "content_block": {"type": "text", "text": ""}}),
        json!({"type": "content_block_delta", "index": 0, "delta": {"type": "text_delta", "text": text}}),
        json!({"type": "content_block_stop", "index": 0}),
    ];
    finish(&mut events, "end_turn");
    ScriptedResponse::new(200, &sse(&events), 7)
}

/// A streamed answer asking for tool calls `(id, name, arguments)`.
pub fn tool_reply(calls: &[(&str, &str, &str)]) -> ScriptedResponse {
    let mut events = vec![start()];
    for (index, (id, name, args)) in calls.iter().enumerate() {
        events.push(json!({"type": "content_block_start", "index": index,
            "content_block": {"type": "tool_use", "id": id, "name": name, "input": {}}}));
        events.push(json!({"type": "content_block_delta", "index": index,
            "delta": {"type": "input_json_delta", "partial_json": args}}));
        events.push(json!({"type": "content_block_stop", "index": index}));
    }
    finish(&mut events, "tool_use");
    ScriptedResponse::new(200, &sse(&events), 11)
}

pub fn error_reply(status: u16, kind: &str, message: &str) -> ScriptedResponse {
    let body = json!({"type": "error", "error": {"type": kind, "message": message}});
    ScriptedResponse::new(status, &body.to_string(), 64)
}

pub fn key_auth() -> Arc<dyn AuthSource> {
    Arc::new(StaticAuth::new(
        "acct-1",
        "test",
        Auth::ApiKey("test-key".into()),
    ))
}

pub fn parts(
    transport: &Arc<ScriptedTransport>,
    auth: Arc<dyn AuthSource>,
    cwd: &Path,
) -> AgentParts {
    AgentParts {
        extensions: Default::default(),
        provider: Arc::new(Anthropic::new("http://scripted.invalid")),
        transport: transport.clone(),
        auth,
        config: Config::default(),
        cwd: cwd.to_path_buf(),
        system: "test system".into(),
    }
}

/// Returns `text` after sleeping `sleepMs` milliseconds.
pub struct Echo;

impl Tool for Echo {
    fn spec(&self) -> ToolSpec {
        ToolSpec {
            name: "echo".into(),
            description: "Echo text".into(),
            schema: json!({"type": "object"}),
        }
    }

    fn run(&self, args: &Value, _ctx: &ToolCtx<'_>) -> ToolOutput {
        std::thread::sleep(Duration::from_millis(args["sleepMs"].as_u64().unwrap_or(0)));
        ToolOutput::ok(args["text"].as_str().unwrap_or("").to_string())
    }
}

pub fn echo_registry() -> Arc<Registry> {
    let mut registry = Registry::new();
    registry.add(Box::new(Echo));
    Arc::new(registry)
}

pub fn events(path: &Path) -> Vec<TraceEvent> {
    trace::read(path).unwrap().1
}

pub fn kinds(path: &Path) -> Vec<String> {
    events(path).into_iter().map(|e| e.kind).collect()
}

pub fn first<'a>(events: &'a [TraceEvent], kind: &str) -> &'a TraceEvent {
    events.iter().find(|e| e.kind == kind).unwrap()
}

/// Accounts in order; `on_failure` answers `decision` and, for `Retry`,
/// moves to the next account.
pub struct FakeAuth {
    pub accounts: Vec<&'static str>,
    pub current: Mutex<usize>,
    pub decision: Failover,
}

impl FakeAuth {
    pub fn new(accounts: Vec<&'static str>, decision: Failover) -> Arc<FakeAuth> {
        Arc::new(FakeAuth {
            accounts,
            current: Mutex::new(0),
            decision,
        })
    }
}

impl AuthSource for FakeAuth {
    fn lease(&self, _http: &dyn Transport) -> Result<Lease, AuthError> {
        let index = *self.current.lock().unwrap();
        let account = self.accounts[index.min(self.accounts.len() - 1)];
        Ok(Lease {
            account: account.into(),
            label: account.into(),
            auth: Auth::OAuth(format!("token-{account}")),
        })
    }

    fn observe(&self, _lease: &Lease, _head: &HttpHead) {}

    fn on_failure(&self, _: &Lease, _: &LlmError, _: &str, _: &dyn Transport) -> Failover {
        if self.decision == Failover::Retry {
            *self.current.lock().unwrap() += 1;
        }
        self.decision
    }
}
