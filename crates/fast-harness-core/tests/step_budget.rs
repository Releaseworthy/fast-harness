//! D84: read cuts a long line at 800 characters, the shell keeps 20 head
//! and 30 tail lines, glob and grep show paths relative to a folder `path`,
//! and the calls of one step share `context.stepOutputBytes`.

mod agent_support;

use std::path::Path;
use std::sync::atomic::AtomicBool;
use std::sync::Arc;

use agent_support::*;
use fast_harness_core::agent::{Agent, Conversation};
use fast_harness_core::config::Config;
use fast_harness_core::http::ScriptedTransport;
use fast_harness_core::llm::ToolSpec;
use fast_harness_core::tools::{budget, builtin, Registry, Tool, ToolCtx, ToolOutput, ToolSession};
use fast_harness_core::types::Block;
use serde_json::{json, Value};

fn run_tool(cwd: &Path, name: &str, args: Value) -> String {
    let cancel = AtomicBool::new(false);
    let ctx = ToolCtx::new(cwd, &cancel);
    builtin().run(name, &args.to_string(), &ctx).into_text()
}

#[test]
fn read_cuts_a_long_line_at_800_characters() {
    let env = env("d84-read");
    std::fs::write(env.cwd.join("min.js"), "x".repeat(5000) + "\nshort\n").unwrap();
    let out = run_tool(&env.cwd, "read", json!({"path": "min.js", "offset": 1}));
    let first = out.lines().next().unwrap();
    let note = "[line cut at 800 of 5000 chars; read on with offset 1, limit 1, column 801]";
    assert!(first.ends_with(note), "{first}");
    assert_eq!(first.matches('x').count(), 800);
    assert!(out.contains("2: short"));
    // The rest of the line, a window at a time.
    let rest = run_tool(
        &env.cwd,
        "read",
        json!({"path": "min.js", "offset": 1, "limit": 1, "column": 801}),
    );
    assert!(
        rest.starts_with(&format!(
            "1: {} [columns 801-2800 of 5000; read on with column 2801]",
            "x".repeat(2000)
        )),
        "{rest}"
    );
    let end = run_tool(
        &env.cwd,
        "read",
        json!({"path": "min.js", "offset": 1, "limit": 1, "column": 4801}),
    );
    assert!(
        end.starts_with(&format!(
            "1: {} [columns 4801-5000 of 5000]\n",
            "x".repeat(200)
        )),
        "{end}"
    );
}

fn numbered(count: usize) -> String {
    (1..=count).map(|i| format!("line {i}\n")).collect()
}

/// The fitted output and the folder its spill file is in (removed on drop).
fn fit_shell(name: &str, full: &str) -> (ToolOutput, Env) {
    let env = env(name);
    let cancel = AtomicBool::new(false);
    let session = ToolSession::new(Some(env.root.0.join("s.out")));
    let ctx = ToolCtx::new(&env.cwd, &cancel)
        .with_call_id("toolu_1")
        .with_session(&session);
    let out = budget::fit("shell", ToolOutput::ok(full.to_string()), &ctx);
    (out, env)
}

#[test]
fn shell_keeps_20_head_and_30_tail_lines_even_under_the_byte_budget() {
    let full = numbered(300);
    assert!(full.len() < 8192);
    let (out, _env) = fit_shell("d84-shell-lines", &full);
    let text = out.text();
    assert!(
        text.starts_with("line 1\n") && text.contains("line 20\n"),
        "{text}"
    );
    assert!(
        !text.contains("line 21\n") && !text.contains("line 270\n"),
        "{text}"
    );
    assert!(
        text.contains("line 271\n") && text.contains("line 300\n"),
        "{text}"
    );
    assert!(text.contains("[... 250 lines, "), "{text}");
    let spilled = out
        .spilled()
        .expect("the whole output is in the spill file");
    assert_eq!(std::fs::read_to_string(spilled).unwrap(), full);
}

#[test]
fn shell_over_the_byte_budget_keeps_at_most_50_lines() {
    let full = numbered(3000);
    assert!(full.len() > 8192);
    let (out, _env) = fit_shell("d84-shell-bytes", &full);
    let kept = out
        .text()
        .lines()
        .filter(|l| l.starts_with("line "))
        .count();
    assert_eq!(kept, 50, "{}", out.text());
    assert!(out.text().contains("line 3000\n"));
    assert!(out.text().contains("[... 2950 lines, "));
}

#[test]
fn a_short_shell_output_is_not_cut() {
    let full = numbered(50);
    let (out, _env) = fit_shell("d84-shell-short", &full);
    assert_eq!(out.text(), full);
    assert!(out.spilled().is_none());
}

#[test]
fn glob_and_grep_in_a_folder_show_paths_relative_to_it() {
    let env = env("d84-paths");
    let deep = env.cwd.join("crates/core/src");
    std::fs::create_dir_all(deep.join("tools")).unwrap();
    std::fs::write(deep.join("tools/a.rs"), "hit\n").unwrap();
    let globbed = run_tool(
        &env.cwd,
        "glob",
        json!({"pattern": "*.rs", "path": "crates/core/src"}),
    );
    assert_eq!(
        globbed,
        "[paths relative to crates/core/src/]\ntools/a.rs\n"
    );
    let grepped = run_tool(
        &env.cwd,
        "grep",
        json!({"pattern": "hit", "path": "crates/core/src"}),
    );
    assert_eq!(
        grepped,
        "[paths relative to crates/core/src/]\ntools/a.rs\n  1: hit\n"
    );
    // Outside the working directory the long start is sent once.
    let outside = env.home.join("outside");
    std::fs::create_dir_all(&outside).unwrap();
    std::fs::write(outside.join("b.rs"), "hit\n").unwrap();
    let path = outside.to_string_lossy().to_string();
    let far = run_tool(&env.cwd, "glob", json!({"pattern": "*.rs", "path": path}));
    let lines: Vec<&str> = far.lines().collect();
    assert_eq!(lines.len(), 2, "{far}");
    assert!(
        lines[0].starts_with("[paths relative to ") && lines[1] == "b.rs",
        "{far}"
    );
    // No path: relative to the working directory, no first line.
    let plain = run_tool(&env.cwd, "glob", json!({"pattern": "*.rs"}));
    assert_eq!(plain, "crates/core/src/tools/a.rs\n");
}

/// Returns `lines` lines of 100 bytes each.
struct Big;

impl Tool for Big {
    fn spec(&self) -> ToolSpec {
        ToolSpec::new("big", "Big output", json!({"type": "object"}))
    }

    fn run(&self, args: &Value, _ctx: &ToolCtx<'_>) -> ToolOutput {
        let count = args["lines"].as_u64().unwrap_or(1) as usize;
        ToolOutput::ok(
            (1..=count)
                .map(|i| format!("{i:>6} {}\n", "x".repeat(92)))
                .collect::<String>(),
        )
    }
}

/// Result sizes of a turn whose first step makes four big calls and whose
/// second step makes one.
fn crowded_turn(name: &str, config: Config) -> Vec<usize> {
    let env = env(name);
    let big = r#"{"lines":400}"#;
    let calls: Vec<(String, &str, &str)> = (1..=4)
        .map(|i| (format!("toolu_{i}"), "big", big))
        .collect();
    let refs: Vec<(&str, &str, &str)> = calls
        .iter()
        .map(|(id, n, a)| (id.as_str(), *n, *a))
        .collect();
    let script = vec![
        tool_reply(&refs),
        tool_reply(&[("toolu_5", "big", big)]),
        text_reply("done"),
    ];
    let transport = Arc::new(ScriptedTransport::new(script));
    let mut parts = parts(&transport, key_auth(), &env.cwd);
    parts.config = config;
    let mut registry = Registry::new();
    registry.add(Box::new(Big));
    let agent = Agent::new(parts, Arc::new(registry));
    let mut conv = Conversation::create(&env.home, &env.cwd, None).unwrap();
    agent.run_turn(&mut conv, "go", &AtomicBool::new(false), &mut |_| {});
    let blocks = conv.messages.iter().flat_map(|m| m.content.iter());
    blocks
        .filter_map(|block| match block {
            Block::ToolResult { content, .. } => Some(content.len()),
            _ => None,
        })
        .collect()
}

#[test]
fn the_calls_of_one_step_share_the_step_budget() {
    let sizes = crowded_turn("d84-step", Config::default());
    assert_eq!(sizes.len(), 5, "{sizes:?}");
    for size in &sizes[..4] {
        assert!(*size <= 4096, "a quarter of 16 KB each: {sizes:?}");
    }
    assert!(sizes[..4].iter().sum::<usize>() <= 16 * 1024);
    assert!(
        sizes[4] > 4096 && sizes[4] <= 8192,
        "a lone call keeps 8 KB: {sizes:?}"
    );
    // `stepOutputBytes: 0` gives every call its own budget again.
    let mut off = Config::default();
    off.context.step_output_bytes = 0;
    let old = crowded_turn("d84-step-off", off);
    assert!(old[..4].iter().all(|size| *size > 4096), "{old:?}");
}
