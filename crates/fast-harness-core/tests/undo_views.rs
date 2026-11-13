//! The undo views' core (PLAN.md D69 "Views"): the notes a front end shows
//! for an undo it ran equal what a replay of the session log shows.

mod agent_support;

use std::sync::atomic::AtomicBool;
use std::sync::Arc;

use agent_support::{env, key_auth, parts, text_reply, tool_reply};
use fast_harness_core::agent::{Agent, Conversation};
use fast_harness_core::http::ScriptedTransport;
use fast_harness_core::service::undo::undo_conv;
use fast_harness_core::tools;
use fast_harness_core::view::{Item, Transcript};
use serde_json::json;

fn notes(items: &[Item]) -> Vec<String> {
    (items.iter())
        .filter_map(|item| match item {
            Item::Note { kind, text } => Some(format!("{kind:?} {text}")),
            _ => None,
        })
        .collect()
}

#[test]
fn the_live_undo_notes_equal_the_replayed_ones() {
    let env = env("undo-views-parity");
    std::fs::write(env.cwd.join("p.txt"), "before").unwrap();
    let args = json!({"path": "p.txt", "content": "after"}).to_string();
    let script = vec![tool_reply(&[("c1", "write", &args)]), text_reply("ok")];
    let transport = Arc::new(ScriptedTransport::new(script));
    let mut agent = Agent::new(
        parts(&transport, key_auth(), &env.cwd),
        Arc::new(tools::builtin()),
    );
    agent.preload = vec!["write".into()];
    let mut conv = Conversation::create(&env.home, &env.cwd, None).unwrap();
    agent.run_turn(&mut conv, "change p", &AtomicBool::new(false), &mut |_| {});
    let outcome = undo_conv(&mut conv, false).unwrap();
    let mut live = Transcript::new();
    live.undone(&outcome);
    let live = notes(&live.items);
    assert_eq!(live.len(), 2, "{live:?}");
    assert!(live[0].contains("undid the file changes of turn 1: restored "));
    let id = conv.id.clone();
    drop(conv);
    let (conv, events) = Conversation::reopen(&env.home, &env.cwd, &id).unwrap();
    let replay = Transcript::opened(&events, &conv.messages, conv.take_notes());
    let replay = notes(&replay.items);
    assert_eq!(replay[replay.len() - live.len()..], live[..]);
    assert_eq!(
        std::fs::read_to_string(env.cwd.join("p.txt")).unwrap(),
        "before"
    );
}
