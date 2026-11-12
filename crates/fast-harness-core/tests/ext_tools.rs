//! Tool files (PLAN.md D42): command tools, built-in overrides, the lazy
//! index and its cache, and an untrusted project.

mod agent_support;

use std::fs::{self, File};
use std::path::Path;
use std::sync::atomic::AtomicBool;
use std::time::{Duration, SystemTime};

use agent_support::*;
use fast_harness_core::app::Runtime;
use fast_harness_core::config::index::{ExtIndex, INDEX_FILE};
use fast_harness_core::config::layers::Layers;
use fast_harness_core::tools::{Effect, Extensions, ToolCtx};
use serde_json::json;

fn put(path: &Path, text: &str) {
    fs::create_dir_all(path.parent().unwrap()).unwrap();
    fs::write(path, text).unwrap();
}

/// A tool file running `script` (cmd on Windows, sh elsewhere).
fn command_tool(home: &Path, name: &str, script: &[&str], timeout_ms: u64) {
    let (command, mut args) = match cfg!(windows) {
        true => ("cmd", vec!["/c".to_string()]),
        false => ("sh", vec!["-c".to_string()]),
    };
    match cfg!(windows) {
        true => args.extend(script.iter().map(|s| s.to_string())),
        false => args.push(script.join(" ")),
    }
    let file = json!({"name": name, "description": format!("The {name} tool"),
        "schema": {"type": "object", "properties": {"who": {"type": "string"}}},
        "command": command, "args": args, "timeoutMs": timeout_ms});
    put(
        &home.join("tools").join(format!("{name}.json")),
        &file.to_string(),
    );
}

fn extensions(home: &Path) -> Extensions {
    Extensions::new(ExtIndex::build(&Layers::user_only(home)))
}

fn run(ext: &Extensions, cwd: &Path, name: &str, args: &str) -> (String, bool) {
    let cancel = AtomicBool::new(false);
    let output = ext.registry().run(name, args, &ToolCtx::new(cwd, &cancel));
    (output.content, output.is_error)
}

#[test]
fn command_tool_round_trip_timeout_and_failure() {
    let env = env("ext-command");
    let echo: &[&str] = if cfg!(windows) { &["more"] } else { &["cat"] };
    command_tool(&env.home, "echo_json", echo, 20_000);
    let slow: &[&str] = match cfg!(windows) {
        true => &["ping", "-n", "30", "127.0.0.1"],
        false => &["sleep", "30"],
    };
    command_tool(&env.home, "slow", slow, 300);
    let fail: &[&str] = match cfg!(windows) {
        true => &["echo", "bad", "1>&2", "&", "exit", "/b", "3"],
        false => &["echo bad >&2; exit 3"],
    };
    command_tool(&env.home, "fail", fail, 20_000);
    let ext = extensions(&env.home);
    let registry = ext.registry();
    let find = &registry.specs()[0];
    assert!(
        find.description.contains("echo_json"),
        "deferred behind find_tools"
    );
    let spec = registry
        .specs()
        .iter()
        .find(|s| s.name == "echo_json")
        .unwrap();
    assert_eq!(spec.schema["properties"]["who"]["type"], "string");
    let (out, error) = run(&ext, &env.cwd, "echo_json", r#"{"who":"x"}"#);
    assert!(!error, "{out}");
    assert!(out.contains(r#"{"who":"x"}"#), "{out}");
    let started = std::time::Instant::now();
    let (out, error) = run(&ext, &env.cwd, "slow", "{}");
    assert!(error && out.contains("timed out"), "{out}");
    assert!(started.elapsed() < Duration::from_secs(10));
    let (out, error) = run(&ext, &env.cwd, "fail", "{}");
    assert!(error, "{out}");
    assert!(out.contains("exit code: 3") && out.contains("bad"), "{out}");
}

#[test]
fn a_file_overrides_read_budget_and_disables_a_built_in() {
    let env = env("ext-override");
    let tools = env.home.join("tools");
    put(&tools.join("read.json"), r#"{"outputBytes": 1500}"#);
    put(&tools.join("grep.json"), r#"{"disabled": true}"#);
    put(
        &tools.join("glob.json"),
        r#"{"description": "Find files (custom)."}"#,
    );
    let mut rt = Runtime::load_at(env.home.clone()).unwrap();
    rt.use_project(&env.cwd).unwrap();
    assert_eq!(rt.config.tool_budget("read"), 1500);
    let registry = rt.extensions.registry();
    let names: Vec<&str> = registry.specs().iter().map(|s| s.name.as_str()).collect();
    assert!(
        !names.contains(&"grep") && names.contains(&"read"),
        "{names:?}"
    );
    let glob = registry.specs().iter().find(|s| s.name == "glob").unwrap();
    assert_eq!(glob.description, "Find files (custom).");
    let settings = fast_harness_core::app::load_settings(&env.home, Some(&env.cwd), &|_| None);
    let report = fast_harness_core::app::doctor(&settings.unwrap());
    assert!(report.contains("grep [disabled] (user)"), "{report}");
    assert!(report.contains("read [override] (user)"), "{report}");
    fs::write(env.cwd.join("big.txt"), "line of text\n".repeat(1000)).unwrap();
    let cancel = AtomicBool::new(false);
    let mut ctx = ToolCtx::new(&env.cwd, &cancel);
    ctx.budget = rt.config.tool_budget("read");
    let out = registry.run("read", r#"{"path":"big.txt"}"#, &ctx);
    // The default budget (8 KB) would return far more of the 13 KB file.
    let len = out.content.len();
    assert!((1000..=1500).contains(&len), "{len}");
}

/// Sets the modified time of `path`.
fn touch(path: &Path, when: SystemTime) {
    File::options()
        .write(true)
        .open(path)
        .unwrap()
        .set_modified(when)
        .unwrap();
}

#[test]
fn index_reads_nothing_on_a_cache_hit_and_rereads_a_changed_file() {
    let env = env("ext-index");
    let layers = Layers::user_only(&env.home);
    let empty = ExtIndex::build(&layers);
    assert_eq!((empty.reads, empty.cached), (0, 0));
    assert!(!env.home.join(INDEX_FILE).exists(), "no folder, no work");
    let file = env.home.join("agents").join("rev.md");
    put(&file, "---\ndescription: aaaa\n---\nBody");
    let when = SystemTime::now() - Duration::from_secs(60);
    touch(&file, when);
    let first = ExtIndex::build(&layers);
    assert_eq!((first.reads, first.cached), (1, 0));
    assert_eq!(first.agents[0].entry.description, "aaaa");
    // Same size and time, other text: served from index.json unread.
    fs::write(&file, "---\ndescription: bbbb\n---\nBody").unwrap();
    touch(&file, when);
    let hit = ExtIndex::build(&layers);
    assert_eq!((hit.reads, hit.cached), (0, 1));
    assert_eq!(hit.agents[0].entry.description, "aaaa");
    touch(&file, when + Duration::from_secs(5));
    let changed = ExtIndex::build(&layers);
    assert_eq!((changed.reads, changed.cached), (1, 0));
    assert_eq!(changed.agents[0].entry.description, "bbbb");
}

#[test]
fn an_untrusted_project_loads_no_tool_files() {
    let env = env("ext-untrusted");
    let project = env.cwd.join(".fast-harness").join("tools");
    let echo: &[&str] = if cfg!(windows) { &["more"] } else { &["cat"] };
    command_tool(&env.cwd.join(".fast-harness"), "proj_tool", echo, 1000);
    assert!(project.join("proj_tool.json").is_file());
    let mut rt = Runtime::load_at(env.home.clone()).unwrap();
    rt.use_project(&env.cwd).unwrap();
    assert!(rt.extensions.index.tools.is_empty());
    let names = |rt: &Runtime| -> Vec<String> {
        let registry = rt.extensions.registry();
        registry.specs().iter().map(|s| s.name.clone()).collect()
    };
    assert!(!names(&rt).contains(&"proj_tool".to_string()));
    rt.trust_project(&env.cwd).unwrap();
    assert!(names(&rt).contains(&"proj_tool".to_string()));
}

/// A tool file's `effect` (PLAN.md D54): `read` makes its calls parallel,
/// none keeps them serial (`Run`), an unknown word skips the file.
#[test]
fn a_tool_file_declares_its_effect() {
    let env = env("ext-effect");
    let tools = env.home.join("tools");
    let tool = |effect: &str| {
        let effect = match effect {
            "" => String::new(),
            word => format!(r#","effect":{word}"#),
        };
        format!(r#"{{"description":"A tool","command":"x"{effect}}}"#)
    };
    put(&tools.join("looker.json"), &tool(r#""read""#));
    put(&tools.join("plain.json"), &tool(""));
    put(&tools.join("writer.json"), &tool(r#""write""#));
    put(&tools.join("bad.json"), &tool(r#""fast""#));
    let index = ExtIndex::build(&Layers::user_only(&env.home));
    let skipped: Vec<&str> = index.skipped.iter().map(|s| s.reason.as_str()).collect();
    assert!(
        skipped.iter().any(|reason| reason.contains("bad `effect`")),
        "{skipped:?}"
    );
    let ext = Extensions::new(index);
    let registry = ext.registry();
    assert!(registry.parallel("looker"));
    assert!(!registry.parallel("plain") && !registry.parallel("writer"));
    assert_eq!(registry.effect("writer"), Some(Effect::Write));
    assert_eq!(registry.effect("plain"), Some(Effect::Run));
    assert_eq!(registry.effect("bad"), None, "a skipped file is no tool");
    assert!(registry.parallel("find_tools") && registry.parallel("no_such_tool"));
}
