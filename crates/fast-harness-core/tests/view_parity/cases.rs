//! The scripted turns of the parity table (`view_parity.rs`).

use super::*;

type Case = fn(&Env) -> Run;

pub const CASES: &[(&str, Case)] = &[
    ("plain-text", plain),
    ("thinking-then-text", thinking),
    ("one-tool", one_tool),
    ("parallel-reads-and-a-write", mixed_step),
    ("tool-error", tool_error),
    ("steer-in-tool-step", steer),
    ("follow-up-and-next-turn", follow_up),
    ("cancelled", cancelled),
    ("paused-then-resumed", paused),
    ("retry", retry),
    ("account-switch", switch),
    ("manual-compaction", manual_compaction),
    ("auto-compaction", auto_compaction),
    ("subagent", subagent),
    ("failed", failed),
    ("find-tools", find_tools),
    ("spilled-result", spilled),
    ("approvals-allowed-and-declined", approvals),
];

const SUMMARY: &str = "## Task\nfix it\n## Files\nsrc/a.rs\n## Durable facts\nuse pwsh";

fn tool_then_text() -> Vec<ScriptedResponse> {
    vec![tool_reply(&[("t1", "echo", HI)]), text_reply("done")]
}

fn plain(env: &Env) -> Run {
    let mut run = scripted(env, vec![text_reply("hello")]);
    assert_eq!(run.turn("hi"), TurnOutcome::Done);
    run
}

fn thinking(env: &Env) -> Run {
    let mut run = scripted(env, vec![thinking_reply("let me see", "the answer")]);
    assert_eq!(run.turn("why"), TurnOutcome::Done);
    run
}

fn one_tool(env: &Env) -> Run {
    let mut run = scripted(env, tool_then_text());
    assert_eq!(run.turn("go"), TurnOutcome::Done);
    run
}

fn mixed_step(env: &Env) -> Run {
    let calls = [
        ("t1", "echo", HI),
        ("t2", "echo", r#"{"text":"two","sleepMs":20}"#),
        ("t3", "jot", HI),
    ];
    let mut run = scripted(env, vec![tool_reply(&calls), text_reply("done")]);
    assert_eq!(run.turn("go"), TurnOutcome::Done);
    run
}

fn tool_error(env: &Env) -> Run {
    let calls = [("t1", "nope", "{}"), ("t2", "echo", "{not json")];
    let mut run = scripted(env, vec![tool_reply(&calls), text_reply("done")]);
    assert_eq!(run.turn("go"), TurnOutcome::Done);
    run
}

fn steer(env: &Env) -> Run {
    let mut run = scripted(env, tool_then_text());
    let input = Some(Input::Steer("also check the tests".into()));
    assert_eq!(run.turn_with("go", input, on_tool_call), TurnOutcome::Done);
    run
}

fn follow_up(env: &Env) -> Run {
    let replies = vec![
        tool_reply(&[("t1", "echo", HI)]),
        text_reply("done"),
        text_reply("followed"),
        text_reply("third"),
    ];
    let mut run = scripted(env, replies);
    let input = Some(Input::FollowUp("and then".into()));
    assert_eq!(run.turn_with("go", input, on_tool_call), TurnOutcome::Done);
    assert_eq!(run.turn("one more"), TurnOutcome::Done);
    run
}

fn cancelled(env: &Env) -> Run {
    let mut run = scripted(env, tool_then_text());
    assert_eq!(
        run.turn_with("go", None, on_tool_call),
        TurnOutcome::Cancelled
    );
    run
}

fn paused(env: &Env) -> Run {
    let mut run = scripted(env, tool_then_text());
    let outcome = run.turn_with("go", Some(Input::Pause), on_tool_call);
    assert_eq!(outcome, TurnOutcome::Paused);
    assert_eq!(run.resume(), TurnOutcome::Done);
    run
}

fn retry(env: &Env) -> Run {
    let busy = error_reply(529, "overloaded_error", "busy").with_header("retry-after", "0");
    let mut run = scripted(env, vec![busy, text_reply("fine")]);
    assert_eq!(run.turn("go"), TurnOutcome::Done);
    run
}

fn switch(env: &Env) -> Run {
    let limited = error_reply(429, "rate_limit_error", "slow down");
    let auth = FakeAuth::new(vec!["acct-a", "acct-b"], Failover::Retry);
    let mut run = build(env, vec![limited, text_reply("fine")], auth, |_| {});
    assert_eq!(run.turn("go"), TurnOutcome::Done);
    run
}

fn manual_compaction(env: &Env) -> Run {
    let replies = vec![text_reply("hi"), text_reply(SUMMARY), text_reply("after")];
    let mut run = scripted(env, replies);
    assert_eq!(run.turn("hello"), TurnOutcome::Done);
    assert_eq!(run.turn("/compact keep the paths"), TurnOutcome::Done);
    assert_eq!(run.turn("more"), TurnOutcome::Done);
    run
}

fn auto_compaction(env: &Env) -> Run {
    let replies = vec![
        text_reply("hi"),
        text_reply(SUMMARY),
        tool_reply(&[("t1", "echo", HI)]),
        text_reply(SUMMARY),
        text_reply("next answer"),
    ];
    let small = |parts: &mut AgentParts| parts.config.compaction.max_tokens = Some(10);
    let mut run = build(env, replies, key_auth(), small);
    assert_eq!(run.turn("hello"), TurnOutcome::Done);
    assert_eq!(run.turn("more"), TurnOutcome::Done);
    run
}

fn subagent(env: &Env) -> Run {
    let replies = vec![
        tool_reply(&[("t0", "find_tools", r#"{"names":["subagent"]}"#)]),
        tool_reply(&[("t1", "subagent", r#"{"prompt":"task A"}"#)]),
        text_reply("answer A"),
        text_reply("done"),
    ];
    let mut run = with_subagent(env, replies);
    assert_eq!(run.turn("go"), TurnOutcome::Done);
    run
}

fn failed(env: &Env) -> Run {
    let bad = error_reply(400, "invalid_request_error", "bad request");
    let mut run = scripted(env, vec![bad]);
    assert_eq!(run.turn("go"), TurnOutcome::Failed);
    run
}

fn find_tools(env: &Env) -> Run {
    let replies = vec![
        tool_reply(&[("t0", "find_tools", r#"{"query":"read a file"}"#)]),
        tool_reply(&[("t1", "find_tools", r#"{"names":["read"]}"#)]),
        text_reply("loaded"),
    ];
    let mut run = with_subagent(env, replies);
    assert_eq!(run.turn("go"), TurnOutcome::Done);
    run
}

fn spilled(env: &Env) -> Run {
    let long = format!(r#"{{"text":"{}"}}"#, "line of output\\n".repeat(400));
    let replies = vec![tool_reply(&[("t1", "echo", &long)]), text_reply("done")];
    let budget = |parts: &mut AgentParts| parts.config.tool_output_bytes = 1024;
    let mut run = build(env, replies, key_auth(), budget);
    assert_eq!(run.turn("go"), TurnOutcome::Done);
    let spill = run.conv.log_path.with_extension("out");
    assert!(spill.is_dir(), "the result spilled to {}", spill.display());
    run
}

/// Two `jot` calls (`Effect::Run`) with the gate on: the first is allowed,
/// the second declined, and both rows say so after a replay (D64).
fn approvals(env: &Env) -> Run {
    let calls = [("t1", "jot", HI), ("t2", "jot", r#"{"text":"two"}"#)];
    let replies = vec![tool_reply(&calls), text_reply("done")];
    let plan = vec![Answer::Once, Answer::No(Some("not now".into()))];
    let mut run = approving(env, replies, plan);
    assert_eq!(run.turn("go"), TurnOutcome::Done);
    let word = |id: &str| run.live.approvals.word(id);
    assert_eq!(
        word("t1"),
        Some("allowed once"),
        "the first call was allowed"
    );
    assert_eq!(word("t2"), Some("declined"), "the second was declined");
    assert!(!run.conv.messages.iter().any(|m| {
        m.content.iter().any(
            |b| matches!(b, fast_harness_core::types::Block::ToolResult { content, .. } if content == "noted two"),
        )
    }));
    run
}
