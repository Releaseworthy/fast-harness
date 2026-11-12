//! How the tool calls of one step run (PLAN.md D54): calls whose tool is
//! parallel (`Effect::Read`) start together; any other call runs alone, in
//! call order, after everything before it; a cancel keeps the calls that
//! have not started from starting.

mod agent_support;

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use agent_support::*;
use fast_harness_core::agent::{Agent, Conversation};
use fast_harness_core::events::AgentEvent;
use fast_harness_core::http::ScriptedTransport;
use fast_harness_core::llm::ToolSpec;
use fast_harness_core::tools::{Effect, Registry, Tool, ToolCtx, ToolOutput};
use serde_json::{json, Value};

/// When each call (by its `tag` argument) started and ended.
type Log = Arc<Mutex<Vec<(String, Instant, Instant)>>>;

/// Sleeps `sleepMs`, then for `Write` writes `text` to `file`, for `Read`
/// returns the content of `file` (or "missing"). `stop` sets the cancel flag.
struct Probe {
    name: &'static str,
    effect: Effect,
    log: Log,
}

impl Tool for Probe {
    fn spec(&self) -> ToolSpec {
        ToolSpec::new(self.name, "probe", json!({"type": "object"}))
    }

    fn effect(&self) -> Effect {
        self.effect
    }

    fn run(&self, args: &Value, ctx: &ToolCtx<'_>) -> ToolOutput {
        let started = Instant::now();
        std::thread::sleep(Duration::from_millis(args["sleepMs"].as_u64().unwrap_or(0)));
        let file = ctx.cwd.join(args["file"].as_str().unwrap_or("x.txt"));
        let text = match self.effect {
            Effect::Write => {
                std::fs::write(&file, args["text"].as_str().unwrap_or("")).unwrap();
                "written".to_string()
            }
            _ => std::fs::read_to_string(&file).unwrap_or_else(|_| "missing".into()),
        };
        if args["stop"] == true {
            ctx.cancel.store(true, Ordering::Relaxed);
        }
        let tag = args["tag"].as_str().unwrap_or("").to_string();
        self.log
            .lock()
            .unwrap()
            .push((tag, started, Instant::now()));
        ToolOutput::ok(text)
    }
}

/// One turn whose first step holds `calls` (name, arguments) of the probe
/// tools `look` (Read), `put` (Write) and `exec` (Run). Returns the
/// `ToolResult` texts by call id and the log.
fn run_step(name: &str, calls: &[(&str, &str)]) -> (Vec<(String, String)>, Log) {
    let env = env(name);
    let log = Log::default();
    let mut registry = Registry::new();
    for (name, effect) in [
        ("look", Effect::Read),
        ("put", Effect::Write),
        ("exec", Effect::Run),
    ] {
        registry.add(Box::new(Probe {
            name,
            effect,
            log: log.clone(),
        }));
    }
    let ids: Vec<String> = (0..calls.len()).map(|n| format!("toolu_{n}")).collect();
    let script: Vec<(&str, &str, &str)> = (calls.iter().zip(&ids))
        .map(|((name, args), id)| (id.as_str(), *name, *args))
        .collect();
    let replies = vec![tool_reply(&script), text_reply("done")];
    let transport = Arc::new(ScriptedTransport::new(replies));
    let agent = Agent::new(parts(&transport, key_auth(), &env.cwd), Arc::new(registry));
    let mut conv = Conversation::create(&env.home, &env.cwd, None).unwrap();
    let mut results = Vec::new();
    let mut sink = |event: AgentEvent| {
        if let AgentEvent::ToolResult { id, content, .. } = event {
            results.push((id, content));
        }
    };
    agent.run_turn(&mut conv, "go", &AtomicBool::new(false), &mut sink);
    results.sort();
    (results, log)
}

fn span(log: &Log, tag: &str) -> (Instant, Instant) {
    let log = log.lock().unwrap();
    let (_, start, end) = log.iter().find(|(t, ..)| t == tag).unwrap();
    (*start, *end)
}

#[test]
fn read_calls_of_one_step_run_together() {
    let slow = r#"{"tag":"a","sleepMs":300}"#;
    let other = r#"{"tag":"b","sleepMs":300}"#;
    let started = Instant::now();
    let (results, log) = run_step("fx-read", &[("look", slow), ("look", other)]);
    let elapsed = started.elapsed();
    assert_eq!(results.len(), 2);
    let ((a0, a1), (b0, b1)) = (span(&log, "a"), span(&log, "b"));
    // Serial would take 600 ms or more; 550 leaves room for slow CI runners.
    let together = a1.max(b1) - a0.min(b0);
    assert!(
        together < Duration::from_millis(550),
        "{together:?}, turn {elapsed:?}"
    );
}

#[test]
fn run_calls_of_one_step_never_overlap() {
    let first = r#"{"tag":"a","sleepMs":150}"#;
    let second = r#"{"tag":"b","sleepMs":150}"#;
    let (results, log) = run_step("fx-run", &[("exec", first), ("exec", second)]);
    assert_eq!(results.len(), 2);
    let ((_, a1), (b0, _)) = (span(&log, "a"), span(&log, "b"));
    assert!(
        b0 >= a1,
        "the second Run call started before the first ended"
    );
}

/// Failed before D54: the read ran during the write's sleep and saw no file.
#[test]
fn a_read_after_a_write_in_one_step_sees_the_write() {
    let put = r#"{"tag":"w","sleepMs":200,"file":"x.txt","text":"new text"}"#;
    let look = r#"{"tag":"r","file":"x.txt"}"#;
    let (results, _) = run_step("fx-order", &[("look", look), ("put", put), ("look", look)]);
    let texts: Vec<&str> = results.iter().map(|(_, text)| text.as_str()).collect();
    assert_eq!(texts, ["missing", "written", "new text"]);
}

#[test]
fn a_cancel_keeps_later_serial_calls_from_starting() {
    let stop = r#"{"tag":"s","stop":true}"#;
    let next = r#"{"tag":"n"}"#;
    let (results, log) = run_step(
        "fx-cancel",
        &[("exec", stop), ("put", next), ("look", next)],
    );
    let texts: Vec<&str> = results.iter().map(|(_, text)| text.as_str()).collect();
    assert_eq!(
        texts,
        [
            "missing",
            "cancelled before it ran",
            "cancelled before it ran"
        ]
    );
    assert_eq!(log.lock().unwrap().len(), 1, "only the first call ran");
}
