//! Turns that can be resumed (PLAN.md D39): paused, cancelled, or cut by a
//! crash or a kill (a `turn/start` without its `turn/end`).

use crate::trace::{kind, TraceEvent};
use crate::types::{Block, Message, Role};

/// How the last turn of a log stopped, when it can be resumed.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Unfinished {
    /// No `turn/end`: the process crashed or was killed.
    Interrupted,
    /// `turn/end` outcome `paused`.
    Paused,
    /// `turn/end` outcome `cancelled`.
    Cancelled,
}

impl Unfinished {
    pub fn word(self) -> &'static str {
        match self {
            Unfinished::Interrupted => "interrupted",
            Unfinished::Paused => "paused",
            Unfinished::Cancelled => "cancelled",
        }
    }

    /// The state named by [`Unfinished::word`].
    pub fn from_word(word: &str) -> Option<Unfinished> {
        [
            Unfinished::Interrupted,
            Unfinished::Paused,
            Unfinished::Cancelled,
        ]
        .into_iter()
        .find(|state| state.word() == word)
    }
}

/// The state of the last turn of `events`, when its history does not end
/// with the model's final answer (that turn has nothing left to do).
pub fn unfinished(events: &[TraceEvent]) -> Option<Unfinished> {
    let last_start = events
        .iter()
        .filter(|e| e.kind == kind::TURN_START)
        .max_by_key(|e| e.seq)?;
    let turn = &last_start.data["turn"];
    let end = events
        .iter()
        .filter(|e| e.kind == kind::TURN_END && e.seq > last_start.seq)
        .find(|e| &e.data["turn"] == turn);
    let state = match end.map(|e| e.data["outcome"].as_str().unwrap_or_default()) {
        None => Unfinished::Interrupted,
        Some("paused") => Unfinished::Paused,
        Some("cancelled") => Unfinished::Cancelled,
        Some(_) => return None,
    };
    (!ends_with_answer(events)).then_some(state)
}

/// True when the last message event is an answer without tool calls.
fn ends_with_answer(events: &[TraceEvent]) -> bool {
    let last = events
        .iter()
        .filter(|e| {
            [
                kind::USER_MESSAGE,
                kind::ASSISTANT_MESSAGE,
                kind::TOOL_RESULT,
                kind::COMPACTION,
            ]
            .contains(&e.kind.as_str())
        })
        .max_by_key(|e| e.seq);
    let Some(last) = last.filter(|e| e.kind == kind::ASSISTANT_MESSAGE) else {
        return false;
    };
    let message = serde_json::from_value::<Message>(last.data["message"].clone());
    message.is_ok_and(|m| m.tool_calls().next().is_none())
}

/// Adds a replayed message: a `user/message` with a `kind` (a steer or a
/// context update inside a turn) joins the tool results or user text before
/// it, as the live turn did; anything else is a new message.
pub(super) fn push(messages: &mut Vec<Message>, message: Message, event: &TraceEvent) {
    let inside = event.kind == kind::USER_MESSAGE && event.data.get("kind").is_some();
    match messages.last_mut() {
        Some(last) if inside && last.role != Role::Assistant => {
            let texts = message
                .content
                .into_iter()
                .filter(|b| matches!(b, Block::Text { .. }));
            last.content.extend(texts);
        }
        _ => messages.push(message),
    }
}
