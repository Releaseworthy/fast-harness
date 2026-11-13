//! The approval gate (PLAN.md D64): with `approvals: ask`, a `Write` or
//! `Run` call the allow list does not cover waits, after its PreToolUse
//! hooks, for a front end's answer. The wait ends on the answer, on cancel,
//! on pause and when the session closes (a cancel); a session nobody
//! answers for refuses at once.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Condvar, Mutex, MutexGuard};
use std::time::Duration;

use super::tool_run::CANCELLED;
use super::{child_tools, Agent, SUBAGENT};
use crate::config::approvals::{self, Approvals, Subject};
use crate::events::{AgentEvent, ApprovalRequest};
use crate::tools::ToolOutput;

/// The longest sleep of a waiting call between looks at its own cancel
/// flag (a child stopped by `agent_stop`); answers, cancel and pause of
/// the session wake it at once.
const BACKSTOP: Duration = Duration::from_millis(500);

/// Where each decided request goes: the tool thread pushes `(call id,
/// decision)` and the agent thread writes them as `tool/approval`
/// (trace version 11, PLAN.md D64).
pub(super) type Decisions<'a> = &'a Mutex<Vec<(String, String)>>;

/// A person's answer to one request.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Answer {
    /// Run this call.
    Once,
    /// Run it and, until the session closes, every call of the same tool
    /// with the same command (`shell`, whitespace collapsed), the same
    /// normal path (`write`, `edit`) or the same arguments (other tools).
    Session,
    /// Do not run it; the model reads that the user declined, and why.
    No(Option<String>),
}

impl Answer {
    fn word(&self) -> &'static str {
        match self {
            Answer::Once => "once",
            Answer::Session => "session",
            Answer::No(_) => "declined",
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Stop {
    Cancel,
    Pause,
}

#[derive(Default)]
struct State {
    pending: Vec<ApprovalRequest>,
    answers: Vec<(String, Answer)>,
    /// `(tool, subject key)` allowed for the rest of the session.
    grants: Vec<(String, String)>,
    stop: Option<Stop>,
    /// A front end answers requests; without one a request is refused.
    answering: bool,
    next: u64,
}

/// One session's requests, answers and session grants; shared by the
/// service's `Session`, the turn's agent and its subagents.
#[derive(Default)]
pub struct Gate {
    state: Mutex<State>,
    changed: Condvar,
}

impl Gate {
    fn lock(&self) -> MutexGuard<'_, State> {
        self.state.lock().unwrap_or_else(|p| p.into_inner())
    }

    /// A front end that shows requests and answers them says so; until
    /// then every request is refused at once.
    pub fn set_answering(&self, on: bool) {
        self.lock().answering = on;
    }

    /// The requests waiting for an answer, oldest first.
    pub fn pending(&self) -> Vec<ApprovalRequest> {
        self.lock().pending.clone()
    }

    /// Answers request `id`; false when no request `id` waits (unknown,
    /// answered already, or ended by a cancel or pause).
    pub fn answer(&self, id: &str, answer: Answer) -> bool {
        let mut state = self.lock();
        let waits = state.pending.iter().any(|r| r.id == id);
        if !waits || state.answers.iter().any(|(a, _)| a == id) {
            return false;
        }
        state.answers.push((id.to_string(), answer));
        self.changed.notify_all();
        true
    }

    /// Stop: every waiting call ends cancelled, and so does every request
    /// until [`Gate::reset`].
    pub fn cancel(&self) {
        self.lock().stop = Some(Stop::Cancel);
        self.changed.notify_all();
    }

    /// Pause: every waiting call ends without running (a cancel stays).
    pub fn pause(&self) {
        let mut state = self.lock();
        state.stop.get_or_insert(Stop::Pause);
        self.changed.notify_all();
    }

    /// A new turn starts: no stop.
    pub fn reset(&self) {
        self.lock().stop = None;
    }

    fn granted(&self, tool: &str, key: &str) -> bool {
        (self.lock().grants.iter()).any(|(t, k)| t == tool && k == key)
    }

    /// Waits for the answer to `request` (its id is set here); `decided`
    /// takes `(call id, decision)` for the session log (version 11, D64).
    fn ask(
        &self,
        mut request: ApprovalRequest,
        key: &str,
        cancel: &AtomicBool,
        emit: &dyn Fn(AgentEvent),
        decided: Decisions<'_>,
    ) -> Option<ToolOutput> {
        {
            let mut state = self.lock();
            if let Some(stop) = state.stop {
                return Some(stopped(stop));
            }
            state.next += 1;
            request.id = format!("ap{}", state.next);
            state.pending.push(request.clone());
        }
        let (id, tool) = (request.id.clone(), request.tool.clone());
        let call = request.call_id.clone();
        emit(AgentEvent::ApprovalRequest(request));
        let ended = self.wait(&id, cancel);
        let word = match &ended {
            Ok(answer) => answer.word(),
            Err(Stop::Cancel) => "cancelled",
            Err(Stop::Pause) => "paused",
        };
        let answer = word.to_string();
        (decided.lock().unwrap_or_else(|p| p.into_inner())).push((call, answer.clone()));
        emit(AgentEvent::ApprovalDone { id, answer });
        match ended {
            Ok(Answer::Once) => None,
            Ok(Answer::Session) => {
                self.lock().grants.push((tool, key.to_string()));
                None
            }
            Ok(Answer::No(reason)) => Some(declined(reason.as_deref())),
            Err(stop) => Some(stopped(stop)),
        }
    }

    /// Blocks on the condition variable until `id` is answered or a stop
    /// arrives; the request leaves the pending list either way.
    fn wait(&self, id: &str, cancel: &AtomicBool) -> Result<Answer, Stop> {
        let mut state = self.lock();
        loop {
            if let Some(at) = state.answers.iter().position(|(a, _)| a == id) {
                state.pending.retain(|r| r.id != id);
                return Ok(state.answers.remove(at).1);
            }
            let own = cancel.load(Ordering::Relaxed).then_some(Stop::Cancel);
            if let Some(stop) = state.stop.or(own) {
                state.pending.retain(|r| r.id != id);
                return Err(stop);
            }
            state = match self.changed.wait_timeout(state, BACKSTOP) {
                Ok((guard, _)) => guard,
                Err(poisoned) => poisoned.into_inner().0,
            };
        }
    }
}

/// The gate an agent asks through and who asks.
#[derive(Clone)]
pub struct Approver {
    pub gate: Arc<Gate>,
    /// The subagent (`label (session id)`); `None` for the session.
    pub agent: Option<String>,
}

impl Approver {
    pub fn new(gate: Arc<Gate>) -> Approver {
        Approver { gate, agent: None }
    }

    /// The same gate for a child session.
    pub(super) fn child(&self, label: &str, session: &str) -> Approver {
        let agent = match label.trim() {
            "" => session.to_string(),
            label => format!("{label} ({session})"),
        };
        Approver {
            gate: Arc::clone(&self.gate),
            agent: Some(agent),
        }
    }
}

/// The text of a call refused because nobody can answer.
pub const REFUSED: &str = "not run: approvals are on (`\"approvals\": \"ask\"`) and nobody can answer in this session; tell the user what you wanted to do: they can allow it in the `allow` list (`commands`, `paths`, `tools`) of ~/.fast-harness/config.json";
/// The text of a call that waited when a pause arrived.
pub const PAUSED: &str = "not run: the turn was paused while this call waited for approval; call it again when the turn goes on";

fn declined(reason: Option<&str>) -> ToolOutput {
    match reason.map(str::trim).filter(|r| !r.is_empty()) {
        Some(reason) => ToolOutput::err(format!("not run: the user declined this call: {reason}")),
        None => ToolOutput::err("not run: the user declined this call"),
    }
}

fn stopped(stop: Stop) -> ToolOutput {
    match stop {
        Stop::Cancel => ToolOutput::err(CANCELLED),
        Stop::Pause => ToolOutput::err(PAUSED),
    }
}

/// The subagent tools run no program themselves; their children ask.
fn exempt(tool: &str) -> bool {
    tool == SUBAGENT || child_tools::NAMES.contains(&tool)
}

/// The gate for one call with its final arguments `raw`: `None` runs it,
/// `Some` is its result instead. With `approvals: off` it costs one
/// comparison. `decisions` takes every decided request for the log.
pub(super) fn check(
    agent: &Agent,
    (call_id, tool, raw): (&str, &str, &str),
    cancel: &AtomicBool,
    emit: &dyn Fn(AgentEvent),
    decisions: Decisions<'_>,
) -> Option<ToolOutput> {
    let config = &agent.parts.config;
    if config.approvals == Approvals::Off || exempt(tool) {
        return None;
    }
    let (effect, cwd) = (agent.tools.effect(tool), &agent.parts.cwd);
    if approvals::covered(&config.allow, tool, effect, raw, cwd) {
        return None;
    }
    let Subject { key, summary } = approvals::subject(tool, raw, cwd);
    let approver = agent.approver.as_ref();
    if approver.is_some_and(|a| a.gate.granted(tool, &key)) {
        return None;
    }
    let request = ApprovalRequest {
        id: String::new(),
        call_id: call_id.to_string(),
        tool: tool.to_string(),
        effect: effect.unwrap_or(crate::tools::Effect::Run),
        summary,
        arguments: raw.to_string(),
        agent: approver.and_then(|a| a.agent.clone()),
    };
    match approver.filter(|a| a.gate.lock().answering) {
        Some(approver) => approver.gate.ask(request, &key, cancel, emit, decisions),
        None => {
            let id = format!("refused-{call_id}");
            let answer = "refused".to_string();
            let decided = (call_id.to_string(), answer.clone());
            (decisions.lock().unwrap_or_else(|p| p.into_inner())).push(decided);
            emit(AgentEvent::ApprovalRequest(ApprovalRequest {
                id: id.clone(),
                ..request
            }));
            emit(AgentEvent::ApprovalDone { id, answer });
            Some(ToolOutput::err(REFUSED))
        }
    }
}

impl Agent {
    /// The gate this agent's calls ask through; its subagents get it too.
    pub fn set_approver(&mut self, approver: Approver) {
        if let Some(children) = &self.children {
            children.set_approver(approver.clone());
        }
        self.approver = Some(approver);
    }
}
