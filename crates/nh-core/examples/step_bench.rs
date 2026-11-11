//! Harness time per step without a network: one turn of N tool steps against
//! scripted Claude streams (real SSE parsing, the trace, the `read` tool on a
//! small file), then a text answer. `--wire` turns raw wire capture on (it is
//! off by default, as in the harness).
//!
//! `cargo run --release -p nh-core --example step_bench -- [steps] [--wire]`

use std::path::{Path, PathBuf};
use std::sync::atomic::AtomicBool;
use std::sync::Arc;
use std::time::Instant;

use nh_core::agent::{Agent, AgentParts, Conversation};
use nh_core::auth::StaticAuth;
use nh_core::config::Config;
use nh_core::http::{ScriptedResponse, ScriptedTransport};
use nh_core::llm::anthropic::Anthropic;
use nh_core::llm::Auth;
use nh_core::tools;
use serde_json::{json, Value};

const CHUNK: usize = 512;

fn sse(events: &[Value]) -> String {
    let mut out = String::new();
    for event in events {
        let name = event["type"].as_str().unwrap_or("message");
        out.push_str(&format!("event: {name}\ndata: {event}\n\n"));
    }
    out
}

fn reply(blocks: Vec<Value>, stop: &str) -> ScriptedResponse {
    let mut events = vec![
        json!({"type": "message_start", "message": {"id": "msg", "type": "message",
        "role": "assistant", "content": [], "model": "claude-opus-5-5",
        "usage": {"input_tokens": 10, "output_tokens": 1}}}),
    ];
    events.extend(blocks);
    events.push(
        json!({"type": "message_delta", "delta": {"stop_reason": stop},
        "usage": {"output_tokens": 20}}),
    );
    events.push(json!({"type": "message_stop"}));
    ScriptedResponse::new(200, &sse(&events), CHUNK)
}

fn tool_step(index: usize) -> ScriptedResponse {
    let args = r#"{"path": "notes.txt"}"#;
    reply(
        vec![
            json!({"type": "content_block_start", "index": 0,
                "content_block": {"type": "tool_use", "id": format!("toolu_{index}"), "name": "read", "input": {}}}),
            json!({"type": "content_block_delta", "index": 0,
                "delta": {"type": "input_json_delta", "partial_json": args}}),
            json!({"type": "content_block_stop", "index": 0}),
        ],
        "tool_use",
    )
}

fn text_step() -> ScriptedResponse {
    reply(
        vec![
            json!({"type": "content_block_start", "index": 0, "content_block": {"type": "text", "text": ""}}),
            json!({"type": "content_block_delta", "index": 0, "delta": {"type": "text_delta", "text": "done"}}),
            json!({"type": "content_block_stop", "index": 0}),
        ],
        "end_turn",
    )
}

fn dir_bytes(path: &Path) -> u64 {
    let Ok(entries) = std::fs::read_dir(path) else {
        return 0;
    };
    entries
        .flatten()
        .map(|entry| match entry.metadata() {
            Ok(meta) if meta.is_dir() => dir_bytes(&entry.path()),
            Ok(meta) => meta.len(),
            Err(_) => 0,
        })
        .sum()
}

fn temp_root() -> PathBuf {
    let root = std::env::temp_dir().join(format!("nh-step-bench-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&root);
    root
}

/// `(steps, --wire)` from the command line.
fn options() -> (usize, bool) {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let wire = args.iter().any(|arg| arg == "--wire");
    let steps = args.iter().find_map(|arg| arg.parse().ok()).unwrap_or(200);
    (steps, wire)
}

fn main() {
    let (steps, wire) = options();
    let root = temp_root();
    let (home, cwd) = (root.join("home"), root.join("work"));
    std::fs::create_dir_all(&home).expect("home");
    std::fs::create_dir_all(&cwd).expect("cwd");
    std::fs::write(cwd.join("notes.txt"), "line one\nline two\nline three\n").expect("file");
    let mut responses: Vec<ScriptedResponse> = (0..steps).map(tool_step).collect();
    responses.push(text_step());
    let config = Config {
        max_steps: steps as u32 + 10,
        wire,
        ..Config::default()
    };
    let parts = AgentParts {
        provider: Arc::new(Anthropic::default()),
        transport: Arc::new(ScriptedTransport::new(responses)),
        auth: Arc::new(StaticAuth::new(
            "bench",
            "bench",
            Auth::ApiKey("bench".into()),
        )),
        config,
        cwd: cwd.clone(),
        system: "You are a benchmark.".into(),
    };
    let agent = Agent::new(parts, Arc::new(tools::builtin()));
    let mut conv = Conversation::create(&home, &cwd, None).expect("session");
    let cancel = AtomicBool::new(false);
    let mut events = 0usize;
    let started = Instant::now();
    let outcome = agent.run_turn(&mut conv, "bench", &cancel, &mut |_| events += 1);
    let elapsed = started.elapsed();
    let log = std::fs::metadata(&conv.log_path)
        .map(|m| m.len())
        .unwrap_or(0);
    let total_steps = steps + 1;
    println!("outcome {outcome:?}, steps {total_steps}, events {events}");
    println!(
        "total {:.1} ms, per step {:.0} us",
        elapsed.as_secs_f64() * 1000.0,
        elapsed.as_secs_f64() * 1e6 / total_steps as f64
    );
    println!(
        "session log {log} bytes, wire capture {} bytes",
        dir_bytes(&conv.wire_dir)
    );
    let _ = std::fs::remove_dir_all(&root);
}
