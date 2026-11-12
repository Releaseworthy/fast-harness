//! Live Antigravity check over the real network (ignored by default):
//!
//! ```text
//! $env:FAST_HARNESS_LIVE_GOOGLE_REFRESH='<refresh token>'
//! $env:FAST_HARNESS_GOOGLE_CLIENT_ID='<client id>'
//! $env:FAST_HARNESS_GOOGLE_CLIENT_SECRET='<client secret>'
//! cargo test -p fast-harness-core --test antigravity_live -- --ignored --nocapture
//! ```
//!
//! Refresh, `loadCodeAssist`, `fetchAvailableModels`, then for the first Gemini
//! flash model and the first Claude model: one text turn and one tool round
//! trip through the real Agent. Prints model ids, stop reasons, token counts
//! and the first 80 characters of each answer; never a token, project or email.

use std::path::Path;
use std::sync::atomic::AtomicBool;
use std::sync::Arc;

use fast_harness_core::accounts::google::{client_from, GoogleApi};
use fast_harness_core::agent::{Agent, AgentParts, Conversation};
use fast_harness_core::auth::StaticAuth;
use fast_harness_core::config::Config;
use fast_harness_core::events::AgentEvent;
use fast_harness_core::llm::antigravity::{models, Antigravity};
use fast_harness_core::llm::Auth;
use fast_harness_core::net::UreqTransport;
use fast_harness_core::tools;

const CODE: &str = "violet-7319";

fn env(key: &str) -> Option<String> {
    std::env::var(key).ok().filter(|v| !v.trim().is_empty())
}

/// What one turn produced.
#[derive(Default)]
struct Turn {
    answer: String,
    stops: Vec<String>,
    tool_results: Vec<String>,
    usage: String,
}

fn run(agent: &Agent, conv: &mut Conversation, text: &str) -> Turn {
    let mut turn = Turn::default();
    let cancel = AtomicBool::new(false);
    let outcome = agent.run_turn(conv, text, &cancel, &mut |event| match event {
        AgentEvent::TextDelta(delta) => turn.answer.push_str(&delta),
        AgentEvent::StepEnd { stop, .. } => turn.stops.push(format!("{stop:?}")),
        AgentEvent::ToolResult { content, .. } => turn.tool_results.push(content),
        AgentEvent::TurnEnd { usage, .. } => turn.usage = format!("{usage:?}"),
        AgentEvent::Error(err) => println!("  error event: {}", clip(&err, 200)),
        _ => {}
    });
    println!(
        "  outcome {outcome:?}, stops {:?}, usage {}, answer {:?}",
        turn.stops,
        turn.usage,
        clip(&turn.answer, 80)
    );
    turn
}

fn clip(text: &str, max: usize) -> String {
    text.chars().take(max).collect()
}

fn check_model(model: &str, auth: Auth, dir: &Path) {
    println!("model {model}");
    let config = Config {
        model: model.to_string(),
        ..Config::default()
    };
    let parts = AgentParts {
        extensions: Default::default(),
        provider: Arc::new(Antigravity::default()),
        transport: Arc::new(UreqTransport::new()),
        auth: Arc::new(StaticAuth::new("live", "live Google account", auth)),
        config,
        cwd: dir.join("work"),
        system: "You are a coding agent. Use the tools when asked.".into(),
    };
    let agent = Agent::new(parts, Arc::new(tools::builtin()));
    let mut conv = Conversation::create(&dir.join("home"), &dir.join("work"), None).unwrap();
    let text = run(&agent, &mut conv, "Reply with the single word: pong");
    assert!(text.answer.to_lowercase().contains("pong"), "text turn");
    let ask = "Use the read tool to read the file note.txt in the working directory, \
               then reply with only the code it contains.";
    let tool = run(&agent, &mut conv, ask);
    assert!(
        tool.tool_results.iter().any(|r| r.contains(CODE)),
        "the read tool ran"
    );
    assert!(
        tool.answer.contains(CODE),
        "the answer uses the tool result"
    );
}

#[test]
#[ignore = "calls Google Antigravity with a real account"]
fn antigravity_live_round_trip() {
    let (Some(refresh), Some(id), Some(secret)) = (
        env("FAST_HARNESS_LIVE_GOOGLE_REFRESH"),
        env("FAST_HARNESS_GOOGLE_CLIENT_ID"),
        env("FAST_HARNESS_GOOGLE_CLIENT_SECRET"),
    ) else {
        println!("skipped: set FAST_HARNESS_LIVE_GOOGLE_REFRESH, FAST_HARNESS_GOOGLE_CLIENT_ID and FAST_HARNESS_GOOGLE_CLIENT_SECRET");
        return;
    };
    let dir = std::env::temp_dir().join(format!("fast-harness-ag-live-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(dir.join("work")).unwrap();
    std::fs::write(dir.join("work").join("note.txt"), format!("code: {CODE}\n")).unwrap();
    let http = UreqTransport::new();
    let api = GoogleApi::default();
    let client = client_from(Some(id), Some(secret), &dir).expect("client");
    let tokens = api.refresh(&http, &client, &refresh).expect("refresh");
    println!("refresh: ok");
    let info = api
        .load_project(&http, &tokens.access, None, true)
        .expect("loadCodeAssist");
    println!(
        "project: {}",
        if info.project_id.is_some() {
            "found"
        } else {
            "fallback"
        }
    );
    let project = info
        .project_id
        .unwrap_or_else(|| "rising-fact-p41fc".into());
    let list = models::list(&http, &api.hub, &tokens.access, &project).expect("models");
    let ids: Vec<&str> = list.iter().map(|m| m.id.as_str()).collect();
    println!("models: {ids:?}");
    let flash = ids
        .iter()
        .find(|i| i.contains("gemini") && i.contains("flash"));
    let claude = ids.iter().find(|i| i.starts_with("antigravity/claude-"));
    let auth = Auth::Google {
        access: tokens.access.clone(),
        project,
    };
    for model in [flash, claude].into_iter().flatten() {
        check_model(model, auth.clone(), &dir);
    }
    assert!(flash.is_some(), "a Gemini flash model is listed");
    let _ = std::fs::remove_dir_all(&dir);
}
