//! What the agent tells a front end (CLI print mode, TUI, GUI) while it works.
//! Events cross threads, so they own their data. Each event that matters for
//! tracing is written to the session log before it is sent.

use crate::llm::Timing;
use crate::types::{StopReason, Usage};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
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
}

#[derive(Clone, Debug, PartialEq)]
pub enum AgentEvent {
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
    /// A subagent started or finished; its own session log has `session` as id.
    Subagent {
        session: String,
        running: bool,
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
}
