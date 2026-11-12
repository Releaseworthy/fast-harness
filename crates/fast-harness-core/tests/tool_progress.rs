//! Tool progress (PLAN.md D58): `ToolCtx::progress` reaches the sink as
//! `AgentEvent::ToolProgress` between the call's `ToolCall` and its
//! `ToolResult`, never the trace or a request; `shell` reports its output
//! as it arrives without changing its result.

mod agent_support;

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use agent_support::*;
use fast_harness_core::agent::{Agent, Conversation};
use fast_harness_core::events::AgentEvent;
use fast_harness_core::http::ScriptedTransport;
use fast_harness_core::llm::ToolSpec;
use fast_harness_core::tools::{builtin, Registry, Tool, ToolCtx, ToolOutput};
use serde_json::{json, Value};

/// Reports `first-mark` from its own thread and `second-mark` from a thread
/// it starts; then panics, cancels or returns `done` as `mode` says.
struct Ticker;

impl Tool for Ticker {
    fn spec(&self) -> ToolSpec {
        ToolSpec::new("tick", "ticks", json!({"type": "object"}))
    }

    fn run(&self, args: &Value, ctx: &ToolCtx<'_>) -> ToolOutput {
        ctx.progress("first-mark");
        std::thread::scope(|scope| {
            scope.spawn(|| ctx.progress("second-mark"));
        });
        match args["mode"].as_str() {
            Some("panic") => panic!("ticker panics on purpose"),
            Some("cancel") => {
                ctx.cancel.store(true, Ordering::Relaxed);
                for _ in 0..10_000 {
                    ctx.progress("after-cancel");
                }
            }
            _ => {}
        }
        ToolOutput::ok("done")
    }
}

struct Turn {
    events: Vec<AgentEvent>,
    trace: String,
    bodies: Vec<String>,
}

fn turn(name: &str, args: &str) -> Turn {
    let env = env(name);
    let mut registry = Registry::new();
    registry.add(Box::new(Ticker));
    let replies = vec![tool_reply(&[("toolu_1", "tick", args)]), text_reply("ok")];
    let transport = Arc::new(ScriptedTransport::new(replies));
    let agent = Agent::new(parts(&transport, key_auth(), &env.cwd), Arc::new(registry));
    let mut conv = Conversation::create(&env.home, &env.cwd, None).unwrap();
    let mut events = Vec::new();
    agent.run_turn(&mut conv, "go", &AtomicBool::new(false), &mut |e| {
        events.push(e)
    });
    let trace = std::fs::read_to_string(&conv.log_path).unwrap();
    let bodies = transport.requests().into_iter().map(|r| r.body).collect();
    Turn {
        events,
        trace,
        bodies,
    }
}

/// The tool events in order, as short words.
fn tool_events(events: &[AgentEvent]) -> Vec<String> {
    (events.iter())
        .filter_map(|event| match event {
            AgentEvent::ToolCall { id, .. } => Some(format!("call {id}")),
            AgentEvent::ToolProgress { id, chunk } => Some(format!("progress {id} {chunk}")),
            AgentEvent::ToolResult { id, is_error, .. } => Some(format!("result {id} {is_error}")),
            _ => None,
        })
        .collect()
}

#[test]
fn progress_reaches_the_sink_in_order_and_nothing_else() {
    let run = turn("progress-order", "{}");
    let expected = [
        "call toolu_1",
        "progress toolu_1 first-mark",
        "progress toolu_1 second-mark",
        "result toolu_1 false",
    ];
    assert_eq!(tool_events(&run.events), expected);
    assert!(!run.trace.contains("first-mark"), "progress in the trace");
    assert_eq!(run.bodies.len(), 2);
    assert!(run.bodies[1].contains("done"));
    assert!(
        !run.bodies[1].contains("-mark"),
        "progress sent to the model"
    );
}

#[test]
fn a_panic_after_progress_still_ends_with_the_error_result() {
    let run = turn("progress-panic", r#"{"mode":"panic"}"#);
    let events = tool_events(&run.events);
    assert_eq!(events.last().unwrap(), "result toolu_1 true", "{events:?}");
    let result = run.events.iter().find_map(|event| match event {
        AgentEvent::ToolResult { content, .. } => Some(content.clone()),
        _ => None,
    });
    assert!(result.unwrap().contains("panicked"));
}

#[test]
fn progress_after_a_cancel_does_not_hang() {
    let started = Instant::now();
    let run = turn("progress-cancel", r#"{"mode":"cancel"}"#);
    assert!(started.elapsed() < Duration::from_secs(10));
    let events = tool_events(&run.events);
    assert!(events.contains(&"result toolu_1 false".to_string()));
}

type Seen = Mutex<Vec<(String, Instant)>>;

/// Runs `command` in `shell` with a progress receiver; returns the output,
/// the texts with their times and the end time.
fn shell(command: &str, report: bool) -> (ToolOutput, Vec<(String, Instant)>, Instant) {
    let dir = std::env::temp_dir();
    let cancel = AtomicBool::new(false);
    let seen = Seen::default();
    let progress = |text: &str| {
        seen.lock()
            .unwrap()
            .push((text.to_string(), Instant::now()))
    };
    let ctx = ToolCtx::new(&dir, &cancel);
    let ctx = if report {
        ctx.with_progress(&progress)
    } else {
        ctx
    };
    let args = json!({ "command": command }).to_string();
    let output = builtin().run("shell", &args, &ctx);
    (output, seen.into_inner().unwrap(), Instant::now())
}

#[test]
fn shell_reports_output_before_it_ends_and_keeps_its_result() {
    let command = match cfg!(windows) {
        true => "Write-Output one; Start-Sleep -Milliseconds 300; Write-Output two",
        false => "echo one; sleep 0.3; echo two",
    };
    let (output, seen, ended) = shell(command, true);
    assert!(seen.len() >= 2, "{seen:?}");
    assert!(seen[0].0.contains("one") && !seen[0].0.contains("two"));
    assert!(ended - seen[0].1 >= Duration::from_millis(250), "too late");
    let all: String = seen.iter().map(|(text, _)| text.as_str()).collect();
    assert!(all.contains("two"), "{all}");
    let (plain, none, _) = shell(command, false);
    assert!(none.is_empty());
    assert_eq!(output, plain);
}

#[test]
fn a_shell_flood_sends_few_bounded_texts() {
    let command = match cfg!(windows) {
        true => "[Console]::Out.Write('x' * 2097152)",
        false => "head -c 2097152 /dev/zero | tr '\\0' x",
    };
    let started = Instant::now();
    let (output, seen, ended) = shell(command, true);
    assert!(!output.is_error(), "{}", output.text());
    assert!(!seen.is_empty());
    let most = (ended - started).as_millis() / 100 + 2;
    assert!(seen.len() as u128 <= most, "{} texts", seen.len());
    for (text, _) in &seen {
        assert!(text.len() <= 16 * 1024 + 64, "{}", text.len());
    }
}
