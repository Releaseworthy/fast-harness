//! The approval outcome of each tool call (PLAN.md D64): live from
//! `ApprovalRequest` and `ApprovalDone`, replayed from the `tool/approval`
//! record (trace version 11). A request is live only: the record holds the
//! decision, so a replayed row shows the same word.

use crate::config::approvals::LEFT_OUT;
use crate::events::ApprovalRequest;

/// The words of the calls that asked, by tool call id.
#[derive(Clone, Debug, Default)]
pub struct Approvals {
    /// `(request id, call id)` of every request seen.
    asked: Vec<(String, String)>,
    /// `(call id, word)`, the newest last.
    words: Vec<(String, &'static str)>,
}

/// Only the words compare: `asked` is live bookkeeping (a request id to its
/// call), and a replay has no request ids (D64).
impl PartialEq for Approvals {
    fn eq(&self, other: &Self) -> bool {
        self.words == other.words
    }
}

impl Eq for Approvals {}

/// The word of a request that waits for its answer.
pub const WAITING: &str = "waiting for your answer";

/// The tool row's word for an `ApprovalDone` answer.
pub fn outcome_word(answer: &str) -> &'static str {
    match answer {
        "once" => "allowed once",
        "session" => "allowed for this session",
        "declined" => "declined",
        "cancelled" => "cancelled",
        "paused" => "paused",
        "refused" => "refused: nobody can answer",
        _ => "ended",
    }
}

/// What a prompt shows of `request`, never cut: `summary` is one line that
/// leaves out the middle of a long text, so a longer command comes whole
/// from the arguments, line breaks kept.
pub fn prompt_text(request: &ApprovalRequest) -> String {
    let args: serde_json::Value =
        serde_json::from_str(&request.arguments).unwrap_or(serde_json::Value::Null);
    let command = args.get("command").and_then(serde_json::Value::as_str);
    match command {
        Some(command) if request.tool == "shell" => format!("run: {command}"),
        _ if request.summary.contains(LEFT_OUT) => {
            format!("{} {}", request.tool, request.arguments)
        }
        _ => request.summary.clone(),
    }
}

impl Approvals {
    /// A request of call `call_id`: it waits. A subagent's request names a
    /// call of the child, which has no row here and whose `tool/approval`
    /// line is in the child's log (D61): it is left out, so a replay matches.
    pub(crate) fn asked(&mut self, request: &ApprovalRequest) {
        if request.agent.is_some() {
            return;
        }
        let ids = (request.id.clone(), request.call_id.clone());
        self.set(&request.call_id, WAITING);
        self.asked.push(ids);
    }

    /// Request `id` ended with `answer`.
    pub(crate) fn done(&mut self, id: &str, answer: &str) {
        let call = self
            .asked
            .iter()
            .find(|(r, _)| r == id)
            .map(|(_, c)| c.clone());
        if let Some(call) = call {
            self.set(&call, outcome_word(answer));
        }
    }

    /// The decision of a replayed request (the `tool/approval` line, trace
    /// version 11): the same word the live `ApprovalDone` set.
    pub(crate) fn record(&mut self, call_id: &str, answer: &str) {
        self.set(call_id, outcome_word(answer));
    }

    /// True when tool call `call_id` never ran because the gate ended it: the
    /// row says so in its word and is not a failed call (D64).
    pub fn kept_from_running(&self, call_id: &str) -> bool {
        (self.word(call_id)).is_some_and(gate_ended)
    }

    fn set(&mut self, call: &str, word: &'static str) {
        self.words.retain(|(c, _)| c != call);
        self.words.push((call.to_string(), word));
    }

    /// The word of tool call `call_id`: `None` when it never asked.
    pub fn word(&self, call_id: &str) -> Option<&'static str> {
        (self.words.iter().rev())
            .find(|(c, _)| c == call_id)
            .map(|(_, w)| *w)
    }

    pub fn is_empty(&self) -> bool {
        self.words.is_empty()
    }
}

/// True for a word that says the gate ended the call before it ran:
/// `declined`, `cancelled`, `paused` or `refused ...` (D64).
pub fn gate_ended(word: &str) -> bool {
    matches!(word, "declined" | "cancelled" | "paused") || word.starts_with("refused")
}
