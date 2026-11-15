//! Prompt cache breakpoints and their lifetimes (PLAN.md D36, D88).
//!
//! Tools and system come first and share one lifetime, so a 1 hour
//! breakpoint never follows a 5 minute one. The messages carry one on the
//! last block of a trailing user message. With [`Cache::loop_5m`]
//! (`cacheTtl: auto` in the TUI and GUI) that one lives 5 minutes inside the
//! tool loop (the next step comes within seconds) and 1 hour on a turn's
//! first step, and the user message that opened the latest turn before the
//! trailing message keeps a 1 hour breakpoint: after a pause the next step
//! reads up to there even when the 5 minute entries are gone and it lies
//! more than 20 blocks back (the API looks back only that far).

use serde_json::{json, Map, Value};

use super::request::CLAUDE_CODE_IDENTITY;
use crate::llm::Cache;

/// The most `cache_control` blocks the API takes in one request.
const MAX_BREAKPOINTS: usize = 4;

pub(crate) fn cache_control(long: bool) -> Value {
    if long {
        json!({"type": "ephemeral", "ttl": "1h"})
    } else {
        json!({"type": "ephemeral"})
    }
}

/// The breakpoint of the tools and system blocks.
pub(crate) fn prefix(cache: &Cache) -> Value {
    cache_control(cache.system_1h || cache.messages_1h)
}

/// Puts the message breakpoints on `messages` (the request's array, effort
/// `system` messages included: they carry no content to cache).
pub(crate) fn mark_messages(messages: &mut [Value], cache: &Cache) {
    let Some(last) = messages.iter().rposition(|m| m["role"] != "system") else {
        return;
    };
    if !(cache.loop_5m && cache.messages_1h) {
        mark_user(&mut messages[last], &cache_control(cache.messages_1h));
        return;
    }
    if let Some(turn) = messages[..last].iter().rposition(opens_turn) {
        mark_user(&mut messages[turn], &cache_control(true));
    }
    let turn_start = opens_turn(&messages[last]);
    mark_user(&mut messages[last], &cache_control(turn_start));
}

/// A user message with new input, not tool results (those come first in
/// theirs).
fn opens_turn(message: &Value) -> bool {
    message["role"] == "user" && message["content"][0]["type"] != "tool_result"
}

/// Puts `cache` on the last block of `message` when it is a user message.
fn mark_user(message: &mut Value, cache: &Value) {
    if message["role"] != "user" {
        return;
    }
    if let Some(block) = message["content"].as_array_mut().and_then(|c| c.last_mut()) {
        if matches!(
            block["type"].as_str(),
            Some("text" | "tool_result" | "image")
        ) {
            block["cache_control"] = cache.clone();
        }
    }
}

/// Keeps at most [`MAX_BREAKPOINTS`]: the turn's breakpoint can make 5.
/// The OAuth identity block's goes first (the stable block after it covers
/// it), then the last tool's (the system blocks after it cover it).
pub(crate) fn at_most_four(body: &mut Map<String, Value>) {
    let has = |block: &Value| block.get("cache_control").is_some();
    let in_list = |list: Option<&Value>| {
        let list = list.and_then(Value::as_array).map(Vec::as_slice);
        list.unwrap_or_default().iter().filter(|b| has(b)).count()
    };
    let messages = body.get("messages").and_then(Value::as_array);
    let content = messages.into_iter().flatten().map(|m| m.get("content"));
    let mut count = in_list(body.get("tools"))
        + in_list(body.get("system"))
        + content.map(in_list).sum::<usize>();
    while count > MAX_BREAKPOINTS {
        let system = body.get_mut("system").and_then(Value::as_array_mut);
        if !(system.is_some_and(|list| unmark_identity(list)) || unmark_tools(body)) {
            return;
        }
        count -= 1;
    }
}

/// Drops the breakpoint of a leading identity block followed by others.
fn unmark_identity(system: &mut [Value]) -> bool {
    if system.len() < 2 || system[0]["text"] != CLAUDE_CODE_IDENTITY {
        return false;
    }
    unmark(&mut system[0])
}

/// Drops the breakpoint of the last tool that has one.
fn unmark_tools(body: &mut Map<String, Value>) -> bool {
    let tools = body.get_mut("tools").and_then(Value::as_array_mut);
    tools.is_some_and(|tools| tools.iter_mut().rev().any(unmark))
}

fn unmark(block: &mut Value) -> bool {
    let map = block.as_object_mut();
    map.and_then(|o| o.remove("cache_control")).is_some()
}
