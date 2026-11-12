//! Phase 12 output diet (PLAN.md D37): result budgets, spill files, read
//! ranges and the unchanged answer, grep/glob/shell/subagent limits, the
//! trace `bytes` and `spilled` keys, and the 60-step acceptance run.

mod agent_support;

use std::path::Path;
use std::sync::atomic::AtomicBool;
use std::sync::Arc;

use agent_support::*;
use fast_harness_core::agent::{Agent, Conversation};
use fast_harness_core::config::Config;
use fast_harness_core::http::{ScriptedResponse, ScriptedTransport};
use fast_harness_core::llm::ToolSpec;
use fast_harness_core::stats;
use fast_harness_core::tools::ToolSession;
use fast_harness_core::tools::{budget, builtin, find, Registry, Tool, ToolCtx, ToolOutput};
use fast_harness_core::view::tool_detail;
use serde_json::{json, Value};

/// Returns `lines` numbered lines of 100 bytes each.
struct Big;

impl Tool for Big {
    fn spec(&self) -> ToolSpec {
        ToolSpec::new("big", "Big output", json!({"type": "object"}))
    }

    fn run(&self, args: &Value, _ctx: &ToolCtx<'_>) -> ToolOutput {
        ToolOutput::ok(lines(args["lines"].as_u64().unwrap_or(1) as usize))
    }
}

fn lines(count: usize) -> String {
    (1..=count)
        .map(|i| format!("{i:>6} {}\n", "x".repeat(92)))
        .collect()
}

/// Runs a built-in tool outside a session with `budget`.
fn run_tool(cwd: &Path, name: &str, args: Value, budget: usize) -> ToolOutput {
    let cancel = AtomicBool::new(false);
    let ctx = ToolCtx::new(cwd, &cancel).with_budget(budget);
    builtin().run(name, &args.to_string(), &ctx)
}

fn session_ctx<'a>(
    cwd: &'a Path,
    cancel: &'a AtomicBool,
    session: &'a ToolSession,
    call_id: &'a str,
) -> ToolCtx<'a> {
    ToolCtx::new(cwd, cancel)
        .with_call_id(call_id)
        .with_session(session)
}

#[test]
fn read_returns_whole_lines_within_the_budget_then_the_range() {
    let env = env("diet-read");
    std::fs::write(env.cwd.join("f.txt"), lines(1000)).unwrap();
    let out = run_tool(&env.cwd, "read", json!({"path": "f.txt"}), 8192);
    assert!(out.content.len() <= 8192, "{}", out.content.len());
    assert!(out.spilled.is_none());
    let last = out.content.lines().last().unwrap();
    let shown = out.content.lines().count() - 1;
    let expect = format!(
        "[lines 1 to {shown} of 1000; continue with offset {}]",
        shown + 1
    );
    assert_eq!(last, expect);
    assert!(
        out.content.starts_with("1:      1 "),
        "{}",
        &out.content[..20]
    );
    let next = run_tool(
        &env.cwd,
        "read",
        json!({"path": "f.txt", "offset": shown + 1}),
        8192,
    );
    assert!(next.content.starts_with(&format!("{}: ", shown + 1)));
}

#[test]
fn an_unchanged_range_answers_with_the_earlier_call() {
    let env = env("diet-unchanged");
    let file = env.cwd.join("f.txt");
    std::fs::write(&file, "one\ntwo\n").unwrap();
    let cancel = AtomicBool::new(false);
    let session = ToolSession::new(None);
    let tools = builtin();
    let read = |id: &str, args: Value| {
        let ctx = session_ctx(&env.cwd, &cancel, &session, id);
        tools.run("read", &args.to_string(), &ctx).content
    };
    assert_eq!(read("c1", json!({"path": "f.txt"})), "1: one\n2: two\n");
    assert_eq!(
        read("c2", json!({"path": "f.txt"})),
        "[unchanged since call c1]"
    );
    // Another range is a new read; so is the same range after a change.
    assert_eq!(
        read("c3", json!({"path": "f.txt", "offset": 2})),
        "2: two\n"
    );
    std::fs::write(&file, "one\ntwo\nthree\n").unwrap();
    assert!(read("c4", json!({"path": "f.txt"})).contains("3: three"));
    assert_eq!(
        read("c5", json!({"path": "f.txt"})),
        "[unchanged since call c4]"
    );
}

#[test]
fn grep_shows_fifty_matches_grouped_then_counts_the_rest() {
    let env = env("diet-grep");
    for name in ["a.txt", "b.txt", "c.txt"] {
        let text: String = (0..25).map(|i| format!("hit {i}\n")).collect();
        std::fs::write(env.cwd.join(name), text).unwrap();
    }
    let out = run_tool(&env.cwd, "grep", json!({"pattern": "hit"}), 8192);
    let shown = out.content.lines().filter(|l| l.starts_with("  ")).count();
    assert_eq!(shown, 50, "{}", out.content);
    assert_eq!(
        out.content.lines().filter(|l| l.ends_with(".txt")).count(),
        2
    );
    assert!(
        out.content.contains("[25 more matches in 1 file;"),
        "{}",
        out.content
    );
}

#[test]
fn glob_shows_one_hundred_paths() {
    let env = env("diet-glob");
    for i in 0..120 {
        std::fs::write(env.cwd.join(format!("f{i}.rs")), "").unwrap();
    }
    let out = run_tool(&env.cwd, "glob", json!({"pattern": "*.rs"}), 8192);
    assert_eq!(
        out.content.lines().filter(|l| l.ends_with(".rs")).count(),
        100
    );
    assert!(
        out.content.contains("[20 more files not shown"),
        "{}",
        out.content
    );
}

#[test]
fn shell_keeps_head_and_tail_and_spills_the_whole_output() {
    let env = env("diet-shell");
    let cancel = AtomicBool::new(false);
    let session = ToolSession::new(Some(env.root.0.join("s1.out")));
    let ctx = session_ctx(&env.cwd, &cancel, &session, "toolu_9");
    let full = lines(1000) + "[exit code: 1]";
    let out = budget::fit("shell", ToolOutput::err(full.clone()), &ctx);
    assert!(out.content.len() <= 8192, "{}", out.content.len());
    assert_eq!(out.bytes, full.len());
    assert!(out.content.starts_with("     1 x"));
    assert!(out.content.contains("bytes cut ...]"));
    let path = out.spilled.clone().expect("spilled");
    assert!(path.ends_with("toolu_9.txt"), "{path}");
    assert_eq!(std::fs::read_to_string(&path).unwrap(), full);
    let note = format!(
        "\n[full output: {path}, {} bytes, 1001 lines; read it with offset and limit or grep it]",
        full.len()
    );
    assert!(out.content.ends_with(&note), "{}", out.content);
    let body = &out.content[..out.content.len() - note.len()];
    assert!(body.ends_with("[exit code: 1]"), "errors stay at the end");
    let head = body.split("[...").next().unwrap();
    assert!((1500..=2048).contains(&head.len()), "head {}", head.len());
}

#[test]
fn subagent_keeps_its_stats_line_and_outside_a_session_nothing_spills() {
    let env = env("diet-sub");
    let cancel = AtomicBool::new(false);
    let ctx = ToolCtx::new(&env.cwd, &cancel);
    let stats = "[subagent session s1 \u{b7} 3 steps]";
    let answer = format!("{}\n\n{stats}", "word ".repeat(4000));
    let out = budget::fit("subagent", ToolOutput::ok(answer), &ctx);
    assert!(out.content.len() <= 8192);
    assert!(out.spilled.is_none());
    assert!(
        out.content.contains(&format!("\n\n{stats}\n[output cut: ")),
        "{}",
        out.content
    );
}

#[test]
fn budgets_come_from_config_with_a_per_tool_override() {
    let config: Config = serde_json::from_str(
        r#"{"toolOutputBytes": 4096, "toolOutput": {"read": 16384, "glob": 10}}"#,
    )
    .unwrap();
    assert_eq!(config.tool_budget("read"), 16384);
    assert_eq!(config.tool_budget("grep"), 4096);
    assert_eq!(config.tool_budget("glob"), 1024, "the floor");
    assert_eq!(Config::default().tool_budget("shell"), 8192);
    let env = env("diet-override");
    std::fs::write(env.cwd.join("f.txt"), lines(1000)).unwrap();
    let out = run_tool(
        &env.cwd,
        "read",
        json!({"path": "f.txt"}),
        config.tool_budget("read"),
    );
    assert!(
        (15_000..=16384).contains(&out.content.len()),
        "{}",
        out.content.len()
    );
}

#[test]
fn find_tools_takes_a_limit() {
    let spec = |name: &str| ToolSpec::new(name, "d", json!({}));
    let others: Vec<ToolSpec> = ["alpha", "beta", "gamma"].iter().map(|n| spec(n)).collect();
    let names = json!(["alpha", "beta", "gamma"]);
    let out = find::run(&json!({"names": names, "limit": 2}), &others);
    assert!(
        out.content.contains("beta") && !out.content.contains("gamma"),
        "{}",
        out.content
    );
    let out = find::run(&json!({"names": names}), &others);
    assert!(out.content.contains("gamma"), "{}", out.content);
}

#[test]
fn the_tool_card_names_the_spill_file() {
    let content = "head\n[full output: C:\\s\\x.out\\c1.txt, 9000 bytes, 90 lines; read it with offset and limit or grep it]";
    let detail = tool_detail("shell", r#"{"command":"dir"}"#, Some(content), false, None);
    let field = detail.fields.iter().find(|f| f.label == "full output");
    assert_eq!(
        field.map(|f| f.value.as_str()),
        Some("C:\\s\\x.out\\c1.txt")
    );
}

/// A 60-step session: 59 tool steps (ten of them 40 KB results), then an
/// answer. Returns the body size of the step 60 request and the log events.
fn sixty_steps(
    name: &str,
    config: Config,
) -> (usize, Vec<fast_harness_core::trace::TraceEvent>, Env) {
    let env = env(name);
    let mut script: Vec<ScriptedResponse> = (1..=59)
        .map(|step| {
            let id = format!("toolu_{step}");
            match step % 5 == 0 && step <= 50 {
                true => tool_reply(&[(&id, "big", r#"{"lines":400}"#)]),
                false => tool_reply(&[(&id, "echo", r#"{"text":"ok"}"#)]),
            }
        })
        .collect();
    script.push(text_reply("done"));
    let transport = Arc::new(ScriptedTransport::new(script));
    let mut parts = parts(&transport, key_auth(), &env.cwd);
    parts.config = config;
    let mut registry = Registry::new();
    registry.add(Box::new(Big));
    registry.add(Box::new(Echo));
    let agent = Agent::new(parts, Arc::new(registry));
    let mut conv = Conversation::create(&env.home, &env.cwd, None).unwrap();
    let cancel = AtomicBool::new(false);
    agent.run_turn(&mut conv, "go", &cancel, &mut |_| {});
    let requests = transport.requests();
    assert_eq!(requests.len(), 60);
    let last = requests.last().unwrap().body.len();
    (last, events(&conv.log_path), env)
}

#[test]
fn sixty_step_session_halves_the_prompt_and_keeps_every_budget() {
    let old = Config {
        tool_output_bytes: 32 * 1024,
        ..Config::default()
    };
    let (before, _, _env1) = sixty_steps("diet-old", old);
    let (after, events, _env2) = sixty_steps("diet-new", Config::default());
    eprintln!("step 60 request bytes: 32 KB clip {before}, 8 KB budget {after}");
    assert!(after * 2 <= before, "{after} is not half of {before}");
    let results: Vec<&Value> = events
        .iter()
        .filter(|e| e.kind == "tool/result")
        .map(|e| &e.data)
        .collect();
    assert_eq!(results.len(), 59);
    for data in &results {
        let content = data["message"]["content"][0]["content"].as_str().unwrap();
        assert!(content.len() <= 8192, "{}", content.len());
        assert!(data["bytes"].as_u64().unwrap() >= content.len() as u64);
    }
    let spilled: Vec<&str> = results
        .iter()
        .filter_map(|d| d["spilled"].as_str())
        .collect();
    assert_eq!(spilled.len(), 10);
    assert!(spilled.iter().all(|p| Path::new(p).is_file()));
    assert!(spilled[0].contains(".out"), "{}", spilled[0]);
    assert_eq!(stats::compute("s", &events).clipped, 10);
}
