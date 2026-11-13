//! Print mode through the service (PLAN.md D59): the same bytes as the
//! turn run on the calling thread, and a panic on the turn thread ends the
//! turn `Failed` with a non-zero exit instead of ending the process.

use std::io::Write;
use std::panic::{catch_unwind, AssertUnwindSafe};
use std::path::PathBuf;
use std::process::ExitCode;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};

use fast_harness_core::agent::{Agent, AgentParts, Conversation};
use fast_harness_core::auth::StaticAuth;
use fast_harness_core::config::Config;
use fast_harness_core::events::{AgentEvent, TurnOutcome};
use fast_harness_core::http::{
    Flow, HttpError, HttpHead, HttpRequest, ScriptedResponse, ScriptedTransport, Transport,
};
use fast_harness_core::llm::anthropic::Anthropic;
use fast_harness_core::llm::{Auth, ToolSpec};
use fast_harness_core::service::json as events;
use fast_harness_core::tools::{Registry, Tool, ToolCtx, ToolOutput};
use fast_harness_core::types::Usage;
use serde_json::{json, Value};

use super::{drive, exit_code, PrintHost};
use crate::print_out::{json_sink, text_sink, Out};

/// A writer the test reads back.
#[derive(Clone, Default)]
struct Buf(Arc<Mutex<Vec<u8>>>);

impl Write for Buf {
    fn write(&mut self, data: &[u8]) -> std::io::Result<usize> {
        self.0.lock().unwrap().extend_from_slice(data);
        Ok(data.len())
    }
    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}

impl Buf {
    fn text(&self) -> String {
        String::from_utf8(self.0.lock().unwrap().clone()).unwrap()
    }
}

/// The scripted model; with `panic_at` that request (0-based) panics.
struct Model {
    script: ScriptedTransport,
    calls: AtomicUsize,
    panic_at: Option<usize>,
}

impl Transport for Model {
    fn send(
        &self,
        req: &HttpRequest<'_>,
        on_chunk: &mut dyn FnMut(&HttpHead, &[u8]) -> Flow,
    ) -> Result<HttpHead, HttpError> {
        if Some(self.calls.fetch_add(1, Ordering::SeqCst)) == self.panic_at {
            panic!("boom transport");
        }
        self.script.send(req, on_chunk)
    }
}

/// `echo` answers its text; `boom` panics.
struct Scripted(&'static str);

impl Tool for Scripted {
    fn spec(&self) -> ToolSpec {
        ToolSpec::new(self.0, "test tool", json!({"type": "object"}))
    }
    fn run(&self, args: &Value, _: &ToolCtx<'_>) -> ToolOutput {
        if self.0 == "boom" {
            panic!("boom tool");
        }
        ToolOutput::ok(args["text"].as_str().unwrap_or_default())
    }
}

struct Env {
    root: PathBuf,
    home: PathBuf,
    cwd: PathBuf,
}

impl Drop for Env {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.root);
    }
}

fn env(name: &str) -> Env {
    let root = std::env::temp_dir().join(format!("fh-print-{name}-{}", std::process::id()));
    let (home, cwd) = (root.join("home"), root.join("work"));
    std::fs::create_dir_all(&home).unwrap();
    std::fs::create_dir_all(&cwd).unwrap();
    Env { root, home, cwd }
}

fn agent(env: &Env, script: Vec<ScriptedResponse>, panic_at: Option<usize>) -> Agent {
    let mut registry = Registry::new();
    registry.add(Box::new(Scripted("echo")));
    registry.add(Box::new(Scripted("boom")));
    let script = ScriptedTransport::new(script);
    let calls = AtomicUsize::new(0);
    let parts = AgentParts {
        catalog: Default::default(),
        extensions: Default::default(),
        provider: Arc::new(Anthropic::new("http://scripted.invalid")),
        transport: Arc::new(Model {
            script,
            calls,
            panic_at,
        }),
        auth: Arc::new(StaticAuth::new("acct-1", "test", Auth::ApiKey("k".into()))),
        config: Config::default(),
        cwd: env.cwd.clone(),
        system: "test system".into(),
    };
    Agent::new(parts, Arc::new(registry))
}

fn sse(events: &[Value], chunk: usize) -> ScriptedResponse {
    let body: String = (events.iter())
        .map(|e| format!("event: {}\ndata: {e}\n\n", e["type"].as_str().unwrap_or("")))
        .collect();
    ScriptedResponse::new(200, &body, chunk)
}

fn start() -> Value {
    json!({"type": "message_start", "message": {"id": "m1", "type": "message",
        "role": "assistant", "content": [], "model": "claude-x",
        "usage": {"input_tokens": 10, "output_tokens": 1}}})
}

fn stop(reason: &str) -> [Value; 2] {
    [
        json!({"type": "message_delta", "delta": {"stop_reason": reason},
            "usage": {"output_tokens": 5}}),
        json!({"type": "message_stop"}),
    ]
}

fn text_reply(parts: &[&str]) -> ScriptedResponse {
    let mut events = vec![start()];
    events.push(json!({"type": "content_block_start", "index": 0,
        "content_block": {"type": "text", "text": ""}}));
    for text in parts {
        events.push(json!({"type": "content_block_delta", "index": 0,
            "delta": {"type": "text_delta", "text": text}}));
    }
    events.push(json!({"type": "content_block_stop", "index": 0}));
    events.extend(stop("end_turn"));
    sse(&events, 7)
}

fn tool_reply(id: &str, name: &str, args: &str) -> ScriptedResponse {
    let mut events = vec![start()];
    events.push(json!({"type": "content_block_start", "index": 0,
        "content_block": {"type": "tool_use", "id": id, "name": name, "input": {}}}));
    events.push(json!({"type": "content_block_delta", "index": 0,
        "delta": {"type": "input_json_delta", "partial_json": args}}));
    events.push(json!({"type": "content_block_stop", "index": 0}));
    events.extend(stop("tool_use"));
    sse(&events, 11)
}

fn script() -> Vec<ScriptedResponse> {
    vec![
        tool_reply("c1", "echo", r#"{"text":"one"}"#),
        text_reply(&["Hel", "lo\nwor", "ld"]),
    ]
}

/// One turn of [`script`] on the calling thread, as print mode ran it
/// before the service did (the same sinks).
fn calling_thread(env: &Env, json: bool, out: &Out) -> (TurnOutcome, String) {
    let mut conv = Conversation::create(&env.home, &env.cwd, None).unwrap();
    if json {
        out.line(&events::session_line(&conv.id, &env.cwd.to_string_lossy()));
    }
    let mut sink = match json {
        true => json_sink(out.clone()),
        false => text_sink(out.clone(), conv.id.clone()),
    };
    let mut usage = Usage::default();
    let cancel = AtomicBool::new(false);
    let outcome = agent(env, script(), None).run_turn(&mut conv, "hi", &cancel, &mut |e| {
        if let AgentEvent::TurnEnd { usage: one, .. } = &e {
            usage.add(one);
        }
        sink(e)
    });
    if json {
        out.line(&events::result_line(&conv.id, outcome, &usage, 0));
    }
    (outcome, conv.id)
}

fn through_service(env: &Env, json: bool, out: &Out) -> (TurnOutcome, String) {
    let conv = Conversation::create(&env.home, &env.cwd, None).unwrap();
    let id = conv.id.clone();
    let host = PrintHost::new(
        env.home.clone(),
        Config::default(),
        agent(env, script(), None),
    );
    (drive(host, conv, &env.cwd, "hi", json, out.clone()), id)
}

/// Zeroes the durations of a `--json` line.
fn timeless(value: &mut Value) {
    match value {
        Value::Object(map) => map
            .iter_mut()
            .for_each(|(key, v)| match key.contains("uration") {
                true => *v = json!(0),
                false => timeless(v),
            }),
        Value::Array(list) => list.iter_mut().for_each(timeless),
        _ => {}
    }
}

/// stdout and stderr with the session id and the times taken out.
fn printed(
    run: fn(&Env, bool, &Out) -> (TurnOutcome, String),
    env: &Env,
    json: bool,
) -> (TurnOutcome, String, String) {
    let (out, err) = (Buf::default(), Buf::default());
    let (outcome, id) = run(env, json, &Out::new(out.clone(), err.clone()));
    let clean = |text: String| text.replace(&id, "SID");
    let mut stdout = clean(out.text());
    if json {
        let lines = stdout.lines().map(|line| {
            let mut value: Value = serde_json::from_str(line).unwrap();
            timeless(&mut value);
            value.to_string()
        });
        stdout = lines.collect::<Vec<_>>().join("\n");
    }
    let notes = clean(err.text());
    let stderr = notes.lines().map(|line| {
        let mut parts: Vec<&str> = line.split(" | ").collect();
        if line.starts_with("[usage]") {
            parts[1] = "T s";
        }
        parts.join(" | ")
    });
    (outcome, stdout, stderr.collect::<Vec<_>>().join("\n"))
}

#[test]
fn text_output_is_the_same_through_the_service() {
    let env = env("text");
    let before = printed(calling_thread, &env, false);
    let after = printed(through_service, &env, false);
    assert_eq!(after, before);
    assert_eq!(after.0, TurnOutcome::Done);
    assert_eq!(after.1, "Hello\nworld\n");
    let usage = "[usage] in 20 out 10 cache read 0 write 0 | T s | Done | session SID";
    assert_eq!(
        after.2,
        format!("[tool] echo {{\"text\":\"one\"}}\n{usage}")
    );
}

#[test]
fn json_output_is_the_same_through_the_service() {
    let env = env("json");
    let before = printed(calling_thread, &env, true);
    let after = printed(through_service, &env, true);
    assert_eq!(after, before);
    let lines: Vec<Value> = (after.1.lines())
        .map(|l| serde_json::from_str(l).unwrap())
        .collect();
    assert_eq!(lines[0]["type"], "session");
    assert_eq!(lines.last().unwrap()["type"], "result");
    assert_eq!(after.2, "");
}

#[test]
fn a_panic_on_the_turn_thread_ends_failed_with_a_non_zero_exit() {
    let env = env("panic");
    // `boom` panics in the first step (the tool runner makes that an error
    // result); the second request panics on the turn thread.
    let script = || vec![tool_reply("c1", "boom", "{}"), text_reply(&["never"])];
    let mut conv = Conversation::create(&env.home, &env.cwd, None).unwrap();
    let old = agent(&env, script(), Some(1));
    let cancel = AtomicBool::new(false);
    let unwound = catch_unwind(AssertUnwindSafe(|| {
        old.run_turn(&mut conv, "hi", &cancel, &mut |_| {})
    }));
    assert!(
        unwound.is_err(),
        "on the calling thread the panic ends print mode"
    );
    let conv = Conversation::create(&env.home, &env.cwd, None).unwrap();
    let host = PrintHost::new(
        env.home.clone(),
        Config::default(),
        agent(&env, script(), Some(1)),
    );
    let (out, err) = (Buf::default(), Buf::default());
    let outcome = drive(
        host,
        conv,
        &env.cwd,
        "hi",
        false,
        Out::new(out.clone(), err.clone()),
    );
    assert_eq!(outcome, TurnOutcome::Failed);
    assert_eq!(
        format!("{:?}", exit_code(outcome)),
        format!("{:?}", ExitCode::FAILURE)
    );
    let notes = err.text();
    assert!(notes.contains("[tool] boom failed after"), "{notes}");
    assert!(
        notes.contains("[error] the turn stopped on an internal error: boom transport"),
        "{notes}"
    );
    assert_eq!(out.text(), "");
}
