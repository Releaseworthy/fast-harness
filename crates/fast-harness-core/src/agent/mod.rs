//! The turn and step loop, retries, failover, events and the subagent tool.
//!
//! One turn: the user message, then steps (one model call each, with retries
//! and account failover) until the model stops asking for tools. The tool
//! calls of one step run in parallel on scoped threads; only the thread that
//! calls [`Agent::run_turn`] writes the session log, and every traced event is
//! written before the matching [`AgentEvent`] is sent.

pub mod approval;
mod call;
mod child_run;
mod child_tools;
pub mod children;
mod compact;
pub mod context;
mod conversation;
pub mod hooks;
mod inbox;
mod inbox_file;
pub mod live;
pub mod memory;
mod model_hook;
pub mod nested;
mod preload;
pub mod prompt;
pub mod prompt_store;
pub mod session_tools;
mod spawn_models;
mod subagent;
mod tool_run;
mod turn;
mod waiting;
pub mod worker;

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

use crate::auth::AuthSource;
use crate::config::Config;
use crate::events::{AgentEvent, TurnOutcome};
use crate::http::Transport;
use crate::llm::Provider;
use crate::session::lock;
use crate::tools::{self, Registry};
pub use approval::{Answer, Approver, Gate};
pub use child_run::ChildStats;
pub use child_tools::{OUTPUT as AGENT_OUTPUT, SEND as AGENT_SEND, STOP as AGENT_STOP};
pub use children::Children;
use turn::{Io, Start};

pub use compact::{facts, COMMAND as COMPACT_COMMAND, NO_TOOLS, SUMMARY_REQUEST};
pub use context::Context;
pub use conversation::{repair, Conversation, MISSING_RESULT, NOTHING_TO_RESUME};
pub use inbox::{Inbox, Input, CONTEXT as CONTEXT_KIND, STEER as STEER_KIND};
pub use subagent::{SubagentTool, NAME as SUBAGENT};

/// The note of a turn refused because another process runs a turn of this
/// session (its live `<id>.lock`, PLAN.md D49): two writers would interleave
/// their events in one log. Nothing is logged. A subagent session has its
/// lock too (PLAN.md D79): a send_message to it steers, never a 2nd turn.
fn running_elsewhere(conv: &Conversation) -> Option<String> {
    let lock = lock::live_lock(&conv.log_path).filter(|lock| lock.pid != std::process::id())?;
    Some(lock::busy_note(&conv.id, &lock))
}

/// The provider and credential source of a model id (`<provider id>/<id>`
/// included); a subagent with another model than its parent uses it.
pub type Router =
    Arc<dyn Fn(&str) -> Result<(Arc<dyn Provider>, Arc<dyn AuthSource>), String> + Send + Sync>;

/// Everything an agent needs besides its tools; cheap to clone (shared parts
/// are `Arc`s), so a subagent reuses its parent's.
#[derive(Clone)]
pub struct AgentParts {
    pub provider: Arc<dyn Provider>,
    pub transport: Arc<dyn Transport>,
    pub auth: Arc<dyn AuthSource>,
    pub config: Config,
    /// Working directory of the tools.
    pub cwd: PathBuf,
    /// The system prompt of a new session; a session that has one keeps
    /// its own (PLAN.md D36).
    pub system: String,
    /// The tool, agent and command files of the settings (PLAN.md D42);
    /// every registry of the session and of its children is built from it.
    pub extensions: Arc<tools::Extensions>,
    /// The runtime's model facts (PLAN.md D67): context windows, output
    /// caps and the provider of a model id.
    pub catalog: crate::llm::Catalog,
}

pub struct Agent {
    pub parts: AgentParts,
    pub tools: Arc<Registry>,
    /// The prompt sources read when this agent was built; a session whose
    /// stamp differs gets a context update. `None`: no updates.
    pub context: Option<Context>,
    /// Tools found from the start of a new session (a subagent's `tools`,
    /// PLAN.md D40): its first turn starts with a `find_tools` call naming them.
    pub preload: Vec<String>,
    /// The subagents of this agent's session; `None` without the tool.
    pub children: Option<Arc<Children>>,
    /// The approval gate of the session (PLAN.md D64), set by the service;
    /// `None` refuses what `approvals: ask` would ask.
    pub approver: Option<Approver>,
}

impl Agent {
    pub fn new(parts: AgentParts, tools: Arc<Registry>) -> Agent {
        Agent {
            parts,
            tools,
            context: None,
            preload: Vec::new(),
            children: None,
            approver: None,
        }
    }

    /// This agent comparing `context` with each session's stamp.
    pub fn with_context(mut self, context: Context) -> Agent {
        self.context = Some(context);
        self
    }

    /// The built-in tools plus `subagent` and the background tools, for a
    /// session at depth 0 whose children use the parent's provider.
    pub fn with_subagent(parts: AgentParts, app_home: &Path, parent_session: &str) -> Agent {
        Agent::with_subagents(parts, app_home, parent_session, 0, None)
    }

    /// The built-in tools, then `subagent`, `agent_send`, `agent_stop` and
    /// `agent_output` (PLAN.md D40) for `session` at nesting `depth`; a
    /// child with another model routes it through `router`. A child below
    /// `maxDepth` gets the same list, so its tools match its parent's.
    pub fn with_subagents(
        parts: AgentParts,
        app_home: &Path,
        session: &str,
        depth: u32,
        router: Option<Router>,
    ) -> Agent {
        let children = children::of_session(session);
        let ext = Arc::clone(&parts.extensions);
        let mut registry = ext.builtin();
        let tool = SubagentTool::new(parts.clone(), app_home, session, depth, router);
        ext.add(&mut registry, Box::new(tool));
        for tool in child_tools::all(&children) {
            ext.add(&mut registry, tool);
        }
        // Other sessions through their lock and inbox files (PLAN.md D49).
        for tool in session_tools::all(app_home, worker::default_launcher()) {
            ext.add(&mut registry, tool);
        }
        ext.add_commands(&mut registry);
        let mut agent = Agent::new(parts, Arc::new(registry));
        agent.children = Some(children);
        agent
    }

    /// Runs one turn on `conv` with no inbox. `cancel` may be set from
    /// another thread; the turn then ends `Cancelled` as soon as the model
    /// call, the waits and the tools notice it. `sink` sees the events in
    /// order, `TurnEnd` last. `/compact [instructions]` runs a turn that only
    /// compacts (PLAN.md D38).
    pub fn run_turn(
        &self,
        conv: &mut Conversation,
        user_text: &str,
        cancel: &AtomicBool,
        sink: &mut dyn FnMut(AgentEvent),
    ) -> TurnOutcome {
        self.run_turn_with(conv, user_text, cancel, &mut Inbox::empty(), sink)
    }

    /// [`Agent::run_turn`] reading `inbox` at every step boundary (PLAN.md
    /// D39): steers join the history there, a pause ends the turn `Paused`
    /// after the step's tools, and each follow-up starts a new turn after
    /// one that ended `Done`. Returns the outcome of the last turn; texts no
    /// turn used stay in [`Inbox::leftover`].
    pub fn run_turn_with(
        &self,
        conv: &mut Conversation,
        user_text: &str,
        cancel: &AtomicBool,
        inbox: &mut Inbox,
        sink: &mut dyn FnMut(AgentEvent),
    ) -> TurnOutcome {
        self.turns(conv, Start::Text(user_text), cancel, inbox, sink)
    }

    /// Continues a turn that was paused, cancelled or cut by a crash: the
    /// model is called on the history as it is (ending with tool results or
    /// user text), without a new user message. A history that ends with the
    /// model's answer has nothing to resume: an error, outcome `Failed`, and
    /// nothing is logged.
    pub fn resume_turn(
        &self,
        conv: &mut Conversation,
        cancel: &AtomicBool,
        inbox: &mut Inbox,
        sink: &mut dyn FnMut(AgentEvent),
    ) -> TurnOutcome {
        if !conv.can_resume() {
            sink(AgentEvent::Error(NOTHING_TO_RESUME.into()));
            return TurnOutcome::Failed;
        }
        self.turns(conv, Start::Resume, cancel, inbox, sink)
    }

    /// The turns, then on cancel every background descendant is stopped.
    fn turns(
        &self,
        conv: &mut Conversation,
        first: Start<'_>,
        cancel: &AtomicBool,
        inbox: &mut Inbox,
        sink: &mut dyn FnMut(AgentEvent),
    ) -> TurnOutcome {
        // One turn at a time on a subagent session in this process (D79).
        let _entered = match conv.depth {
            0 => None,
            _ => match live::enter(&conv.id) {
                Ok(entered) => Some(entered),
                Err(note) => {
                    sink(AgentEvent::Error(note));
                    return TurnOutcome::Failed;
                }
            },
        };
        // A lost take (another process took the lock since the check) is
        // refused the same way.
        let refused = running_elsewhere(conv).map(Err);
        if let Err(note) = refused.unwrap_or_else(|| inbox.attach(conv, &self.parts.cwd)) {
            sink(AgentEvent::Error(note));
            return TurnOutcome::Failed;
        }
        let outcome = self.turn_loop(conv, first, cancel, inbox, sink);
        if cancel.load(Ordering::Relaxed) || inbox.stopped() {
            self.stop_children(sink);
        }
        inbox.detach();
        outcome
    }

    fn turn_loop(
        &self,
        conv: &mut Conversation,
        first: Start<'_>,
        cancel: &AtomicBool,
        inbox: &mut Inbox,
        sink: &mut dyn FnMut(AgentEvent),
    ) -> TurnOutcome {
        let mut follow_up: Option<String> = None;
        loop {
            let start = match &follow_up {
                Some(text) => Start::Text(text),
                None => first,
            };
            let io = Io {
                cancel,
                inbox: &mut *inbox,
                sink: &mut *sink,
            };
            let outcome = turn::run(self, conv, start, io);
            if outcome != TurnOutcome::Done || cancel.load(Ordering::Relaxed) || inbox.stopped() {
                return outcome;
            }
            inbox.boundary(conv.turn, 0);
            // Background children still running: wait for their answers.
            let Some((text, steer)) = self.next_input(cancel, inbox, sink) else {
                return outcome;
            };
            sink(AgentEvent::UserInput {
                text: text.clone(),
                steer,
            });
            follow_up = Some(text);
        }
    }
}
