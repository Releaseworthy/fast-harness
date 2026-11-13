//! The events of `old::lines` through the typed writer, in the same order.

use super::*;

fn context<'a>(attempt: u32, effort: Option<&'a str>, wire: Option<&'a str>) -> RequestContext<'a> {
    RequestContext {
        turn: 1,
        step: 1,
        attempt,
        provider: "anthropic",
        model: "m",
        account: "a",
        effort,
        wire,
    }
}

fn compaction(f: &Fixture) -> record::Compaction {
    record::Compaction {
        turn: Some(1),
        step: Some(2),
        summary: "s".into(),
        facts: Some("f".into()),
        loaded: Some(vec!["read".into()]),
        kept: Some(vec![]),
        call_id: Some("compact_1_2".into()),
        mid: Some(true),
        before: Some(10),
        after: Some(5),
        usage: Some(f.usage),
        auto: Some(true),
        model: Some(Some("m".into())),
        instructions: Some(None),
    }
}

fn hooks(w: &mut TraceWriter) {
    let (event, command) = ("PreToolUse".to_string(), "c".to_string());
    let decision = "none".to_string();
    let hook = record::Hook {
        turn: 1,
        event,
        command: command.clone(),
        ms: 4,
        decision,
    };
    w.write(&Record::Hook(hook)).unwrap();
    let (event, error) = ("Stop".to_string(), "boom".to_string());
    let failed = record::HookError {
        turn: 1,
        event,
        command,
        ms: 4,
        error,
    };
    w.write(&Record::HookError(failed)).unwrap();
}

pub fn write_all(w: &mut TraceWriter, f: &Fixture) {
    w.turn_start(1).unwrap();
    w.user_message(1, &f.message).unwrap();
    w.user_text(1, &f.message, "steer").unwrap();
    w.turn_resume(2).unwrap();
    w.step_start(1, 1).unwrap();
    w.request_header("sys", std::slice::from_ref(&f.tool))
        .unwrap();
    let stamp = json!({"date": "d"});
    let header =
        fast_harness_core::trace::header_data("sys", std::slice::from_ref(&f.tool), Some(&stamp));
    w.request_header_data(header).unwrap();
    w.request_context(&context(1, Some("high"), Some("t1-s1-a1")))
        .unwrap();
    w.request_context(&context(2, None, None)).unwrap();
    w.assistant_attempt(1, 1, 1, "e", &f.timing).unwrap();
    w.llm_retry(1, 1, 1, 2000, "e").unwrap();
    w.account_switch("a", "b", "RateLimit", Some(9)).unwrap();
    w.account_switch("a", "b", "Manual", None).unwrap();
    let (message, usage, timing) = (&f.message, &f.usage, &f.timing);
    w.assistant_message(
        1,
        1,
        message,
        usage,
        StopReason::ToolUse,
        timing,
        "a",
        1_000_000,
    )
    .unwrap();
    let transformations = json!([{"type": "prefix_binding_mismatch"}]);
    w.request_transformations(1, 1, &transformations).unwrap();
    w.tool_call(1, 1, "c1", "read", "{}").unwrap();
    w.tool_result(1, 1, &f.result, 3).unwrap();
    w.tool_result_sized(1, 1, &f.result, 3, (9000, Some("x.out/c1.txt")))
        .unwrap();
    w.compaction(compaction(f)).unwrap();
    w.step_end(1, 1, 5120).unwrap();
    w.turn_end(1, 3, &f.usage, 14200, "done").unwrap();
    hooks(w);
    w.tool_approval(1, 1, "c1", "once").unwrap();
}
