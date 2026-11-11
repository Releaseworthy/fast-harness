//! The turn and step loop, retries, failover, events and the subagent tool.
//!
//! One turn: the user message, then steps (one model call each, with retries
//! and account failover) until the model stops asking for tools. The tool
//! calls of one step run in parallel on scoped threads; only the thread that
//! calls [`Agent::run_turn`] writes the session log, and every traced event is
//! written before the matching [`AgentEvent`] is sent.

mod call;
mod conversation;
pub mod prompt;
pub mod prompt_store;
mod subagent;
mod tool_run;
mod turn;

use std::path::{Path, PathBuf};
use std::sync::atomic::AtomicBool;
use std::sync::Arc;

use crate::auth::AuthSource;
use crate::config::Config;
use crate::events::{AgentEvent, TurnOutcome};
use crate::http::Transport;
use crate::llm::Provider;
use crate::tools::{self, Registry};

pub use conversation::{repair, Conversation, MISSING_RESULT};
pub use subagent::SubagentTool;

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
    pub system: String,
}

pub struct Agent {
    pub parts: AgentParts,
    pub tools: Arc<Registry>,
}

impl Agent {
    pub fn new(parts: AgentParts, tools: Arc<Registry>) -> Agent {
        Agent { parts, tools }
    }

    /// The built-in tools plus `subagent`, whose child sessions name
    /// `parent_session` as their parent.
    pub fn with_subagent(parts: AgentParts, app_home: &Path, parent_session: &str) -> Agent {
        let mut registry = tools::builtin();
        let tool = SubagentTool::new(parts.clone(), app_home, parent_session);
        registry.add(Box::new(tool));
        Agent::new(parts, Arc::new(registry))
    }

    /// Runs one turn on `conv`. `cancel` may be set from another thread; the
    /// turn then ends `Cancelled` as soon as the model call, the waits and
    /// the tools notice it. `sink` sees the events in order, `TurnEnd` last.
    pub fn run_turn(
        &self,
        conv: &mut Conversation,
        user_text: &str,
        cancel: &AtomicBool,
        sink: &mut dyn FnMut(AgentEvent),
    ) -> TurnOutcome {
        turn::run(self, conv, user_text, cancel, sink)
    }
}
