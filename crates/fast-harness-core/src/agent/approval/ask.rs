//! `ask_user_question` through the session's gate (PLAN.md D81): the call
//! waits, like an approval request, for a front end's reply, a cancel, a
//! pause or the close of the session. Without a front end that answers
//! questions the tool itself runs and refuses at once.

use std::sync::atomic::{AtomicBool, Ordering};

use serde_json::Value;

use super::{Agent, Gate, Stop, BACKSTOP};
use crate::agent::tool_run::CANCELLED;
use crate::events::{AgentEvent, QuestionAnswer, QuestionRequest};
use crate::tools::ask::{self, Question, SKIPPED};
use crate::tools::ToolOutput;

/// A front end's reply: the answers, or `None` when the user skipped.
pub type Reply = Option<Vec<QuestionAnswer>>;

/// The text of a question call that waited when a pause arrived.
const PAUSED: &str = "not answered: the turn was paused while the questions waited; ask again \
when the turn goes on";

impl Gate {
    /// A front end that shows questions and answers them says so.
    pub fn set_asking(&self, on: bool) {
        self.lock().asking = on;
    }

    /// The question requests waiting for a reply, oldest first.
    pub fn questions(&self) -> Vec<QuestionRequest> {
        self.lock().questions.clone()
    }

    /// Replies to question request `id`; false when none `id` waits.
    pub fn reply(&self, id: &str, reply: Reply) -> bool {
        let mut state = self.lock();
        let waits = state.questions.iter().any(|r| r.id == id);
        if !waits || state.replies.iter().any(|(r, _)| r == id) {
            return false;
        }
        state.replies.push((id.to_string(), reply));
        self.changed.notify_all();
        true
    }

    fn ask_user(
        &self,
        mut request: QuestionRequest,
        cancel: &AtomicBool,
        emit: &dyn Fn(AgentEvent),
    ) -> Result<Reply, Stop> {
        {
            let mut state = self.lock();
            if let Some(stop) = state.stop {
                return Err(stop);
            }
            state.next += 1;
            request.id = format!("q{}", state.next);
            state.questions.push(request.clone());
        }
        let id = request.id.clone();
        emit(AgentEvent::QuestionRequest(request));
        let ended = self.wait_reply(&id, cancel);
        let answer = match &ended {
            Ok(Some(_)) => "answered",
            Ok(None) => "skipped",
            Err(Stop::Cancel) => "cancelled",
            Err(Stop::Pause) => "paused",
        };
        let answer = answer.to_string();
        emit(AgentEvent::QuestionDone { id, answer });
        ended
    }

    fn wait_reply(&self, id: &str, cancel: &AtomicBool) -> Result<Reply, Stop> {
        let mut state = self.lock();
        loop {
            if let Some(at) = state.replies.iter().position(|(r, _)| r == id) {
                state.questions.retain(|r| r.id != id);
                return Ok(state.replies.remove(at).1);
            }
            let own = cancel.load(Ordering::Relaxed).then_some(Stop::Cancel);
            if let Some(stop) = state.stop.or(own) {
                state.questions.retain(|r| r.id != id);
                return Err(stop);
            }
            state = match self.changed.wait_timeout(state, BACKSTOP) {
                Ok((guard, _)) => guard,
                Err(poisoned) => poisoned.into_inner().0,
            };
        }
    }
}

/// `None` lets the tool run (it refuses: nobody answers, or the arguments
/// are wrong); `Some` is the result of the asked questions.
pub(super) fn check(
    agent: &Agent,
    (call_id, raw): (&str, &str),
    cancel: &AtomicBool,
    emit: &dyn Fn(AgentEvent),
) -> Option<ToolOutput> {
    let approver = agent.approver.as_ref()?;
    if !approver.gate.lock().asking {
        return None;
    }
    let args = serde_json::from_str::<Value>(raw).ok()?;
    let questions: Vec<Question> = ask::parse(&args).ok()?;
    let request = QuestionRequest {
        id: String::new(),
        call_id: call_id.to_string(),
        questions: questions.clone(),
        agent: approver.agent.clone(),
    };
    Some(match approver.gate.ask_user(request, cancel, emit) {
        Ok(Some(answers)) => ToolOutput::ok(ask::format_answers(&questions, &answers)),
        Ok(None) => ToolOutput::ok(SKIPPED),
        Err(Stop::Cancel) => ToolOutput::err(CANCELLED),
        Err(Stop::Pause) => ToolOutput::err(PAUSED),
    })
}
