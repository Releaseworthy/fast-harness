//! One turn of a served session on its own thread (PLAN.md D48).

use std::sync::atomic::AtomicBool;
use std::sync::Arc;
use std::time::Instant;

use super::{busy, Host, Service, ServiceError, Session};
use crate::agent::{Conversation, Inbox};
use crate::config::Config;
use crate::events::{AgentEvent, TurnOutcome};
use crate::types::Usage;

/// How a turn starts.
pub enum Start {
    /// A new user message (a `/compact` text compacts, PLAN.md D38).
    Text(String),
    /// Goes on with a paused, cancelled or cut turn (`Agent::resume_turn`).
    Resume,
}

/// How a turn ended.
#[derive(Clone, Debug)]
pub struct Finished {
    pub outcome: TurnOutcome,
    /// Every turn of the run (follow-ups included).
    pub usage: Usage,
    pub duration_ms: u64,
    /// The last error event, if any.
    pub error: Option<String>,
    /// Steers and follow-ups no turn used (after a cancel or a pause).
    pub leftover: Vec<String>,
    /// Notes of the agent's build (an untrusted project).
    pub notes: Vec<String>,
}

/// Sees each event of the turn, in order.
pub type Sink = Box<dyn FnMut(AgentEvent) + Send>;
/// Runs on the turn thread once the session is idle again.
pub type Done = Box<dyn FnOnce(&Finished) + Send>;

struct Job {
    host: Arc<dyn Host>,
    session: Arc<Session>,
    conv: Conversation,
    config: Config,
    cancel: Arc<AtomicBool>,
    inbox: Inbox,
}

impl Service {
    /// Starts a turn of `session` on a new thread. Returns the index the
    /// user message takes in the history (its ACP id, [`super::acp::user_id`]).
    /// `Busy` when a turn runs already: send it an `Input` instead.
    pub fn start(
        &self,
        session: &Arc<Session>,
        start: Start,
        sink: Sink,
        done: Done,
    ) -> Result<usize, ServiceError> {
        let mut state = session.lock();
        let conv = state.conv.take().ok_or_else(|| busy(&session.id))?;
        let index = conv.messages.len();
        let (tx, inbox) = Inbox::channel();
        let cancel = Arc::new(AtomicBool::new(false));
        state.inbox = Some(tx);
        state.cancel = Arc::clone(&cancel);
        if let Some(old) = state.worker.take() {
            // A thread that is still ending is left to end (dropped handle).
            if old.is_finished() {
                let _ = old.join();
            }
        }
        let job = Job {
            host: Arc::clone(&self.host),
            session: Arc::clone(session),
            conv,
            config: state.config.clone(),
            cancel,
            inbox,
        };
        let name = format!("turn-{}", session.id);
        let spawned = std::thread::Builder::new()
            .name(name)
            .spawn(move || job.run(start, sink, done));
        match spawned {
            Ok(handle) => {
                state.worker = Some(handle);
                Ok(index)
            }
            Err(err) => {
                // The conversation went with the closure: reopen it.
                let home = self.host.home();
                state.conv = Conversation::resume(&home, &session.cwd, &session.id).ok();
                state.inbox = None;
                Err(ServiceError::Failed(format!("turn thread: {err}")))
            }
        }
    }
}

impl Job {
    fn run(mut self, start: Start, mut sink: Sink, done: Done) {
        let started = Instant::now();
        let mut usage = Usage::default();
        let mut error = None;
        let mut notes = Vec::new();
        let outcome = {
            let mut seen = |event: AgentEvent| {
                match &event {
                    AgentEvent::TurnEnd { usage: one, .. } => usage.add(one),
                    AgentEvent::Error(message) => error = Some(message.clone()),
                    _ => {}
                }
                sink(event);
            };
            let built = self.host.agent(&self.session.cwd, &self.conv, &self.config);
            match built {
                Ok((agent, agent_notes)) => {
                    notes = agent_notes;
                    let (conv, cancel, inbox) = (&mut self.conv, &self.cancel, &mut self.inbox);
                    match start {
                        Start::Text(text) => {
                            agent.run_turn_with(conv, &text, cancel, inbox, &mut seen)
                        }
                        Start::Resume => agent.resume_turn(conv, cancel, inbox, &mut seen),
                    }
                }
                Err(err) => {
                    seen(AgentEvent::Error(err));
                    TurnOutcome::Failed
                }
            }
        };
        let finished = self.finish(outcome, usage, error, notes, started);
        done(&finished);
    }

    /// Gives the conversation back (the session is idle from here) and
    /// collects what the inbox still held.
    fn finish(
        mut self,
        outcome: TurnOutcome,
        usage: Usage,
        error: Option<String>,
        notes: Vec<String>,
        started: Instant,
    ) -> Finished {
        {
            let mut state = self.session.lock();
            state.inbox = None;
            state.conv = Some(self.conv);
            state.last = Some(outcome);
        }
        if let Ok(mut queued) = self.session.queued.lock() {
            queued.clear();
        }
        Finished {
            outcome,
            usage,
            duration_ms: started.elapsed().as_millis() as u64,
            error,
            leftover: self.inbox.leftover(),
            notes,
        }
    }
}
