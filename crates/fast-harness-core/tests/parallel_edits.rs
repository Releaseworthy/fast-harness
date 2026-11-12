//! Calls of one step that change a file (PLAN.md D54, finding A1). Before
//! D54 every call of a step ran at once and `edit` reads and writes the
//! whole file without a lock: 8 edits of one file all reported success and
//! 1 stayed (20 of 20 rounds, 2026-11-12). Now `edit` and `write` declare
//! `Effect::Write` and run one at a time in call order, and `shell` (`Run`)
//! starts only after them.

mod agent_support;

use std::sync::atomic::AtomicBool;
use std::sync::Arc;

use agent_support::*;
use fast_harness_core::agent::{Agent, Conversation};
use fast_harness_core::http::ScriptedTransport;
use fast_harness_core::tools;
use fast_harness_core::types::{Block, Role};

const EDITS: usize = 8;
const ROUNDS: usize = 20;

/// One turn: a find loads `edit`, then one step holds `EDITS` edit calls on
/// `a.txt`, each replacing its own marker. Returns the file afterwards.
fn edit_all_markers_in_one_step(round: usize) -> String {
    let env = env(&format!("paredit{round}"));
    let text: String = (0..EDITS).map(|n| format!("marker_{n}_old\n")).collect();
    std::fs::write(env.cwd.join("a.txt"), text).unwrap();
    let args: Vec<String> = (0..EDITS)
        .map(|n| {
            format!(
                r#"{{"path":"a.txt","old_string":"marker_{n}_old","new_string":"marker_{n}_new"}}"#
            )
        })
        .collect();
    let ids: Vec<String> = (0..EDITS).map(|n| format!("toolu_e{n}")).collect();
    let calls: Vec<(&str, &str, &str)> = (0..EDITS)
        .map(|n| (ids[n].as_str(), "edit", args[n].as_str()))
        .collect();
    let transport = Arc::new(ScriptedTransport::new(vec![
        tool_reply(&[("toolu_f", "find_tools", r#"{"names":["edit"]}"#)]),
        tool_reply(&calls),
        text_reply("done"),
    ]));
    let parts = parts(&transport, key_auth(), &env.cwd);
    let agent = Agent::new(parts, Arc::new(tools::builtin()));
    let mut conv = Conversation::create(&env.home, &env.cwd, None).unwrap();
    agent.run_turn(&mut conv, "go", &AtomicBool::new(false), &mut |_| {});
    std::fs::read_to_string(env.cwd.join("a.txt")).unwrap()
}

#[test]
fn every_edit_of_one_step_lands_in_the_file() {
    for round in 0..ROUNDS {
        let after = edit_all_markers_in_one_step(round);
        let landed = (0..EDITS)
            .filter(|n| after.contains(&format!("marker_{n}_new")))
            .count();
        assert_eq!(landed, EDITS, "round {round}: the file holds\n{after}");
    }
}

/// One step: `write` a file, then `shell` prints it. The shell starts after
/// the write ended, so it prints the new content.
#[test]
fn shell_after_write_in_one_step_sees_the_file() {
    let env = env("writeshell");
    let write = r#"{"path":"b.txt","content":"fresh content 42"}"#;
    let transport = Arc::new(ScriptedTransport::new(vec![
        tool_reply(&[("toolu_f", "find_tools", r#"{"names":["write","shell"]}"#)]),
        tool_reply(&[
            ("toolu_w", "write", write),
            ("toolu_s", "shell", r#"{"command":"cat b.txt"}"#),
        ]),
        text_reply("done"),
    ]));
    let parts = parts(&transport, key_auth(), &env.cwd);
    let agent = Agent::new(parts, Arc::new(tools::builtin()));
    let mut conv = Conversation::create(&env.home, &env.cwd, None).unwrap();
    agent.run_turn(&mut conv, "go", &AtomicBool::new(false), &mut |_| {});
    let results = tool_results(&conv);
    assert!(results[0].contains("Wrote 16 bytes"), "{results:?}");
    assert!(results[1].contains("fresh content 42"), "{results:?}");
}

/// The texts of the last tool message, in call order.
fn tool_results(conv: &Conversation) -> Vec<String> {
    let message = conv
        .messages
        .iter()
        .rev()
        .find(|m| m.role == Role::Tool)
        .unwrap();
    (message.content.iter())
        .map(|block| match block {
            Block::ToolResult { content, .. } => content.clone(),
            _ => String::new(),
        })
        .collect()
}
