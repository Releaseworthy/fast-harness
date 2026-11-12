//! Turns that can be resumed (PLAN.md D39): paused, cancelled, or cut by a
//! crash or a kill (a `turn/start` without its `turn/end`).

use crate::trace::record::TurnEnd;
use crate::trace::{kind, Record, TraceEvent};
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
    let turn = match last_start.record() {
        Record::TurnStart(start) => Some(start.turn),
        _ => None,
    };
    let ends = events
        .iter()
        .filter(|e| e.kind == kind::TURN_END && e.seq > last_start.seq);
    let end = ends.map(|e| match e.record() {
        Record::TurnEnd(end) => end,
        // An end this build cannot read: the turn ended.
        _ => TurnEnd::default(),
    });
    let mut end =
        end.filter(|end| turn.is_none_or(|turn| end.turn == turn || end.outcome.is_empty()));
    let state = match end.next().map(|end| end.outcome) {
        None => Unfinished::Interrupted,
        Some(word) if word == "paused" => Unfinished::Paused,
        Some(word) if word == "cancelled" => Unfinished::Cancelled,
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
    match last.record() {
        Record::AssistantMessage(answer) => answer.message.tool_calls().next().is_none(),
        _ => false,
    }
}

/// Adds a replayed message: a `user/message` with a `kind` (a steer or a
/// context update inside a turn) joins the tool results or user text before
/// it, as the live turn did; anything else is a new message.
pub(super) fn push(messages: &mut Vec<Message>, message: Message, event: &TraceEvent) {
    let inside = event.kind == kind::USER_MESSAGE
        && matches!(event.record(), Record::UserMessage(user) if user.kind.is_some());
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
