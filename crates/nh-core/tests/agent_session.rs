//! Resume repair, the system prompt, the subagent tool and config loading.

mod agent_support;

use std::sync::atomic::AtomicBool;
use std::sync::Arc;

use agent_support::*;
use nh_core::agent::{prompt, Agent, Conversation, MISSING_RESULT};
use nh_core::config::{save_setting, Config};
use nh_core::events::TurnOutcome;
use nh_core::http::ScriptedTransport;
use nh_core::llm::Timing;
use nh_core::session;
use nh_core::trace::{self, SessionHeader, TraceWriter};
use nh_core::types::{Block, Message, Role, StopReason, Usage};

fn call(id: &str) -> Block {
    Block::ToolCall {
        id: id.into(),
        name: "echo".into(),
        arguments: "{}".into(),
    }
}

fn result_message(id: &str) -> Message {
    Message {
        role: Role::Tool,
        content: vec![Block::ToolResult {
            call_id: id.into(),
            content: format!("result {id}"),
            is_error: false,
        }],
    }
}

/// A log whose turn crashed after two of three tool results.
fn write_crashed_log(env: &Env, id: &str) {
    let path = session::log_path(&env.home, &env.cwd, id);
    let header = SessionHeader::new(id, &env.cwd.to_string_lossy(), None);
    let mut writer = TraceWriter::create(&path, &header).unwrap();
    writer.turn_start(1).unwrap();
    writer.user_message(1, &Message::user("go")).unwrap();
    let assistant = Message {
        role: Role::Assistant,
        content: vec![call("toolu_1"), call("toolu_2"), call("toolu_3")],
    };
    let usage = Usage::default();
    let timing = Timing::default();
    writer
        .assistant_message(
            1,
            1,
            &assistant,
            &usage,
            StopReason::ToolUse,
            &timing,
            "acct-1",
        )
        .unwrap();
    writer
        .tool_result(1, 1, &result_message("toolu_1"), 3)
        .unwrap();
    writer
        .tool_result(1, 1, &result_message("toolu_2"), 4)
        .unwrap();
}

#[test]
fn resume_merges_results_and_repairs_a_dangling_call() {
    let env = env("resume");
    let id = "20260101-000000-0001";
    write_crashed_log(&env, id);
    let mut conv = Conversation::resume(&env.home, &env.cwd, id).unwrap();
    assert_eq!(conv.turn, 1);
    let roles: Vec<Role> = conv.messages.iter().map(|m| m.role).collect();
    assert_eq!(roles, [Role::User, Role::Assistant, Role::Tool]);
    let results = &conv.messages[2].content;
    assert_eq!(results.len(), 3);
    let expected = Block::ToolResult {
        call_id: "toolu_3".into(),
        content: MISSING_RESULT.into(),
        is_error: true,
    };
    assert_eq!(results[2], expected);
    let transport = Arc::new(ScriptedTransport::new(vec![text_reply("resumed")]));
    let agent = Agent::new(parts(&transport, key_auth(), &env.cwd), echo_registry());
    let outcome = agent.run_turn(&mut conv, "next", &AtomicBool::new(false), &mut |_| {});
    assert_eq!(outcome, TurnOutcome::Done);
    assert_eq!(conv.turn, 2);
    assert!(transport.requests()[0].body.contains(MISSING_RESULT));
    let events = events(&conv.log_path);
    let seqs: Vec<u64> = events.iter().map(|e| e.seq).collect();
    assert_eq!(seqs, (0..events.len() as u64).collect::<Vec<_>>());
    assert_eq!(first(&events, "turn/end").data["turn"], 2);
    drop(env);
}

#[test]
fn prompt_includes_agents_files_root_first() {
    let root = TempDir::new("prompt");
    let sub = root.0.join("sub");
    std::fs::create_dir_all(&sub).unwrap();
    std::fs::write(root.0.join("AGENTS.md"), "ROOT RULES").unwrap();
    std::fs::write(sub.join("AGENTS.md"), "SUB RULES").unwrap();
    let text = prompt::system_prompt(&sub);
    assert!(text.starts_with("You are nh, a coding agent working in"));
    assert!(text.contains(&sub.display().to_string()));
    let root_at = text.find("ROOT RULES").unwrap();
    let sub_at = text.find("SUB RULES").unwrap();
    assert!(root_at < sub_at);
    assert!(text.contains(&format!("## {}", root.0.join("AGENTS.md").display())));
    std::fs::write(sub.join("AGENTS.md"), "x".repeat(40 * 1024)).unwrap();
    let text = prompt::system_prompt(&sub);
    assert!(text.contains("cut at 32 KB"));
    assert!(text.len() < prompt::MAX_AGENTS_BYTES + 4096);
    drop(root);
}

#[test]
fn subagent_writes_a_child_log_naming_the_parent() {
    let env = env("subagent");
    let call = r#"{"prompt":"look around","description":"look"}"#;
    let responses = vec![
        tool_reply(&[("toolu_1", "subagent", call)]),
        text_reply("child done"),
        text_reply("parent done"),
    ];
    let transport = Arc::new(ScriptedTransport::new(responses));
    let mut conv = Conversation::create(&env.home, &env.cwd, None).unwrap();
    let agent = Agent::with_subagent(parts(&transport, key_auth(), &env.cwd), &env.home, &conv.id);
    let outcome = agent.run_turn(&mut conv, "go", &AtomicBool::new(false), &mut |_| {});
    assert_eq!(outcome, TurnOutcome::Done);
    let sessions = session::list(&env.home, &env.cwd).unwrap();
    assert_eq!(sessions.len(), 2);
    let child = sessions.iter().find(|s| s.id != conv.id).unwrap();
    let child_path = session::log_path(&env.home, &env.cwd, &child.id);
    let (header, _) = trace::read(&child_path).unwrap();
    assert_eq!(header.parent.as_deref(), Some(conv.id.as_str()));
    let events = events(&conv.log_path);
    let content = first(&events, "tool/result").data["message"]["content"][0]["content"]
        .as_str()
        .unwrap()
        .to_string();
    assert!(content.starts_with("child done"), "{content}");
    assert!(content.contains(&child.id));
    let requests = transport.requests();
    assert!(requests[0].body.contains(r#""name":"subagent""#));
    assert!(!requests[1].body.contains(r#""name":"subagent""#));
    drop(env);
}

#[test]
fn config_defaults_file_and_env_overrides() {
    let dir = TempDir::new("config");
    let none = |_: &str| None;
    let config = Config::load_with(&dir.0, &none).unwrap();
    assert_eq!(config, Config::default());
    assert_eq!(config.model, "claude-opus-5-5");
    assert_eq!(config.effort.as_deref(), Some("high"));
    assert_eq!(
        (config.max_tokens, config.max_steps, config.retries),
        (32_000, 200, 8)
    );
    assert_eq!((config.max_wait_ms, config.cache_long), (3_600_000, false));
    let file = r#"{"model":"m1","maxSteps":5,"effort":null,"cacheLong":true,"other":1}"#;
    std::fs::write(dir.0.join("config.json"), file).unwrap();
    let config = Config::load_with(&dir.0, &none).unwrap();
    assert_eq!((config.model.as_str(), config.max_steps), ("m1", 5));
    assert_eq!(
        (config.effort, config.cache_long, config.retries),
        (None, true, 8)
    );
    let env = |key: &str| match key {
        "NH_MODEL" => Some("m2".to_string()),
        "NH_EFFORT" => Some("low".to_string()),
        _ => None,
    };
    let config = Config::load_with(&dir.0, &env).unwrap();
    assert_eq!(
        (config.model.as_str(), config.effort.as_deref()),
        ("m2", Some("low"))
    );
    std::fs::write(dir.0.join("config.json"), "{oops").unwrap();
    let err = Config::load_with(&dir.0, &none).unwrap_err();
    assert!(err.contains("config.json"), "{err}");
    drop(dir);
}

#[test]
fn config_theme_wire_and_save_setting() {
    let dir = TempDir::new("config-save");
    let none = |_: &str| None;
    let config = Config::load_with(&dir.0, &none).unwrap();
    assert_eq!((config.theme.as_str(), config.wire), ("dark", false));
    let env = |key: &str| match key {
        "NH_THEME" => Some(" light ".to_string()),
        "NH_WIRE" => Some("ON".to_string()),
        _ => None,
    };
    let config = Config::load_with(&dir.0, &env).unwrap();
    assert_eq!((config.theme.as_str(), config.wire), ("light", true));
    let odd = |key: &str| (key == "NH_WIRE").then(|| "maybe".to_string());
    assert!(!Config::load_with(&dir.0, &odd).unwrap().wire);
    // The first save creates the folder and the file; later saves keep the other keys.
    let home = dir.0.join("home");
    save_setting(&home, "theme", serde_json::json!("greyscale-dark")).unwrap();
    assert_eq!(
        Config::load_with(&home, &none).unwrap().theme,
        "greyscale-dark"
    );
    let file = home.join("config.json");
    std::fs::write(&file, r#"{"model":"m1","wire":true,"other":[1]}"#).unwrap();
    save_setting(&home, "theme", serde_json::json!("greyscale-light")).unwrap();
    let config = Config::load_with(&home, &none).unwrap();
    assert_eq!(
        (config.model.as_str(), config.theme.as_str(), config.wire),
        ("m1", "greyscale-light", true)
    );
    let raw: serde_json::Value = serde_json::from_slice(&std::fs::read(&file).unwrap()).unwrap();
    assert_eq!(raw["other"], serde_json::json!([1]));
    assert!(!home.join("config.json.tmp").exists());
    std::fs::write(&file, "[1]").unwrap();
    let err = save_setting(&home, "theme", serde_json::json!("dark")).unwrap_err();
    assert!(err.contains("not a JSON object"), "{err}");
    assert_eq!(std::fs::read_to_string(&file).unwrap(), "[1]");
    drop(dir);
}
