//! Live and replayed transcripts agree (PLAN.md D61, gate G8 a): every case
//! runs scripted turns through the real agent loop with a sink feeding
//! `Transcript::apply`, then rebuilds the transcript from the log it wrote
//! with `Transcript::from_trace`; the two must be equal.

mod agent_support;

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::Sender;
use std::sync::Arc;

use agent_support::*;
use fast_harness_core::agent::{Agent, AgentParts, Conversation, Inbox, Input};
use fast_harness_core::auth::{AuthSource, Failover};
use fast_harness_core::events::{AgentEvent, TurnOutcome};
use fast_harness_core::http::{ScriptedResponse, ScriptedTransport};
use fast_harness_core::llm::ToolSpec;
use fast_harness_core::tools::{Registry, Tool, ToolCtx, ToolOutput};
use fast_harness_core::view::{Item, NoteKind, State, Transcript};
use serde_json::{json, Value};

const HI: &str = r#"{"text":"hi"}"#;

/// Echoes `text`, serial (no `effect`): the write of a mixed step.
struct Jot;

impl Tool for Jot {
    fn spec(&self) -> ToolSpec {
        ToolSpec::new("jot", "Write a note", json!({"type": "object"}))
    }

    fn run(&self, args: &Value, _ctx: &ToolCtx<'_>) -> ToolOutput {
        ToolOutput::ok(format!("noted {}", args["text"].as_str().unwrap_or("")))
    }
}

/// One case: its live view and the conversation whose log replays it.
struct Run {
    live: Transcript,
    conv: Conversation,
    agent: Option<Agent>,
}

fn scripted(env: &Env, replies: Vec<ScriptedResponse>) -> Run {
    build(env, replies, key_auth(), |_| {})
}

/// A run on `echo` and `jot`, its parts changed by `tweak`.
fn build(
    env: &Env,
    replies: Vec<ScriptedResponse>,
    auth: Arc<dyn AuthSource>,
    tweak: fn(&mut AgentParts),
) -> Run {
    let transport = Arc::new(ScriptedTransport::new(replies));
    let mut registry = Registry::new();
    registry.add(Box::new(Echo));
    registry.add(Box::new(Jot));
    let mut parts = parts(&transport, auth, &env.cwd);
    tweak(&mut parts);
    let conv = Conversation::create(&env.home, &env.cwd, None).unwrap();
    let agent = Agent::new(parts, Arc::new(registry));
    Run {
        live: Transcript::new(),
        conv,
        agent: Some(agent),
    }
}

/// A run whose agent has the built-in tools (`find_tools` first) and `subagent`.
fn with_subagent(env: &Env, replies: Vec<ScriptedResponse>) -> Run {
    let transport = Arc::new(ScriptedTransport::new(replies));
    let parts = parts(&transport, key_auth(), &env.cwd);
    let conv = Conversation::create(&env.home, &env.cwd, None).unwrap();
    let agent = Agent::with_subagent(parts, &env.home, &conv.id);
    Run {
        live: Transcript::new(),
        conv,
        agent: Some(agent),
    }
}

impl Run {
    /// A turn as the front ends run it: the user text first, then the events.
    fn turn(&mut self, text: &str) -> TurnOutcome {
        self.turn_with(text, None, |_| false)
    }

    /// [`Run::turn`] that sends `input` at the first event `when` matches,
    /// or cancels there when `input` is `None` and `cancel` is set.
    fn turn_with(
        &mut self,
        text: &str,
        input: Option<Input>,
        when: fn(&AgentEvent) -> bool,
    ) -> TurnOutcome {
        if !text.starts_with('/') {
            self.live.push_user(text);
        }
        let (tx, mut inbox) = Inbox::channel();
        let mut pending: Option<(Sender<Input>, Input)> = input.map(|i| (tx, i));
        let cancel = AtomicBool::new(false);
        let (agent, live) = (self.agent.as_ref().unwrap(), &mut self.live);
        agent.run_turn_with(&mut self.conv, text, &cancel, &mut inbox, &mut |event| {
            if when(&event) {
                match pending.take() {
                    Some((tx, input)) => tx.send(input).unwrap(),
                    None => cancel.store(true, Ordering::SeqCst),
                }
            }
            live.apply(&event);
        })
    }

    fn resume(&mut self) -> TurnOutcome {
        let (agent, live) = (self.agent.as_ref().unwrap(), &mut self.live);
        let cancel = AtomicBool::new(false);
        agent.resume_turn(&mut self.conv, &cancel, &mut Inbox::empty(), &mut |event| {
            live.apply(&event)
        })
    }
}

fn on_tool_call(event: &AgentEvent) -> bool {
    matches!(event, AgentEvent::ToolCall { .. })
}

/// Fields that differ by design, live only: `revision` (counts changes),
/// `state` (what the agent is doing now) and the subagent notes
/// (`AgentEvent::Subagent` and `SubagentProgress` have no log event: the
/// child's own log holds its work, the parent's `tool/result` names it).
fn normalise(mut view: Transcript) -> Transcript {
    view.revision = 0;
    view.state = State::Idle;
    view.items.retain(|item| !subagent_note(item));
    view
}

fn subagent_note(item: &Item) -> bool {
    matches!(item, Item::Note { kind: NoteKind::Info, text } if text.starts_with("Subagent "))
}

/// One line per differing field or item, `live | replay`.
fn diff(live: &Transcript, replay: &Transcript) -> Vec<String> {
    let mut out = Vec::new();
    let fields = |v: &Transcript| {
        let (turn, step, model, account) = (v.turn, v.step, &v.model, &v.account);
        let (context, window) = (v.context, v.window);
        let usage = (format!("{:?}", v.totals), format!("{:?}", v.turn_usage));
        format!("turn {turn} step {step} model {model} account {account} context {context} window {window} totals {} turn {}", usage.0, usage.1)
    };
    if fields(live) != fields(replay) {
        out.push(format!("  fields {} | {}", fields(live), fields(replay)));
    }
    let count = live.items.len().max(replay.items.len());
    for index in 0..count {
        let (a, b) = (live.items.get(index), replay.items.get(index));
        if a != b {
            out.push(format!("  item {index}: {a:?}\n        | {b:?}"));
        }
    }
    out
}

#[path = "view_parity/cases.rs"]
mod cases;

#[test]
fn every_scripted_turn_replays_to_the_live_transcript() {
    let mut failures = Vec::new();
    for (name, case) in cases::CASES {
        let env = env(&format!("parity-{name}"));
        let run = case(&env);
        let replayed = Transcript::from_trace(&events(&run.conv.log_path));
        let (live, replayed) = (normalise(run.live), normalise(replayed));
        if live != replayed {
            failures.push(format!("{name}:\n{}", diff(&live, &replayed).join("\n")));
        }
        drop(run.agent);
    }
    assert!(failures.is_empty(), "{}", failures.join("\n\n"));
}
