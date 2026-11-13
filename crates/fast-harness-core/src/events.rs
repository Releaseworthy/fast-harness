//! What the agent tells a front end (CLI print mode, TUI, GUI) while it works.
//! Events cross threads, so they own their data. Each event that matters for
//! tracing is written to the session log before it is sent.

use crate::llm::Timing;
pub use crate::tools::ask::{Question, QuestionAnswer, QuestionOption};
use crate::tools::Effect;
use crate::types::{StopReason, Usage};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub enum TurnOutcome {
    /// The model finished its answer.
    Done,
    Cancelled,
    /// Stopped by an error that retries and failover could not fix.
    Failed,
    /// The step limit of one turn was reached.
    StepLimit,
    /// The output token limit cut the answer (`max_tokens`).
    Truncated,
    /// The prompt no longer fits the model's context window.
    ContextOverflow,
    /// A pause arrived: the step's tools finished and the model was not
    /// called again; `Agent::resume_turn` goes on (PLAN.md D39).
    Paused,
}

#[derive(Clone, Debug, PartialEq)]
#[non_exhaustive]
pub enum AgentEvent {
    /// A steer or follow-up text the turn took from its inbox (the first
    /// text of `run_turn` is the caller's and has no event).
    UserInput {
        text: String,
        steer: bool,
    },
    TurnStart {
        turn: u32,
    },
    StepStart {
        turn: u32,
        step: u32,
    },
    /// One attempt of a model call is about to be sent.
    Request {
        model: String,
        account: String,
        attempt: u32,
    },
    TextDelta(String),
    ThinkingDelta(String),
    ToolCallStart {
        id: String,
        name: String,
    },
    ToolArgsDelta {
        id: String,
        chunk: String,
    },
    /// A tool call is complete and is about to run.
    ToolCall {
        id: String,
        name: String,
        arguments: String,
    },
    /// New text from running tool call `id` since its last progress event
    /// (`ToolCtx::progress`, PLAN.md D58), after its `ToolCall` and before
    /// its `ToolResult`. Live only: never traced, never sent to the model.
    ToolProgress {
        id: String,
        chunk: String,
    },
    ToolResult {
        id: String,
        content: String,
        is_error: bool,
        duration_ms: u64,
    },
    /// A failed attempt will be retried after `delay_ms`.
    Retry {
        attempt: u32,
        delay_ms: u64,
        error: String,
    },
    /// The credential changed after a limit or auth failure.
    AccountSwitch {
        from: String,
        to: String,
        reason: String,
    },
    /// New usage numbers are known for this account (read them from the pool).
    UsageUpdate {
        account: String,
    },
    /// A subagent (of this agent or a deeper descendant) started or
    /// finished; its own session log has `session` as id (PLAN.md D40).
    Subagent {
        session: String,
        running: bool,
    },
    /// What a running subagent does now, at most every 500 ms per child:
    /// `line` is short text such as `look · step 3 · read`.
    SubagentProgress {
        session: String,
        line: String,
    },
    StepEnd {
        turn: u32,
        step: u32,
        usage: Usage,
        /// The context after this step: prompt plus output tokens.
        context: u64,
        /// The model's context window in tokens; 0 when unknown.
        window: u64,
        timing: Timing,
        stop: StopReason,
    },
    /// The history was replaced by a summary (PLAN.md D38): `before` is the
    /// context of the last step, `after` an estimate of the new one.
    Compaction {
        summary: String,
        before: u64,
        after: u64,
        auto: bool,
    },
    TurnEnd {
        turn: u32,
        usage: Usage,
        duration_ms: u64,
        outcome: TurnOutcome,
    },
    Error(String),
    /// A `Write` or `Run` call waits for the user's answer (PLAN.md D64),
    /// after its PreToolUse hooks; answer with `service::Session::answer`.
    /// Live only: never traced.
    ApprovalRequest(ApprovalRequest),
    /// Request `id` ended: `once`, `session`, `declined`, `cancelled`,
    /// `paused` or `refused` (nobody can answer). Live only.
    ApprovalDone {
        id: String,
        answer: String,
    },
    /// An `ask_user_question` call waits for the user's answers (PLAN.md
    /// D81); answer with `service::Session::reply`. Live only.
    QuestionRequest(QuestionRequest),
    /// Question request `id` ended: `answered`, `skipped`, `cancelled` or
    /// `paused`. Live only.
    QuestionDone {
        id: String,
        answer: String,
    },
}

/// Questions of one `ask_user_question` call (PLAN.md D81).
#[derive(Clone, Debug, Default, PartialEq, Eq)]
#[non_exhaustive]
pub struct QuestionRequest {
    /// Unique in the session: what a reply names.
    pub id: String,
    /// The model's tool call id.
    pub call_id: String,
    pub questions: Vec<Question>,
    /// The subagent that asks; `None` for the session itself.
    pub agent: Option<String>,
}

/// One question to the user before a call runs (PLAN.md D64).
#[derive(Clone, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub struct ApprovalRequest {
    /// Unique in the session: what an answer names.
    pub id: String,
    /// The model's tool call id.
    pub call_id: String,
    pub tool: String,
    pub effect: Effect,
    /// One line: the command, or the kind of change and the path.
    pub summary: String,
    /// The final arguments (after the hooks).
    pub arguments: String,
    /// The subagent that asks (`label (session id)`); `None` for the
    /// session itself.
    pub agent: Option<String>,
}

/// Empty texts and `Effect::Run` (front end tests set the fields).
impl Default for ApprovalRequest {
    fn default() -> ApprovalRequest {
        ApprovalRequest {
            id: String::new(),
            call_id: String::new(),
            tool: String::new(),
            effect: Effect::Run,
            summary: String::new(),
            arguments: String::new(),
            agent: None,
        }
    }
}
