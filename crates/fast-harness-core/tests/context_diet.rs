//! Context diet (PLAN.md D82): a read without a range is a head with the
//! line count, lines already returned answer `[unchanged since call ID]`,
//! and a scripted 48-step read-heavy turn sends smaller requests while
//! every request stays an append-only extension of the one before (the
//! cached prefix holds).

mod agent_support;

use std::path::Path;
use std::sync::atomic::AtomicBool;
use std::sync::Arc;

use agent_support::*;
use fast_harness_core::agent::{Agent, Conversation};
use fast_harness_core::config::context::ContextConfig;
use fast_harness_core::config::Config;
use fast_harness_core::http::{ScriptedResponse, ScriptedTransport};
use fast_harness_core::tools::{builtin, ToolCtx, ToolSession};
use serde_json::{json, Value};

/// `count` code-like lines of about 46 bytes (the owner's session averaged
/// 46 bytes per read line).
fn code(count: usize) -> String {
    (1..=count)
        .map(|i| format!("    let value_{i:04} = compute(input, {i:04}); // x\n"))
        .collect()
}

fn read_with(cwd: &Path, args: Value, context: &ContextConfig) -> String {
    let cancel = AtomicBool::new(false);
    let ctx = ToolCtx::new(cwd, &cancel)
        .with_budget(8192)
        .with_context(context);
    builtin().run("read", &args.to_string(), &ctx).into_text()
}

#[test]
fn a_read_without_a_range_is_a_head_that_names_the_line_count() {
    let env = env("ctx-head");
    std::fs::write(env.cwd.join("f.rs"), code(1000)).unwrap();
    let context = ContextConfig::default();
    let head = read_with(&env.cwd, json!({"path": "f.rs"}), &context);
    assert!(head.len() <= 4096, "{}", head.len());
    let shown = head.lines().count() - 1;
    let last = head.lines().last().unwrap();
    let expect = format!(
        "[lines 1 to {shown} of 1000; continue with offset {}; first grep it, then read the lines you need with offset and limit]",
        shown + 1
    );
    assert_eq!(last, expect);
    // A range keeps the whole budget and the short note.
    let ranged = read_with(&env.cwd, json!({"path": "f.rs", "offset": 1}), &context);
    assert!((6000..=8192).contains(&ranged.len()), "{}", ranged.len());
    assert!(!ranged.contains("first grep it"));
    // A small file comes whole, with no note.
    std::fs::write(env.cwd.join("s.rs"), code(20)).unwrap();
    let small = read_with(&env.cwd, json!({"path": "s.rs"}), &context);
    assert_eq!(small.lines().count(), 20);
    // `readHeadBytes: 0` leaves a plain read at the result budget.
    let off = ContextConfig {
        read_head_bytes: 0,
        ..ContextConfig::default()
    };
    let plain = read_with(&env.cwd, json!({"path": "f.rs"}), &off);
    assert!(plain.len() > 6000 && !plain.contains("first grep it"));
}

#[test]
fn lines_inside_an_earlier_unchanged_read_answer_with_that_call() {
    let env = env("ctx-dedupe");
    let file = env.cwd.join("f.rs");
    std::fs::write(&file, code(1000)).unwrap();
    let cancel = AtomicBool::new(false);
    let session = ToolSession::new(None);
    let tools = builtin();
    let read = |id: &str, args: Value| {
        let ctx = ToolCtx::new(&env.cwd, &cancel)
            .with_call_id(id)
            .with_session(&session);
        tools.run("read", &args.to_string(), &ctx).into_text()
    };
    let head = read("c1", json!({"path": "f.rs"}));
    let shown = (head.lines().count() - 1) as u64;
    let unchanged = |call: &str| format!("[unchanged since call {call}]");
    assert_eq!(read("c2", json!({"path": "f.rs"})), unchanged("c1"));
    let inside = json!({"path": "f.rs", "offset": 5, "limit": shown - 4});
    assert_eq!(read("c3", inside), unchanged("c1"));
    // Past the lines c1 returned (its result was cut) is a new read.
    let past = read("c4", json!({"path": "f.rs", "offset": 5, "limit": shown}));
    assert!(past.starts_with("5: "), "{}", &past[..20]);
    let later = read("c5", json!({"path": "f.rs", "offset": 400, "limit": 10}));
    assert!(later.starts_with("400: "));
    assert_eq!(
        read("c6", json!({"path": "f.rs", "offset": 402, "limit": 3})),
        unchanged("c5")
    );
    // A changed file is read again.
    std::fs::write(&file, code(999)).unwrap();
    assert!(read("c7", json!({"path": "f.rs", "offset": 402, "limit": 3})).starts_with("402: "));
    // `dedupeReads: false` always reads.
    let off = ContextConfig {
        dedupe_reads: false,
        ..ContextConfig::default()
    };
    let ctx = ToolCtx::new(&env.cwd, &cancel)
        .with_call_id("c8")
        .with_session(&session)
        .with_context(&off);
    let args = json!({"path": "f.rs", "offset": 402, "limit": 3}).to_string();
    assert!(tools
        .run("read", &args, &ctx)
        .into_text()
        .starts_with("402: "));
}

/// The 48-step script: whole-file reads of 20 files of 600 lines, a ranged
/// read every third step, and a re-read of an earlier range every eighth.
fn read_heavy_script() -> Vec<ScriptedResponse> {
    let mut script: Vec<ScriptedResponse> = (1..=48)
        .map(|step| {
            let id = format!("toolu_{step}");
            let file = format!("src/f{:02}.rs", step % 20);
            let args = match (step % 8 == 0, step % 3 == 0) {
                (true, _) => json!({"path": "src/f01.rs", "offset": 10, "limit": 20}),
                (false, true) => json!({"path": file, "offset": 200, "limit": 60}),
                _ => json!({"path": file}),
            };
            tool_reply(&[(&id, "read", &args.to_string())])
        })
        .collect();
    script.push(text_reply("done"));
    script
}

/// Runs the 48-step turn with `context` and returns the request bodies.
fn read_heavy_turn(name: &str, context: ContextConfig) -> (Vec<String>, Env) {
    let env = env(name);
    std::fs::create_dir_all(env.cwd.join("src")).unwrap();
    for n in 0..20 {
        std::fs::write(env.cwd.join(format!("src/f{n:02}.rs")), code(600)).unwrap();
    }
    std::fs::write(env.cwd.join("src/f01.rs"), code(600)).unwrap();
    let transport = Arc::new(ScriptedTransport::new(read_heavy_script()));
    let mut parts = parts(&transport, key_auth(), &env.cwd);
    parts.config = Config {
        context,
        ..Config::default()
    };
    let mut agent = Agent::new(parts, Arc::new(builtin()));
    agent.preload = vec!["read".into()];
    let mut conv = Conversation::create(&env.home, &env.cwd, None).unwrap();
    agent.run_turn(&mut conv, "go", &AtomicBool::new(false), &mut |_| {});
    let bodies: Vec<String> = transport
        .requests()
        .iter()
        .map(|r| r.body.clone())
        .collect();
    assert_eq!(bodies.len(), 49);
    (bodies, env)
}

/// `messages` of a body without the moving `cache_control` marks and the
/// trailing effort entry (a `system` message that always comes last).
fn messages(body: &str) -> Vec<Value> {
    let mut value: Value = serde_json::from_str(body).unwrap();
    let mut list = value["messages"].take();
    strip_cache(&mut list);
    let mut list = list.as_array().cloned().unwrap_or_default();
    if list.last().is_some_and(|m| m["role"] == "system") {
        list.pop();
    }
    list
}

fn strip_cache(value: &mut Value) {
    match value {
        Value::Object(map) => {
            map.remove("cache_control");
            map.values_mut().for_each(strip_cache);
        }
        Value::Array(items) => items.iter_mut().for_each(strip_cache),
        _ => {}
    }
}

/// Per step: (body bytes, estimated cache-read tokens, cache-write tokens)
/// at 3.2 bytes per token; a step reads the previous body from the cache
/// and writes what it adds.
fn cache_estimate(bodies: &[String], step: usize) -> (usize, usize, usize) {
    let tokens = |bytes: usize| bytes * 10 / 32;
    let read: usize = (1..step).map(|i| tokens(bodies[i - 1].len())).sum();
    let write: usize = tokens(bodies[step - 1].len());
    (bodies[step - 1].len(), read, write)
}

#[test]
fn a_read_heavy_turn_sends_less_and_every_request_extends_the_last() {
    let before = ContextConfig {
        read_head_bytes: 0,
        dedupe_reads: false,
    };
    let (old, _env1) = read_heavy_turn("ctx-before", before);
    let (new, _env2) = read_heavy_turn("ctx-after", ContextConfig::default());
    for step in [10, 24, 48] {
        let (b, br, bw) = cache_estimate(&old, step);
        let (a, ar, aw) = cache_estimate(&new, step);
        eprintln!(
            "step {step}: request bytes {b} -> {a}; est. tokens {} -> {}; \
             cumulative cache read {br} -> {ar}, cache write {bw} -> {aw}",
            b * 10 / 32,
            a * 10 / 32
        );
    }
    assert!(
        new[47].len() * 10 <= old[47].len() * 7,
        "{} vs {}",
        new[47].len(),
        old[47].len()
    );
    for bodies in [&old, &new] {
        for pair in bodies.windows(2) {
            let (earlier, later) = (messages(&pair[0]), messages(&pair[1]));
            assert_eq!(
                earlier[..],
                later[..earlier.len()],
                "a sent message changed"
            );
        }
    }
}
