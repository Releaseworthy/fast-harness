//! A read keeps its own room in a crowded step (PLAN.md D90): the owner's
//! retest of 2026-11-15 got 58 of 64 asked lines in a step of six calls
//! and 87 short or 24 wide lines from a plain read in a step of four.

mod agent_support;

use std::collections::HashMap;
use std::sync::atomic::AtomicBool;
use std::sync::Arc;

use agent_support::*;
use fast_harness_core::agent::{Agent, Conversation};
use fast_harness_core::http::ScriptedTransport;
use fast_harness_core::tools::{self};
use fast_harness_core::types::Block;

/// The owner's `big.txt` (short rows) and `wide.txt` (120-character rows).
fn files(env: &Env) {
    let big: String = (1..=20000)
        .map(|i| format!("row {i:05} padding-text-xxxxx\n"))
        .collect();
    std::fs::write(env.cwd.join("big.txt"), big).unwrap();
    let wide: String = (1..=2000)
        .map(|i| format!("{i:05} {}\n", "w".repeat(114)))
        .collect();
    std::fs::write(env.cwd.join("wide.txt"), wide).unwrap();
}

/// The results of one step making `calls`, by call id.
fn one_step(name: &str, calls: &[(&str, &str, &str)]) -> HashMap<String, String> {
    let env = env(name);
    files(&env);
    std::fs::write(env.cwd.join("e.txt"), "alpha\n").unwrap();
    let script = vec![tool_reply(calls), text_reply("done")];
    let transport = Arc::new(ScriptedTransport::new(script));
    let parts = parts(&transport, key_auth(), &env.cwd);
    let mut agent = Agent::new(parts, Arc::new(tools::builtin()));
    agent.preload = vec!["read".into(), "edit".into()];
    let mut conv = Conversation::create(&env.home, &env.cwd, None).unwrap();
    agent.run_turn(&mut conv, "go", &AtomicBool::new(false), &mut |_| {});
    let blocks = conv.messages.iter().flat_map(|m| m.content.iter());
    blocks
        .filter_map(|block| match block {
            Block::ToolResult {
                call_id, content, ..
            } => Some((call_id.clone(), content.clone())),
            _ => None,
        })
        .collect()
}

/// Numbered lines of a read result (`N: text`).
fn numbered(text: &str) -> usize {
    let digits = |l: &&str| {
        l.split(':')
            .next()
            .is_some_and(|n| n.parse::<u64>().is_ok())
    };
    text.lines().filter(digits).count()
}

const EDIT: &str = r#"{"path":"e.txt","old_string":"nope","new_string":"x"}"#;

#[test]
fn a_read_with_a_limit_in_a_step_of_six_calls_returns_every_line() {
    let ranged = r#"{"path":"big.txt","offset":2000,"limit":64}"#;
    let wide = r#"{"path":"wide.txt","offset":1000,"limit":40}"#;
    let calls = [
        ("e1", "edit", EDIT),
        ("e2", "edit", EDIT),
        ("e3", "edit", EDIT),
        ("r1", "read", ranged),
        ("r2", "read", wide),
        ("r3", "read", r#"{"path":"e.txt"}"#),
    ];
    let results = one_step("d90-limit", &calls);
    assert_eq!(numbered(&results["r1"]), 64, "{}", results["r1"]);
    assert_eq!(numbered(&results["r2"]), 40, "{}", results["r2"]);
}

#[test]
fn a_plain_read_in_a_crowded_step_returns_six_to_eight_kb() {
    let calls = [
        ("e1", "edit", EDIT),
        ("r1", "read", r#"{"path":"big.txt","offset":1000}"#),
        ("r2", "read", r#"{"path":"wide.txt","offset":1000}"#),
        ("r3", "read", r#"{"path":"big.txt"}"#),
    ];
    let results = one_step("d90-head", &calls);
    for result in ["r1", "r2", "r3"].map(|id| &results[id]) {
        let size = result.len();
        assert!((6 * 1024..=8 * 1024).contains(&size), "{size}: {result}");
    }
    assert!(numbered(&results["r2"]) >= 48, "{}", results["r2"]);
}
