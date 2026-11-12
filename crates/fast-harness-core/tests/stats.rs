//! `stats` on a synthetic session log.

use fast_harness_core::stats::{self, Stats};
use fast_harness_core::trace::TraceEvent;
use serde_json::{json, Value};

const MINUTE: u64 = 60_000;

fn ev(kind: &str, time: u64, data: Value) -> TraceEvent {
    TraceEvent {
        kind: kind.into(),
        seq: 0,
        time,
        data,
    }
}

fn answer(turn: u32, step: u32, time: u64, usage: Value) -> TraceEvent {
    let data = json!({"turn": turn, "step": step, "usage": usage, "window": 1_000_000});
    ev("assistant/message", time, data)
}

fn results(turn: u32, step: u32, time: u64, blocks: &[(&str, String)]) -> TraceEvent {
    let content: Vec<Value> = blocks
        .iter()
        .map(|(id, text)| json!({"type": "tool_result", "callId": id, "content": text}))
        .collect();
    let message = json!({"role": "tool", "content": content});
    ev(
        "tool/result",
        time,
        json!({"turn": turn, "step": step, "message": message}),
    )
}

fn call(turn: u32, step: u32, id: &str, name: &str) -> TraceEvent {
    let data = json!({"turn": turn, "step": step, "callId": id, "name": name});
    ev("tool/call", 0, data)
}

/// Turn 1: a find step and a read step; turn 2 after 10 idle minutes
/// rewrites the cache; one transformation and one compaction.
fn log() -> Vec<TraceEvent> {
    let clipped = format!("{}\n[... 9 bytes clipped ...]\n", "x".repeat(100));
    let changes = json!([{"type": "prefix_binding_mismatch"}]);
    vec![
        ev("turn/start", 0, json!({"turn": 1})),
        ev("step/start", 0, json!({"turn": 1, "step": 1})),
        answer(
            1,
            1,
            MINUTE,
            json!({"inputTokens": 10, "outputTokens": 5, "cacheWriteTokens": 1000}),
        ),
        call(1, 1, "c1", "find_tools"),
        results(1, 1, MINUTE, &[("c1", "abc".into())]),
        ev("step/end", MINUTE, json!({"turn": 1, "step": 1})),
        ev("step/start", 2 * MINUTE, json!({"turn": 1, "step": 2})),
        answer(
            1,
            2,
            2 * MINUTE,
            json!({"inputTokens": 5, "outputTokens": 5, "cacheReadTokens": 1000}),
        ),
        call(1, 2, "c2", "read"),
        call(1, 2, "c3", "read"),
        results(1, 2, 2 * MINUTE, &[("c2", clipped), ("c3", "y".repeat(40))]),
        ev("step/end", 2 * MINUTE, json!({"turn": 1, "step": 2})),
        ev("turn/start", 12 * MINUTE, json!({"turn": 2})),
        ev("step/start", 12 * MINUTE, json!({"turn": 2, "step": 1})),
        ev(
            "request/transformations",
            13 * MINUTE,
            json!({"transformations": changes}),
        ),
        answer(
            2,
            1,
            13 * MINUTE,
            json!({"inputTokens": 5, "outputTokens": 9, "cacheWriteTokens": 1020}),
        ),
        ev("step/end", 13 * MINUTE, json!({"turn": 2, "step": 1})),
        ev("compaction", 14 * MINUTE, json!({})),
    ]
}

#[test]
fn counts_a_synthetic_session() {
    let s = stats::compute("s1", &log());
    assert_eq!((s.sessions, s.turns, s.steps), (1, 2, 3));
    assert_eq!(s.prompt_tokens, 10 + 1000 + 5 + 1000 + 5 + 1020);
    assert_eq!(s.cache_hit_percent, Some(1000 * 100 / 3040));
    assert_eq!((s.idle_rewrites, s.idle_rewrite_tokens), (1, 1020));
    assert_eq!((s.peak_context, s.window), (1034, 1_000_000));
    assert_eq!(s.tools.len(), 2);
    assert_eq!((s.tools[0].name.as_str(), s.tools[0].results), ("read", 2));
    assert_eq!(s.tools[0].bytes, 127 + 40);
    assert_eq!(
        (s.tools[1].name.as_str(), s.tools[1].bytes),
        ("find_tools", 3)
    );
    let sizes: Vec<u64> = s.largest.iter().map(|r| r.bytes).collect();
    assert_eq!(sizes, [127, 40, 3]);
    assert_eq!(
        (s.largest[0].session.as_str(), s.largest[0].step),
        ("s1", 2)
    );
    assert_eq!((s.clipped, s.find_steps, s.compactions), (1, 1, 1));
    assert_eq!((s.transformations, s.prefix_binding_mismatches), (1, 1));
    let text = stats::format_text(&s);
    assert!(text.contains("cache hit rate 32%"), "{text}");
    assert!(
        text.contains("cache rewrites after an idle gap 1 (1,020 tokens)"),
        "{text}"
    );
    assert!(text.contains("peak context 1,034 (0% of 1M)"), "{text}");
}

#[test]
fn one_hour_writes_survive_a_short_gap_and_sessions_merge() {
    let mut events = log();
    // The turn 2 write was a 1 hour write: a 10 minute gap is no rewrite.
    events[15].data["usage"]["cacheWrite1hTokens"] = json!(1020);
    let one = stats::compute("s1", &events);
    assert_eq!(one.idle_rewrites, 0);
    let mut all = Stats::default();
    all.merge(one.clone());
    all.merge(stats::compute("s2", &log()));
    assert_eq!((all.sessions, all.steps, all.idle_rewrites), (2, 6, 1));
    assert_eq!(all.tools[0].results, 4);
    assert_eq!(all.largest.len(), stats::LARGEST);
    assert_eq!(all.cache_hit_percent, one.cache_hit_percent);
    let json = serde_json::to_value(&all).unwrap();
    assert_eq!(json["findSteps"], json!(2));
}
