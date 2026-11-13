//! Building and running one subagent (PLAN.md D40).

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::Sender;
use std::sync::Arc;
use std::time::{Duration, Instant};

use serde_json::json;

use super::child_tools;
use super::children::{self, Children, Slot};
use super::live;
use super::subagent::{Spec, SubagentTool, NAME};
use crate::agent::{Agent, AgentParts, Conversation, Inbox, Input};
use crate::events::{AgentEvent, TurnOutcome};
use crate::llm::catalog;
use crate::tools::{self, ToolCtx, ToolOutput};
use crate::types::{Role, Usage};

/// The shortest time between two progress lines of one child.
const PROGRESS_EVERY: Duration = Duration::from_millis(500);

/// Runs the child of `spec`: in the foreground its answer is the output,
/// in the background the output is `{id, session}`.
pub(super) fn start(
    tool: &SubagentTool,
    spec: &Spec,
    ctx: &ToolCtx<'_>,
) -> Result<ToolOutput, String> {
    let max = tool.parts.config.subagent.max_parallel.max(1);
    // Foreground calls wait for a place, background calls fail at once.
    let slot = tool.children.acquire(max, !spec.background, ctx.cancel)?;
    let (agent, mut conv) = build(tool, spec)?;
    if spec.background {
        return background(tool, spec, agent, conv, slot);
    }
    // Its own stop, steer and pause (PLAN.md D79); the parent's stop reaches it.
    let (steer, mut inbox) = Inbox::channel();
    let own = Arc::new(AtomicBool::new(false));
    let mut fwd = Forward::new(&tool.children, &spec.label);
    let output = linked(ctx.cancel, &own, || {
        let prompt = &spec.prompt;
        run(
            &agent,
            &mut conv,
            prompt,
            (&own, steer),
            &mut inbox,
            &mut fwd,
        )
    });
    drop(slot);
    Ok(output)
}

/// The child agent and its new session one level below the parent. Below
/// `maxDepth` it gets the parent's tool list (so a child of the same model
/// reads the parent's cached prefix), at `maxDepth` the built-in tools only.
fn build(tool: &SubagentTool, spec: &Spec) -> Result<(Agent, Conversation), String> {
    let depth = tool.depth + 1;
    let mut parts = tool.parts.clone();
    // The parent's frozen system, not the one its parts were built with.
    if let Some(system) = children::of_session(&tool.session).system() {
        parts.system = system;
    }
    // A subagent session is short: `cacheTtl: auto` gives its messages the
    // 5 minute cache (PLAN.md D36).
    parts.config.interactive = false;
    let settings = parts.config.subagent.clone();
    if let Some(model) = spec.model.as_deref().or(settings.model.as_deref()) {
        route(tool, &mut parts, model.trim())?;
    }
    if let Some(effort) = spec.effort.as_deref().or(settings.effort.as_deref()) {
        parts.config.set_effort(effort);
    }
    // An agent type's `maxDepth` may only lower the setting.
    let max_depth = spec
        .max_depth
        .map_or(settings.max_depth, |m| m.min(settings.max_depth));
    let nested = depth < max_depth;
    check_tools(&spec.tools, nested, &parts.extensions)?;
    let conv = Conversation::create_at(&tool.app_home, &parts.cwd, Some(&tool.session), depth)
        .map_err(|err| format!("cannot start a subagent session: {err}"))?;
    let home = &tool.app_home;
    let mut agent = match nested {
        true => Agent::with_subagents(parts, home, &conv.id, depth, tool.router.clone()),
        false => {
            let registry = parts.extensions.registry();
            Agent::new(parts, Arc::new(registry))
        }
    };
    agent.preload = spec.tools.clone();
    // The child asks through the session's gate, named (PLAN.md D64).
    if let Some(parent) = tool.children.approver() {
        agent.set_approver(parent.child(&spec.label, &conv.id));
    }
    Ok((agent, conv))
}

/// Points `parts` at `model`: through the router, else only a model of the
/// parent's provider.
fn route(tool: &SubagentTool, parts: &mut AgentParts, model: &str) -> Result<(), String> {
    if model.is_empty() || model == parts.config.model {
        return Ok(());
    }
    if let Some(note) = catalog::retired_note(model, &parts.config.model) {
        return Err(note);
    }
    match &tool.router {
        Some(router) => (parts.provider, parts.auth) = router(model)?,
        None if parts.catalog.provider_of(model)
            == parts.catalog.provider_of(&parts.config.model) => {}
        None => {
            return Err(format!(
                "model `{model}` needs a provider this session lacks"
            ))
        }
    }
    parts.config.model = model.to_string();
    Ok(())
}

/// Every name in `names` must be a tool of the child (not `find_tools`).
fn check_tools(names: &[String], nested: bool, ext: &tools::Extensions) -> Result<(), String> {
    let builtin = ext.registry();
    let mut known: Vec<&str> = builtin.specs()[1..]
        .iter()
        .map(|s| s.name.as_str())
        .collect();
    if nested {
        known.push(NAME);
        known.extend(child_tools::NAMES);
    }
    let unknown: Vec<&str> = names
        .iter()
        .map(String::as_str)
        .filter(|name| !known.contains(name))
        .collect();
    if unknown.is_empty() {
        return Ok(());
    }
    Err(format!(
        "`tools`: unknown {} (the subagent has {})",
        unknown.join(", "),
        known.join(", ")
    ))
}

fn background(
    tool: &SubagentTool,
    spec: &Spec,
    agent: Agent,
    mut conv: Conversation,
    slot: Slot,
) -> Result<ToolOutput, String> {
    let (steer, mut inbox) = Inbox::channel();
    let cancel = Arc::new(AtomicBool::new(false));
    let id = tool.children.add_kid(cancel.clone(), steer.clone());
    let session = conv.id.clone();
    let children = tool.children.clone();
    let (prompt, label, kid) = (spec.prompt.clone(), spec.label.clone(), id.clone());
    let budget = agent.parts.config.subagent.output_bytes;
    let job = move || {
        let _slot = slot;
        let mut fwd = Forward::new(&children, &label);
        let controls = (&cancel, steer);
        let output = run(&agent, &mut conv, &prompt, controls, &mut inbox, &mut fwd);
        children.finish_kid(&kid, output.into_text(), budget);
    };
    let thread = std::thread::Builder::new().name(format!("subagent-{id}"));
    if let Err(err) = thread.spawn(job) {
        let message = format!("cannot start a background subagent: {err}");
        tool.children.finish_kid(&id, message.clone(), budget);
        return Err(message);
    }
    Ok(ToolOutput::ok(
        json!({"id": id, "session": session}).to_string(),
    ))
}

/// One turn of the child; its events feed the stats and the parent's
/// progress lines, its last answer and the stats line make the output.
fn run(
    agent: &Agent,
    conv: &mut Conversation,
    prompt: &str,
    (cancel, steer): (&Arc<AtomicBool>, Sender<Input>),
    inbox: &mut Inbox,
    fwd: &mut Forward,
) -> ToolOutput {
    fwd.session = conv.id.clone();
    // A front end showing the child steers, pauses and stops it (D79).
    let _live = live::run(&conv.id, cancel.clone(), steer);
    let running = |running| AgentEvent::Subagent {
        session: conv.id.clone(),
        running,
    };
    fwd.parent.push_event(running(true));
    let outcome = agent.run_turn_with(conv, prompt, cancel, inbox, &mut |e| fwd.event(e));
    let text = conv
        .messages
        .iter()
        .rev()
        .find(|m| m.role == Role::Assistant)
        .map(|m| m.text())
        .unwrap_or_default();
    fwd.parent.push_event(AgentEvent::Subagent {
        session: conv.id.clone(),
        running: false,
    });
    result(text, &fwd.stats.line(&conv.id), outcome, &fwd.errors)
}

/// Runs `f` while a watcher copies the parent's stop into the child's own
/// flag, so a stop of the parent stops the child and not the other way.
fn linked<T>(parent: &AtomicBool, own: &AtomicBool, f: impl FnOnce() -> T) -> T {
    let done = AtomicBool::new(false);
    std::thread::scope(|scope| {
        scope.spawn(|| {
            while !done.load(Ordering::Relaxed) {
                if parent.load(Ordering::Relaxed) {
                    own.store(true, Ordering::SeqCst);
                    return;
                }
                std::thread::sleep(Duration::from_millis(50));
            }
        });
        let out = f();
        done.store(true, Ordering::Relaxed);
        out
    })
}

/// The child's events as the parent sees them.
struct Forward {
    parent: Arc<Children>,
    session: String,
    label: String,
    stats: ChildStats,
    errors: Vec<String>,
    step: u32,
    tool: Option<String>,
    sent: Option<Instant>,
    dirty: bool,
}

impl Forward {
    fn new(parent: &Arc<Children>, label: &str) -> Forward {
        Forward {
            parent: parent.clone(),
            session: String::new(),
            label: label.to_string(),
            stats: ChildStats::default(),
            errors: Vec::new(),
            step: 0,
            tool: None,
            sent: None,
            dirty: false,
        }
    }

    /// Descendants' subagent events go up unchanged; the child's own
    /// steps and tool calls become a progress line, at most every 500 ms.
    fn event(&mut self, event: AgentEvent) {
        live::publish(&self.session, &event);
        match &event {
            AgentEvent::Subagent { .. }
            | AgentEvent::SubagentProgress { .. }
            | AgentEvent::ApprovalRequest(_)
            | AgentEvent::ApprovalDone { .. } => {
                return self.parent.push_event(event);
            }
            AgentEvent::StepStart { step, .. } => (self.step, self.tool) = (*step, None),
            AgentEvent::ToolCall { name, .. } => self.tool = Some(name.clone()),
            AgentEvent::Error(message) => self.errors.push(message.clone()),
            _ => {}
        }
        let moved = matches!(
            event,
            AgentEvent::StepStart { .. } | AgentEvent::ToolCall { .. }
        );
        self.dirty |= moved;
        self.stats.record(event);
        let due = match self.sent {
            Some(at) => at.elapsed() >= PROGRESS_EVERY,
            None => true,
        };
        if self.dirty && due {
            self.dirty = false;
            self.sent = Some(Instant::now());
            let line = self.line();
            let session = self.session.clone();
            self.parent
                .push_event(AgentEvent::SubagentProgress { session, line });
        }
    }

    /// `label · step N · tool` without the empty parts.
    fn line(&self) -> String {
        let mut parts: Vec<String> = Vec::new();
        if !self.label.is_empty() {
            parts.push(self.label.clone());
        }
        parts.push(format!("step {}", self.step));
        parts.extend(self.tool.clone());
        parts.join(" · ")
    }
}

/// What the child agent did, for the last line of the tool output.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct ChildStats {
    /// The model of the last request.
    pub model: String,
    pub steps: u32,
    /// Distinct tool names in first-use order.
    pub tools: Vec<String>,
    pub usage: Usage,
}

impl ChildStats {
    pub fn record(&mut self, event: AgentEvent) {
        match event {
            AgentEvent::Request { model, .. } => self.model = model,
            AgentEvent::StepEnd { .. } => self.steps += 1,
            AgentEvent::ToolCall { name, .. } if !self.tools.contains(&name) => {
                self.tools.push(name)
            }
            AgentEvent::TurnEnd { usage, .. } => self.usage = usage,
            _ => {}
        }
    }

    /// `[subagent session ID · model M · N steps · tools a, b · in I out O]`
    /// without the empty parts.
    pub fn line(&self, session: &str) -> String {
        let mut parts = vec![format!("session {session}")];
        if !self.model.is_empty() {
            parts.push(format!("model {}", self.model));
        }
        match self.steps {
            0 => {}
            1 => parts.push("1 step".to_string()),
            n => parts.push(format!("{n} steps")),
        }
        if !self.tools.is_empty() {
            parts.push(format!("tools {}", self.tools.join(", ")));
        }
        if self.usage.input > 0 || self.usage.output > 0 {
            parts.push(format!("in {} out {}", self.usage.input, self.usage.output));
        }
        format!("[subagent {}]", parts.join(" · "))
    }
}

fn result(text: String, last: &str, outcome: TurnOutcome, errors: &[String]) -> ToolOutput {
    let mut content = text.trim_end().to_string();
    if outcome != TurnOutcome::Done {
        let detail = errors.last().map(|e| format!(": {e}")).unwrap_or_default();
        content.push_str(&format!("\n\nsubagent ended {outcome:?}{detail}"));
    }
    content.push_str(&format!("\n\n{last}"));
    ToolOutput::new(content.trim_start(), outcome != TurnOutcome::Done)
}
