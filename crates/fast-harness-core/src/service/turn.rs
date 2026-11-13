//! One turn of a session on its own thread (PLAN.md D48, D59), behind a
//! panic guard: a panic anywhere on the thread (the agent's build, the
//! loop, the sink) ends the turn `Failed` with an error event, and the
//! session is idle and usable again.

use std::any::Any;
use std::panic::{catch_unwind, AssertUnwindSafe};
use std::path::Path;
use std::sync::atomic::AtomicBool;
use std::sync::Arc;
use std::time::Instant;

use super::{busy, queue, Host, Service, ServiceError, Session};
use crate::agent::{Approver, Conversation, Inbox};
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
    /// Steers and follow-ups no turn used (after a cancel or a pause); they
    /// are first in the session's queue now ([`Session::status`]).
    pub leftover: Vec<String>,
    /// Notes of the agent's build (an untrusted project).
    pub notes: Vec<String>,
    /// The thread panicked; the conversation was reopened from its log.
    pub panicked: bool,
}

/// Sees each event of the turn, in order.
pub type Sink = Box<dyn FnMut(AgentEvent) + Send>;
/// Runs on the turn thread once the session is idle again, exactly once.
pub type Done = Box<dyn FnOnce(&Finished) + Send>;

struct Job {
    host: Arc<dyn Host>,
    session: Arc<Session>,
    conv: Conversation,
    config: Config,
    cancel: Arc<AtomicBool>,
    inbox: Inbox,
}

/// What the turn thread keeps across a panic: the sink and the numbers.
struct Tally {
    sink: Sink,
    session: Arc<Session>,
    usage: Usage,
    error: Option<String>,
    notes: Vec<String>,
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
        queue::begun(&mut state);
        session.gate().reset();
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
                state.conv = reopen(&self.host.home(), session);
                state.inbox = None;
                Err(ServiceError::Failed(format!("turn thread: {err}")))
            }
        }
    }
}

/// The session's conversation read again from its log, keeping the
/// session's background jobs (PLAN.md D68).
fn reopen(home: &Path, session: &Session) -> Option<Conversation> {
    let mut conv = Conversation::resume(home, &session.cwd, &session.id).ok()?;
    conv.tool_session.share_jobs(session.jobs());
    Some(conv)
}

impl Job {
    fn run(mut self, start: Start, sink: Sink, done: Done) {
        let started = Instant::now();
        let mut tally = Tally {
            sink,
            session: Arc::clone(&self.session),
            usage: Usage::default(),
            error: None,
            notes: Vec::new(),
        };
        let ran = catch_unwind(AssertUnwindSafe(|| self.turn(start, &mut tally)));
        let (outcome, panicked) = match ran {
            Ok(outcome) => (outcome, false),
            Err(payload) => {
                tally.panicked(payload.as_ref());
                (TurnOutcome::Failed, true)
            }
        };
        let finished = self.finish(outcome, panicked, tally, started);
        // The session is idle already: a panic in `done` is the front end's.
        let _ = catch_unwind(AssertUnwindSafe(|| done(&finished)));
    }

    fn turn(&mut self, start: Start, tally: &mut Tally) -> TurnOutcome {
        let built = self.host.agent(&self.session.cwd, &self.conv, &self.config);
        let mut agent = match built {
            Ok((agent, notes)) => {
                tally.notes = notes;
                agent
            }
            Err(err) => {
                tally.see(AgentEvent::Error(err));
                return TurnOutcome::Failed;
            }
        };
        agent.set_approver(Approver::new(self.session.gate()));
        let mut seen = |event: AgentEvent| tally.see(event);
        let (conv, cancel, inbox) = (&mut self.conv, &self.cancel, &mut self.inbox);
        match start {
            Start::Text(text) => agent.run_turn_with(conv, &text, cancel, inbox, &mut seen),
            Start::Resume => agent.resume_turn(conv, cancel, inbox, &mut seen),
        }
    }

    /// Gives the conversation back (the session is idle from here) and
    /// queues what the inbox still held. After a panic the conversation in
    /// memory may be half changed: the log's is taken when it reads.
    fn finish(
        mut self,
        outcome: TurnOutcome,
        panicked: bool,
        tally: Tally,
        started: Instant,
    ) -> Finished {
        let leftover = self.inbox.leftover();
        let mut conv = self.conv;
        if panicked {
            if let Some(fresh) = reopen(&self.host.home(), &self.session) {
                conv = fresh;
            }
        }
        {
            let mut state = self.session.lock();
            state.inbox = None;
            state.conv = Some(conv);
            state.last = Some(outcome);
            queue::ended(&mut state, outcome, &leftover);
        }
        let mut ids = (self.session.queued.lock()).unwrap_or_else(|p| p.into_inner());
        ids.clear();
        drop(ids);
        Finished {
            outcome,
            usage: tally.usage,
            duration_ms: started.elapsed().as_millis() as u64,
            error: tally.error,
            leftover,
            notes: tally.notes,
            panicked,
        }
    }
}

impl Tally {
    fn see(&mut self, event: AgentEvent) {
        match &event {
            AgentEvent::TurnEnd { usage: one, .. } => self.usage.add(one),
            AgentEvent::Error(message) => self.error = Some(message.clone()),
            AgentEvent::UserInput { text, steer: true } => self.session.steer_read(text),
            _ => {}
        }
        (self.sink)(event);
    }

    /// Reports a panic as the turn's error; a sink that panics again is
    /// skipped.
    fn panicked(&mut self, payload: &(dyn Any + Send)) {
        let text = format!(
            "the turn stopped on an internal error: {}",
            panic_text(payload)
        );
        self.error = Some(text.clone());
        let sink = &mut self.sink;
        let _ = catch_unwind(AssertUnwindSafe(|| sink(AgentEvent::Error(text))));
    }
}

/// The message of a panic payload.
fn panic_text(payload: &(dyn Any + Send)) -> String {
    if let Some(text) = payload.downcast_ref::<&str>() {
        return (*text).to_string();
    }
    match payload.downcast_ref::<String>() {
        Some(text) => text.clone(),
        None => "a panic without a message".into(),
    }
}
