//! Several `edit` calls of one step on one file (docs/architecture-plan.md,
//! A1). The calls of a step run at once on scoped threads and `edit` reads
//! and writes the whole file without a lock, so today every call reports
//! success and only one edit stays in the file. Ignored until A1 lands; run
//! by hand: `cargo test -p fast-harness-core --test parallel_edits -- --ignored`.

mod agent_support;

use std::sync::atomic::AtomicBool;
use std::sync::Arc;

use agent_support::*;
use fast_harness_core::agent::{Agent, Conversation};
use fast_harness_core::http::ScriptedTransport;
use fast_harness_core::tools;

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
#[ignore = "fails until docs/architecture-plan.md A1: 2026-11-12, 20 of 20 rounds kept 1 of 8 edits"]
fn every_edit_of_one_step_lands_in_the_file() {
    for round in 0..ROUNDS {
        let after = edit_all_markers_in_one_step(round);
        let landed = (0..EDITS)
            .filter(|n| after.contains(&format!("marker_{n}_new")))
            .count();
        assert_eq!(landed, EDITS, "round {round}: the file holds\n{after}");
    }
}
