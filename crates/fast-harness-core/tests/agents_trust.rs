//! D62: an untrusted project supplies nothing. Its `AGENTS.md` files stay
//! out of the system prompt and nested ones are never attached; the
//! untrusted note names them; after `/trust` a new session gets them.

mod agent_support;

use std::fs;
use std::path::Path;
use std::sync::atomic::AtomicBool;
use std::sync::Arc;

use agent_support::*;
use fast_harness_core::agent::{Agent, Context};
use fast_harness_core::app::Runtime;
use fast_harness_core::config::layers::{self, Layers};
use fast_harness_core::config::trust;
use fast_harness_core::events::TurnOutcome;
use fast_harness_core::http::{ScriptedTransport, Transport};

const ROOT_RULE: &str = "ROOT-RULE-7f3a";
const NESTED_RULE: &str = "NESTED-RULE-9c2e";

fn put(path: &Path, text: &str) {
    fs::create_dir_all(path.parent().unwrap()).unwrap();
    fs::write(path, text).unwrap();
}

/// A repository in `env.cwd` with a root and a nested AGENTS.md, and a
/// local provider with `read` preloaded.
fn project(env: &Env) {
    fs::create_dir_all(env.cwd.join(".git")).unwrap();
    put(&env.cwd.join("AGENTS.md"), ROOT_RULE);
    put(&env.cwd.join("sub").join("AGENTS.md"), NESTED_RULE);
    put(&env.cwd.join("sub").join("a.txt"), "alpha");
    let config = r#"{"model": "local/m", "tools": {"preload": ["read"]}}"#;
    put(&env.home.join("config.json"), config);
    let provider = r#"{"id": "local", "api": "anthropic-messages", "baseUrl": "http://127.0.0.1:9", "models": [{"id": "m"}]}"#;
    put(&env.home.join("providers").join("local.json"), provider);
}

/// One turn that reads `sub/a.txt`; returns the request bodies and notes.
fn turn(rt: &Runtime, env: &Env) -> (Vec<String>, Vec<String>) {
    let mut conv = rt.open_conversation(&env.cwd, None, false).unwrap();
    let (mut agent, notes): (Agent, Vec<String>) = rt.agent(&env.cwd, &conv).unwrap();
    let responses = vec![
        tool_reply(&[("c1", "read", r#"{"path":"sub/a.txt"}"#)]),
        text_reply("done"),
    ];
    let transport = Arc::new(ScriptedTransport::new(responses));
    let scripted: Arc<dyn Transport> = transport.clone();
    agent.parts.transport = scripted;
    let outcome = agent.run_turn(&mut conv, "read it", &AtomicBool::new(false), &mut |_| {});
    assert_eq!(outcome, TurnOutcome::Done);
    let bodies = transport
        .requests()
        .iter()
        .map(|r| r.body.clone())
        .collect();
    (bodies, notes)
}

#[test]
fn an_untrusted_projects_agents_md_is_not_in_the_system_prompt() {
    let env = env("agents-untrusted-prompt");
    project(&env);
    let (context, _) = Context::read_at(&env.home, &env.cwd, "m", "2026-11-13");
    assert!(!context.system().contains(ROOT_RULE));
    assert!(context.agents.is_empty());
    let layers = Layers::find(&env.home, &env.cwd);
    assert_eq!(layers.agents_skipped, vec![env.cwd.join("AGENTS.md")]);
    let note = layers.untrusted_note().unwrap();
    let path = env.cwd.join("AGENTS.md").display().to_string();
    assert!(note.contains("AGENTS.md files were left out") && note.contains(&path));
    assert!(
        note.contains("/trust") && !note.contains(ROOT_RULE),
        "{note}"
    );
    // What `/trust` trusts (a temp home does not hide `~/.fast-harness`,
    // which then counts as a project folder above the temp folder).
    trust::trust(&env.home, &layers::trust_root(&env.home, &env.cwd)).unwrap();
    let (context, _) = Context::read_at(&env.home, &env.cwd, "m", "2026-11-13");
    assert!(context.system().contains(ROOT_RULE));
    let note = Layers::find(&env.home, &env.cwd).untrusted_note();
    assert!(note.is_none(), "{note:?}");
}

#[test]
fn an_untrusted_session_sends_no_agents_text_and_trust_brings_it_back() {
    let env = env("agents-untrusted-wire");
    project(&env);
    let mut rt = Runtime::load_at(env.home.clone()).unwrap();
    rt.use_project(&env.cwd).unwrap();
    let (bodies, notes) = turn(&rt, &env);
    assert_eq!(bodies.len(), 2);
    for body in &bodies {
        assert!(!body.contains(ROOT_RULE), "root AGENTS.md was sent");
        assert!(!body.contains(NESTED_RULE), "nested AGENTS.md was attached");
    }
    assert!(bodies[1].contains("alpha"), "the read itself ran");
    let note = notes.iter().find(|n| n.contains("not trusted")).unwrap();
    assert!(note.contains(&env.cwd.join("AGENTS.md").display().to_string()));
    rt.trust_project(&env.cwd).unwrap();
    let (bodies, _) = turn(&rt, &env);
    assert!(
        bodies[0].contains(ROOT_RULE),
        "trusted: in the system prompt"
    );
    assert!(bodies[1].contains(NESTED_RULE), "trusted: nested attached");
}

#[test]
fn a_folder_above_the_project_folder_follows_its_own_trust() {
    let env = env("agents-above");
    fs::create_dir_all(env.cwd.join(".git")).unwrap();
    put(&env.cwd.join("AGENTS.md"), ROOT_RULE);
    let inner = env.cwd.join("inner");
    fs::create_dir_all(inner.join(".fast-harness")).unwrap();
    put(&inner.join("AGENTS.md"), NESTED_RULE);
    trust::trust(&env.home, &inner).unwrap();
    let (context, _) = Context::read_at(&env.home, &inner, "m", "2026-11-13");
    let system = context.system();
    assert!(system.contains(NESTED_RULE) && !system.contains(ROOT_RULE));
    let layers = Layers::find(&env.home, &inner);
    assert_eq!(layers.agents_skipped, vec![env.cwd.join("AGENTS.md")]);
}
