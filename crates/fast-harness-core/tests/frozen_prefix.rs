//! Frozen prefix (PLAN.md D36): a resumed session sends the system prompt
//! and tools of its log, a prompt file edit reaches it once as a context
//! update, and `cacheTtl` picks the breakpoint lifetimes.

mod agent_support;

use std::path::Path;
use std::sync::atomic::AtomicBool;
use std::sync::Arc;

use agent_support::{echo_registry, env, events, key_auth, parts, text_reply, Echo, Env};
use fast_harness_core::agent::{Agent, Context, Conversation};
use fast_harness_core::config::{CacheTtl, Config};
use fast_harness_core::events::TurnOutcome;
use fast_harness_core::http::ScriptedTransport;
use fast_harness_core::llm::antigravity::provider::session_id_of;
use fast_harness_core::llm::ToolSpec;
use fast_harness_core::tools::{Registry, Tool, ToolCtx, ToolOutput};
use fast_harness_core::trace::kind;
use serde_json::{json, Value};

const MODEL: &str = "claude-sonnet-5-5";

/// A second tool, added after the session froze its tool list.
struct Later;

impl Tool for Later {
    fn spec(&self) -> ToolSpec {
        ToolSpec {
            name: "later".into(),
            description: "Added later".into(),
            schema: json!({"type": "object"}),
        }
    }

    fn run(&self, _args: &Value, _ctx: &ToolCtx<'_>) -> ToolOutput {
        ToolOutput::ok("later")
    }
}

/// One turn the way `Runtime::agent` builds it: the sources read now, the
/// session's own system prompt when it has one.
fn turn(
    env: &Env,
    transport: &Arc<ScriptedTransport>,
    conv: &mut Conversation,
    registry: Registry,
) {
    let (context, _) = Context::read_at(&env.home, &env.cwd, MODEL, "2026-11-12");
    let mut parts = parts(transport, key_auth(), &env.cwd);
    parts.config.model = MODEL.into();
    parts.system = conv.system.clone().unwrap_or_else(|| context.system());
    let agent = Agent::new(parts, Arc::new(registry)).with_context(context);
    let outcome = agent.run_turn(conv, "go", &AtomicBool::new(false), &mut |_| {});
    assert_eq!(outcome, TurnOutcome::Done);
}

fn bodies(transport: &ScriptedTransport) -> Vec<Value> {
    let requests = transport.requests();
    requests
        .iter()
        .map(|r| serde_json::from_str(&r.body).unwrap())
        .collect()
}

fn last_user_text(body: &Value) -> String {
    let messages = body["messages"].as_array().unwrap();
    let user = messages.iter().rev().find(|m| m["role"] == "user").unwrap();
    user["content"].to_string()
}

fn logged_header(log: &Path) -> Value {
    let all = events(log);
    let header = all.iter().rev().find(|e| e.kind == kind::REQUEST_HEADER);
    header.unwrap().data.clone()
}

fn registry(later: bool) -> Registry {
    let mut registry = Registry::new();
    registry.add(Box::new(Echo));
    if later {
        registry.add(Box::new(Later));
    }
    registry
}

#[test]
fn resume_after_a_prompt_edit_sends_the_logged_system_once_noted() {
    let env = env("frozen-resume");
    let file = env.home.join("system-prompt.md");
    std::fs::write(&file, "Prompt A for {model}.\n").unwrap();
    let replies = (0..3).map(|n| text_reply(&format!("answer {n}"))).collect();
    let transport = Arc::new(ScriptedTransport::new(replies));
    let mut conv = Conversation::create(&env.home, &env.cwd, None).unwrap();
    turn(&env, &transport, &mut conv, registry(false));
    let (id, log) = (conv.id.clone(), conv.log_path.clone());
    let logged = logged_header(&log)["system"].as_str().unwrap().to_string();
    assert!(logged.starts_with("Prompt A for claude-sonnet-5-5."));
    drop(conv);
    std::fs::write(&file, "Prompt B.\n").unwrap();
    let mut conv = Conversation::resume(&env.home, &env.cwd, &id).unwrap();
    assert_eq!(conv.system.as_deref(), Some(logged.as_str()));
    turn(&env, &transport, &mut conv, registry(true));
    drop(conv);
    let mut conv = Conversation::resume(&env.home, &env.cwd, &id).unwrap();
    turn(&env, &transport, &mut conv, registry(true));
    let sent = bodies(&transport);
    for body in &sent {
        // Golden: the logged system text, byte for byte, and the frozen tools.
        assert_eq!(
            body["system"],
            json!([{"type": "text", "text": logged,
            "cache_control": {"type": "ephemeral", "ttl": "1h"}}])
        );
        let names: Vec<&str> = body["tools"]
            .as_array()
            .unwrap()
            .iter()
            .map(|t| t["name"].as_str().unwrap())
            .collect();
        assert_eq!(
            names,
            ["echo"],
            "a tool added later belongs to new sessions"
        );
    }
    let second = last_user_text(&sent[1]);
    assert!(second.contains("[context update] The system prompt changed (global prompt"));
    assert!(second.contains("Prompt B."));
    assert!(!second.contains("Today is"), "the date did not change");
    assert!(
        !last_user_text(&sent[2]).contains("[context update]"),
        "never repeated"
    );
    assert!(logged_header(&log)["context"]["date"] == "2026-11-12");
}

#[test]
fn context_update_names_added_changed_and_removed_files() {
    let env = env("frozen-agents");
    std::fs::create_dir_all(env.cwd.join(".git")).unwrap();
    let agents = env.cwd.join("AGENTS.md");
    let (before, _) = Context::read_at(&env.home, &env.cwd, MODEL, "2026-11-12");
    assert_eq!(before.update(&before.stamp()), None);
    std::fs::write(&agents, "Rule 1.\n").unwrap();
    let (added, _) = Context::read_at(&env.home, &env.cwd, MODEL, "2026-11-13");
    let note = added.update(&before.stamp()).unwrap();
    let path = agents.display().to_string();
    assert_eq!(
        note,
        format!("[context update] Today is 2026-11-13.\n\n[context update] {path} was added; it now reads:\nRule 1.")
    );
    std::fs::remove_file(&agents).unwrap();
    let (removed, _) = Context::read_at(&env.home, &env.cwd, MODEL, "2026-11-13");
    let note = removed.update(&added.stamp()).unwrap();
    assert_eq!(
        note,
        format!("[context update] {path} was removed; its instructions no longer apply.")
    );
}

#[test]
fn workspace_override_toggle_is_a_prompt_change() {
    let env = env("frozen-toggle");
    let (global, _) = Context::read_at(&env.home, &env.cwd, MODEL, "2026-11-12");
    let slug = fast_harness_core::session::slug(&env.cwd);
    let dir = env.home.join("prompts");
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(dir.join(format!("{slug}.md")), "Workspace prompt.\n").unwrap();
    let (own, _) = Context::read_at(&env.home, &env.cwd, MODEL, "2026-11-12");
    let note = own.update(&global.stamp()).unwrap();
    assert!(note.starts_with("[context update] The system prompt changed (workspace prompt"));
    assert!(note.ends_with("Workspace prompt."));
}

fn config(file: &str) -> Config {
    let env = env("frozen-config");
    std::fs::write(env.home.join("config.json"), file).unwrap();
    Config::load_with(&env.home, &|_| None).unwrap()
}

#[test]
fn cache_ttl_setting() {
    let auto = config("{}");
    assert_eq!(
        (auto.cache_setting(), auto.cache_hours()),
        (CacheTtl::Auto, (true, false))
    );
    let interactive = Config {
        interactive: true,
        ..auto
    };
    assert_eq!(interactive.cache_hours(), (true, true));
    assert_eq!(config(r#"{"cacheTtl":"5m"}"#).cache_hours(), (false, false));
    assert_eq!(config(r#"{"cacheTtl":"1h"}"#).cache_hours(), (true, true));
    assert_eq!(
        config(r#"{"cacheLong":true}"#).cache_setting(),
        CacheTtl::Long
    );
    let both = config(r#"{"cacheLong":true,"cacheTtl":"5m"}"#);
    assert_eq!(both.cache_setting(), CacheTtl::Short, "cacheTtl wins");
    assert_eq!(
        config(r#"{"cacheTtl":"2d"}"#).cache_setting(),
        CacheTtl::Auto
    );
}

#[test]
fn antigravity_session_id_follows_the_conversation() {
    let (a, b) = (session_id_of("20261112-1"), session_id_of("20261112-2"));
    assert_ne!(a, b);
    assert_eq!(a, session_id_of("20261112-1"));
    assert!(a.starts_with('-') && a[1..].parse::<u64>().is_ok());
}

#[test]
fn new_session_freezes_on_its_first_turn() {
    let env = env("frozen-new");
    let transport = Arc::new(ScriptedTransport::new(vec![text_reply("ok")]));
    let mut conv = Conversation::create(&env.home, &env.cwd, None).unwrap();
    assert!(conv.system.is_none() && conv.tools.is_none());
    let agent = Agent::new(parts(&transport, key_auth(), &env.cwd), echo_registry());
    agent.run_turn(&mut conv, "hi", &AtomicBool::new(false), &mut |_| {});
    assert_eq!(conv.system.as_deref(), Some("test system"));
    assert_eq!(conv.tools.as_ref().map(Vec::len), Some(1));
    assert!(logged_header(&conv.log_path).get("context").is_none());
}
