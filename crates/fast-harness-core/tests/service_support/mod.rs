//! A served connection over channels (PLAN.md D48 tests): the client side
//! writes lines into `rpc::serve` on a thread and reads every line back.
#![allow(dead_code)]

use std::io::{BufReader, Read, Write};
use std::path::{Path, PathBuf};
use std::sync::mpsc::{self, Receiver, Sender};
use std::sync::{Arc, Condvar, Mutex};
use std::thread::JoinHandle;
use std::time::Duration;

use fast_harness_core::agent::{Agent, Conversation};
use fast_harness_core::config::Config;
use fast_harness_core::events::AgentEvent;
use fast_harness_core::http::ScriptedTransport;
use fast_harness_core::llm::ToolSpec;
use fast_harness_core::service::{rpc, Done, Finished, Host, Service, Session, Sink, Start};
use fast_harness_core::tools::{Registry, Tool, ToolCtx, ToolOutput};
use serde_json::{json, Value};

use crate::agent_support::{key_auth, parts, Echo};

/// Two tool calls meet here: each answers `together` when the other one
/// arrived within 5 s, else `alone`.
#[derive(Default)]
pub struct Gate {
    count: Mutex<u32>,
    arrived: Condvar,
}

struct GateTool(Arc<Gate>);

impl Tool for GateTool {
    fn spec(&self) -> ToolSpec {
        ToolSpec::new("gate", "Meet the other session", json!({"type": "object"}))
    }

    fn run(&self, _args: &Value, _ctx: &ToolCtx<'_>) -> ToolOutput {
        let gate = &self.0;
        let mut count = gate.count.lock().unwrap();
        *count += 1;
        gate.arrived.notify_all();
        let wait = Duration::from_secs(5);
        let (_count, result) = (gate.arrived)
            .wait_timeout_while(count, wait, |n| *n < 2)
            .unwrap();
        ToolOutput::ok(
            if result.timed_out() {
                "alone"
            } else {
                "together"
            }
            .to_string(),
        )
    }
}

/// Agents over a `ScriptedTransport` with `echo` and `gate` loaded.
pub struct TestHost {
    pub home: PathBuf,
    pub transport: Arc<ScriptedTransport>,
    pub gate: Arc<Gate>,
}

impl Host for TestHost {
    fn home(&self) -> PathBuf {
        self.home.clone()
    }

    fn config(&self, _cwd: &Path) -> Result<Config, String> {
        Ok(Config::default())
    }

    fn agent(
        &self,
        cwd: &Path,
        _conv: &Conversation,
        config: &Config,
    ) -> Result<(Agent, Vec<String>), String> {
        let mut parts = parts(&self.transport, key_auth(), cwd);
        parts.config = config.clone();
        let mut registry = Registry::new();
        registry.add(Box::new(Echo));
        registry.add(Box::new(GateTool(Arc::clone(&self.gate))));
        Ok((Agent::new(parts, Arc::new(registry)), Vec::new()))
    }

    fn models(&self) -> Vec<String> {
        vec!["claude-test-a".into(), "claude-test-b".into()]
    }
}

/// Lines from a channel as a reader; the end of input when it closes.
struct ChanReader {
    rx: Receiver<String>,
    buf: Vec<u8>,
    pos: usize,
}

impl Read for ChanReader {
    fn read(&mut self, out: &mut [u8]) -> std::io::Result<usize> {
        if self.pos == self.buf.len() {
            match self.rx.recv() {
                Ok(line) => (self.buf, self.pos) = (line.into_bytes(), 0),
                Err(_) => return Ok(0),
            }
        }
        let n = out.len().min(self.buf.len() - self.pos);
        out[..n].copy_from_slice(&self.buf[self.pos..self.pos + n]);
        self.pos += n;
        Ok(n)
    }
}

/// Every written line, parsed (a line that is not JSON fails the test).
struct LineWriter {
    buf: Vec<u8>,
    tx: Sender<Value>,
}

impl Write for LineWriter {
    fn write(&mut self, data: &[u8]) -> std::io::Result<usize> {
        self.buf.extend_from_slice(data);
        while let Some(end) = self.buf.iter().position(|b| *b == b'\n') {
            let line: Vec<u8> = self.buf.drain(..=end).collect();
            let value = serde_json::from_slice(&line).expect("server wrote a JSON line");
            let _ = self.tx.send(value);
        }
        Ok(data.len())
    }

    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}

pub struct Client {
    tx: Option<Sender<String>>,
    rx: Receiver<Value>,
    pub seen: Vec<Value>,
    next: u64,
    server: Option<JoinHandle<()>>,
}

impl Client {
    pub fn start(host: Arc<dyn Host>) -> Client {
        let (tx, lines) = mpsc::channel::<String>();
        let (out_tx, rx) = mpsc::channel::<Value>();
        let service = Arc::new(Service::new(host));
        let reader = BufReader::new(ChanReader {
            rx: lines,
            buf: Vec::new(),
            pos: 0,
        });
        let out = rpc::out(LineWriter {
            buf: Vec::new(),
            tx: out_tx,
        });
        let server = std::thread::spawn(move || rpc::serve(service, reader, out));
        Client {
            tx: Some(tx),
            rx,
            seen: Vec::new(),
            next: 1,
            server: Some(server),
        }
    }

    pub fn raw(&self, line: &str) {
        let tx = self.tx.as_ref().unwrap();
        tx.send(format!("{line}\n")).unwrap();
    }

    pub fn request(&mut self, method: &str, params: Value) -> u64 {
        let id = self.next;
        self.next += 1;
        let message = json!({"jsonrpc": "2.0", "id": id, "method": method, "params": params});
        self.raw(&message.to_string());
        id
    }

    pub fn notify(&self, method: &str, params: Value) {
        let message = json!({"jsonrpc": "2.0", "method": method, "params": params});
        self.raw(&message.to_string());
    }

    /// The response to `id`.
    pub fn response(&mut self, id: u64) -> Value {
        self.wait(|v| v["id"] == id && (v.get("result").is_some() || v.get("error").is_some()))
    }

    /// The result of a call (panics on an error response).
    pub fn call(&mut self, method: &str, params: Value) -> Value {
        let id = self.request(method, params);
        let answer = self.response(id);
        assert!(answer.get("error").is_none(), "{method}: {answer}");
        answer["result"].clone()
    }

    /// The first message so far, or arriving within 10 s, matching `pred`.
    pub fn wait(&mut self, pred: impl Fn(&Value) -> bool) -> Value {
        if let Some(found) = self.seen.iter().find(|v| pred(v)) {
            return found.clone();
        }
        loop {
            let value = (self.rx.recv_timeout(Duration::from_secs(10)))
                .unwrap_or_else(|_| panic!("timed out; seen: {:#?}", self.seen));
            self.seen.push(value.clone());
            if pred(&value) {
                return value;
            }
        }
    }

    /// Waits for the idle state of `session` with `stop`.
    pub fn idle(&mut self, session: &str, stop: &str) -> Value {
        self.wait(|v| {
            let update = &v["params"]["update"];
            v["params"]["sessionId"] == session
                && update["sessionUpdate"] == "state_update"
                && update["state"] == "idle"
                && update["stopReason"] == stop
        })
    }

    /// The updates of `session` seen so far.
    pub fn updates(&self, session: &str) -> Vec<Value> {
        (self.seen.iter())
            .filter(|v| v["method"] == "session/update" && v["params"]["sessionId"] == session)
            .map(|v| v["params"]["update"].clone())
            .collect()
    }

    /// Ends the input and waits for the server to finish.
    pub fn finish(&mut self) {
        self.tx.take();
        if let Some(server) = self.server.take() {
            server.join().unwrap();
        }
        while let Ok(value) = self.rx.try_recv() {
            self.seen.push(value);
        }
    }
}

/// A turn started in process whose events and end come back over channels.
pub struct Watch {
    pub events: Receiver<AgentEvent>,
    pub done: Receiver<Finished>,
}

/// Starts `start` on `session`; panics when the service refuses it.
pub fn watch(service: &Service, session: &Arc<Session>, start: Start) -> Watch {
    let (event_tx, events) = mpsc::channel();
    let (done_tx, done) = mpsc::channel();
    let sink: Sink = Box::new(move |event| {
        let _ = event_tx.send(event);
    });
    let end: Done = Box::new(move |finished: &Finished| {
        let _ = done_tx.send(finished.clone());
    });
    service.start(session, start, sink, end).unwrap();
    Watch { events, done }
}

impl Watch {
    /// The first event, within 10 s, that matches `pred`.
    pub fn until(&self, pred: impl Fn(&AgentEvent) -> bool) -> AgentEvent {
        loop {
            let event =
                (self.events.recv_timeout(Duration::from_secs(10))).expect("event within 10 s");
            if pred(&event) {
                return event;
            }
        }
    }

    /// The end of the turn within 10 s; `done` ran exactly once.
    pub fn end(&self) -> Finished {
        let finished = (self.done.recv_timeout(Duration::from_secs(10)))
            .expect("the turn's done ran within 10 s");
        assert!(self.done.recv().is_err(), "done ran once");
        finished
    }
}

/// The text of the `agent_message_chunk` updates of `session`.
pub fn answer(updates: &[Value]) -> String {
    (updates.iter())
        .filter(|u| u["sessionUpdate"] == "agent_message_chunk")
        .filter_map(|u| u["content"]["text"].as_str())
        .collect()
}
