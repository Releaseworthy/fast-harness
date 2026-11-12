//! The approval outcome of each tool call (PLAN.md D64), live only: the
//! request events are never traced, so a replayed transcript has none (the
//! declined, cancelled, paused and refused calls say so in their result).

use crate::config::approvals::LEFT_OUT;
use crate::events::ApprovalRequest;

/// The words of the calls that asked, by tool call id.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Approvals {
    /// `(request id, call id)` of every request seen.
    asked: Vec<(String, String)>,
    /// `(call id, word)`, the newest last.
    words: Vec<(String, &'static str)>,
}

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
    /// A request of call `call_id`: it waits.
    pub(crate) fn asked(&mut self, request: &ApprovalRequest) {
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
