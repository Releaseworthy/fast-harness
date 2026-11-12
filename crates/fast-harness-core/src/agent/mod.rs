//! The turn and step loop, retries, failover, events and the subagent tool.
//!
//! One turn: the user message, then steps (one model call each, with retries
//! and account failover) until the model stops asking for tools. The tool
//! calls of one step run in parallel on scoped threads; only the thread that
//! calls [`Agent::run_turn`] writes the session log, and every traced event is
//! written before the matching [`AgentEvent`] is sent.

mod call;
mod compact;
pub mod context;
mod conversation;
mod inbox;
pub mod prompt;
pub mod prompt_store;
mod subagent;
mod tool_run;
mod turn;

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

use crate::auth::AuthSource;
use crate::config::Config;
use crate::events::{AgentEvent, TurnOutcome};
use crate::http::Transport;
use crate::llm::Provider;
use crate::tools::{self, Registry};
use turn::{Io, Start};

pub use compact::{facts, COMMAND as COMPACT_COMMAND, NO_TOOLS, SUMMARY_REQUEST};
pub use context::Context;
pub use conversation::{repair, Conversation, MISSING_RESULT, NOTHING_TO_RESUME};
pub use inbox::{Inbox, Input, CONTEXT as CONTEXT_KIND, STEER as STEER_KIND};
pub use subagent::{ChildStats, SubagentTool};

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
}

pub struct Agent {
    pub parts: AgentParts,
    pub tools: Arc<Registry>,
    /// The prompt sources read when this agent was built; a session whose
    /// stamp differs gets a context update. `None`: no updates.
    pub context: Option<Context>,
}

impl Agent {
    pub fn new(parts: AgentParts, tools: Arc<Registry>) -> Agent {
        Agent {
            parts,
            tools,
            context: None,
        }
    }

    /// This agent comparing `context` with each session's stamp.
    pub fn with_context(mut self, context: Context) -> Agent {
        self.context = Some(context);
        self
    }

    /// The built-in tools plus `subagent`, whose child sessions name
    /// `parent_session` as their parent.
    pub fn with_subagent(parts: AgentParts, app_home: &Path, parent_session: &str) -> Agent {
        let mut registry = tools::builtin();
        let tool = SubagentTool::new(parts.clone(), app_home, parent_session);
        registry.add(Box::new(tool));
        Agent::new(parts, Arc::new(registry))
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

    fn turns(
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
            if outcome != TurnOutcome::Done || cancel.load(Ordering::Relaxed) {
                return outcome;
            }
            let Some(text) = inbox.next_turn() else {
                return outcome;
            };
            sink(AgentEvent::UserInput {
                text: text.clone(),
                steer: false,
            });
            follow_up = Some(text);
        }
    }
}
