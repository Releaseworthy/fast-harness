//! Resume repair, the system prompt, the subagent tool and config loading.

mod agent_support;

use std::sync::atomic::AtomicBool;
use std::sync::Arc;

use agent_support::*;
use fast_harness_core::agent::{prompt, Agent, Conversation, MISSING_RESULT};
use fast_harness_core::config::{save_setting, Config};
use fast_harness_core::events::TurnOutcome;
use fast_harness_core::http::ScriptedTransport;
use fast_harness_core::llm::Timing;
use fast_harness_core::session;
use fast_harness_core::trace::{self, SessionHeader, TraceWriter};
use fast_harness_core::types::{Block, Message, Role, StopReason, Usage};

fn call(id: &str) -> Block {
    Block::ToolCall {
        id: id.into(),
        name: "echo".into(),
        arguments: "{}".into(),
        signature: None,
    }
}

fn result_message(id: &str) -> Message {
    Message::new(
        Role::Tool,
        vec![Block::tool_result(id, format!("result {id}"), false)],
    )
}

/// A log whose turn crashed after two of three tool results.
fn write_crashed_log(env: &Env, id: &str) {
    let path = session::log_path(&env.home, &env.cwd, id);
    let header = SessionHeader::new(id, &env.cwd.to_string_lossy(), None);
    let mut writer = TraceWriter::create(&path, &header).unwrap();
    writer.turn_start(1).unwrap();
    writer.user_message(1, &Message::user("go")).unwrap();
    let assistant = Message::new(
        Role::Assistant,
        vec![call("toolu_1"), call("toolu_2"), call("toolu_3")],
    );
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
            0,
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

/// D3 (G10): a damaged line that is not the last is skipped with one note;
/// the session opens, the dangling call is repaired, the next turn runs and
/// the log keeps the damaged line.
#[test]
fn resume_skips_a_damaged_middle_line_with_one_note() {
    let env = env("damaged");
    let id = "20260101-000000-0002";
    write_crashed_log(&env, id);
    let path = session::log_path(&env.home, &env.cwd, id);
    let text = std::fs::read_to_string(&path).unwrap();
    let mut lines: Vec<&str> = text.lines().collect();
    // Line 5 is the result of toolu_1: the repair gives it an error result.
    lines[4] = "{\"type\":\"tool/res";
    let damaged = lines.join("\n") + "\n";
    std::fs::write(&path, &damaged).unwrap();
    let offset = damaged.find("{\"type\":\"tool/res\n").unwrap();
    let mut conv = Conversation::resume(&env.home, &env.cwd, id).unwrap();
    let notes = conv.take_notes();
    assert_eq!(notes.len(), 1, "{notes:?}");
    assert!(
        notes[0].contains("line 5") && notes[0].contains(&format!("byte {offset}")),
        "{notes:?}"
    );
    assert!(conv.take_notes().is_empty());
    let results = &conv.messages[2].content;
    assert_eq!(results.len(), 3);
    let errors: Vec<&str> = (results.iter())
        .filter_map(|b| match b {
            Block::ToolResult {
                call_id,
                is_error: true,
                ..
            } => Some(call_id.as_str()),
            _ => None,
        })
        .collect();
    assert_eq!(errors, ["toolu_1", "toolu_3"]);
    let transport = Arc::new(ScriptedTransport::new(vec![text_reply("resumed")]));
    let agent = Agent::new(parts(&transport, key_auth(), &env.cwd), echo_registry());
    let outcome = agent.run_turn(&mut conv, "next", &AtomicBool::new(false), &mut |_| {});
    assert_eq!(outcome, TurnOutcome::Done);
    let after = std::fs::read_to_string(&path).unwrap();
    assert!(after.starts_with(&damaged), "the log is never rewritten");
    let (_, events) = trace::read(&path).unwrap();
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
    // No `.git` anywhere above: only the working directory's own file.
    let text = prompt::system_prompt(&sub);
    assert!(text.contains("SUB RULES") && !text.contains("ROOT RULES"));
    std::fs::create_dir(root.0.join(".git")).unwrap();
    let text = prompt::system_prompt(&sub);
    let start = format!(
        "You are a coding agent in Fast Harness, working in {} on ",
        sub.display()
    );
    assert!(text.starts_with(&start), "{text}");
    assert!(text.contains(
        " (UTC).\nCall find_tools first to load the tools you need; \
        independent calls in one step run in parallel. Find code with grep or glob, then read \
        only the line ranges you need. Be brief.\n"
    ));
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
fn prompt_agents_files_stop_at_the_repository_root() {
    let outer = TempDir::new("prompt-git");
    let repo = outer.0.join("repo");
    let sub = repo.join("sub");
    std::fs::create_dir_all(&sub).unwrap();
    std::fs::create_dir(outer.0.join(".git")).unwrap();
    for (dir, text) in [(&outer.0, "OUTER"), (&repo, "REPO"), (&sub, "SUB")] {
        std::fs::write(dir.join("AGENTS.md"), text).unwrap();
    }
    let names = |cwd: &std::path::Path| -> Vec<String> {
        prompt::agents_files(cwd)
            .into_iter()
            .map(|(_, text)| text)
            .collect()
    };
    assert_eq!(names(&sub), ["OUTER", "REPO", "SUB"]);
    // A nested repository (a `.git` directory) starts its own project.
    std::fs::create_dir(repo.join(".git")).unwrap();
    assert_eq!(names(&sub), ["REPO", "SUB"]);
    assert_eq!(names(&repo), ["REPO"]);
    // A `.git` file (worktree or submodule) counts too.
    std::fs::remove_dir(repo.join(".git")).unwrap();
    std::fs::write(repo.join(".git"), "gitdir: elsewhere").unwrap();
    assert_eq!(names(&sub), ["REPO", "SUB"]);
    assert!(!prompt::system_prompt(&sub).contains("OUTER"));
    drop(outer);
}

#[test]
fn subagent_writes_a_child_log_naming_the_parent() {
    let env = env("subagent");
    let call = r#"{"prompt":"look around","description":"look"}"#;
    // The subagent tool must be found first (PLAN.md D30).
    let responses = vec![
        tool_reply(&[("toolu_0", "find_tools", r#"{"names":["subagent"]}"#)]),
        tool_reply(&[("toolu_1", "subagent", call)]),
        text_reply("child done"),
        text_reply("parent done"),
    ];
    let transport = Arc::new(ScriptedTransport::new(responses));
    let mut conv = Conversation::create(&env.home, &env.cwd, None).unwrap();
    let mut nested = parts(&transport, key_auth(), &env.cwd);
    // The default is 1 (children start none); this test nests one level.
    nested.config.subagent.max_depth = 2;
    let agent = Agent::with_subagent(nested, &env.home, &conv.id);
    let outcome = agent.run_turn(&mut conv, "go", &AtomicBool::new(false), &mut |_| {});
    assert_eq!(outcome, TurnOutcome::Done);
    let sessions = session::list(&env.home, &env.cwd).unwrap();
    assert_eq!(sessions.len(), 2);
    let child = sessions.iter().find(|s| s.id != conv.id).unwrap();
    let child_path = session::log_path(&env.home, &env.cwd, &child.id);
    let (header, _) = trace::read(&child_path).unwrap();
    assert_eq!(header.parent.as_deref(), Some(conv.id.as_str()));
    let events = events(&conv.log_path);
    let results: Vec<_> = events.iter().filter(|e| e.kind == "tool/result").collect();
    let content = results[1].data["message"]["content"][0]["content"]
        .as_str()
        .unwrap()
        .to_string();
    assert!(content.starts_with("child done"), "{content}");
    assert!(content.contains(&child.id));
    let requests = transport.requests();
    assert!(requests[0].body.contains(r#""name":"subagent""#));
    // The child starts with find_tools only; at depth 1 (below maxDepth 2)
    // it has the parent's tool list, subagent included (PLAN.md D40).
    let child_body: serde_json::Value = serde_json::from_str(&requests[2].body).unwrap();
    let tools = child_body["tools"].as_array().unwrap();
    let loaded: Vec<&str> = tools
        .iter()
        .filter(|t| t.get("defer_loading").is_none())
        .map(|t| t["name"].as_str().unwrap())
        .collect();
    assert_eq!(loaded, ["find_tools"]);
    assert!(requests[2].body.contains(r#""name":"subagent""#));
    assert_eq!(header.depth, 1);
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
        "FAST_HARNESS_MODEL" => Some("m2".to_string()),
        "FAST_HARNESS_EFFORT" => Some("low".to_string()),
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
    assert_eq!((config.icons.as_str(), config.greyscale), ("nerd", 0));
    let env = |key: &str| match key {
        "FAST_HARNESS_THEME" => Some(" light ".to_string()),
        "FAST_HARNESS_WIRE" => Some("ON".to_string()),
        "FAST_HARNESS_ICONS" => Some(" plain".to_string()),
        "FAST_HARNESS_GREYSCALE" => Some(" 35 ".to_string()),
        _ => None,
    };
    let config = Config::load_with(&dir.0, &env).unwrap();
    assert_eq!((config.theme.as_str(), config.wire), ("light", true));
    assert_eq!((config.icons.as_str(), config.greyscale), ("plain", 35));
    std::fs::write(
        dir.0.join("config.json"),
        r#"{"icons":"plain","greyscale":140}"#,
    )
    .unwrap();
    let config = Config::load_with(&dir.0, &none).unwrap();
    assert_eq!((config.icons.as_str(), config.greyscale), ("plain", 100));
    std::fs::write(dir.0.join("config.json"), r#"{"greyscale":"lots"}"#).unwrap();
    assert_eq!(Config::load_with(&dir.0, &none).unwrap().greyscale, 0);
    std::fs::write(dir.0.join("config.json"), r#"{"greyscale":42.4}"#).unwrap();
    let odd = |key: &str| (key == "FAST_HARNESS_GREYSCALE").then(|| "half".to_string());
    assert_eq!(Config::load_with(&dir.0, &odd).unwrap().greyscale, 42);
    std::fs::remove_file(dir.0.join("config.json")).unwrap();
    let odd = |key: &str| (key == "FAST_HARNESS_WIRE").then(|| "maybe".to_string());
    assert!(!Config::load_with(&dir.0, &odd).unwrap().wire);
    // The first save creates the folder and the file; later saves keep the other keys.
    // A saved greyscale theme id loads as its base theme with greyscale 100.
    let home = dir.0.join("home");
    save_setting(&home, "theme", serde_json::json!("greyscale-dark")).unwrap();
    let config = Config::load_with(&home, &none).unwrap();
    assert_eq!((config.theme.as_str(), config.greyscale), ("dark", 100));
    let file = home.join("config.json");
    std::fs::write(&file, r#"{"model":"m1","wire":true,"other":[1]}"#).unwrap();
    save_setting(&home, "theme", serde_json::json!("greyscale-light")).unwrap();
    let config = Config::load_with(&home, &none).unwrap();
    assert_eq!(
        (config.model.as_str(), config.theme.as_str(), config.wire),
        ("m1", "light", true)
    );
    assert_eq!(config.greyscale, 100);
    save_setting(&home, "greyscale", serde_json::json!(30)).unwrap();
    save_setting(&home, "theme", serde_json::json!("sakura")).unwrap();
    let config = Config::load_with(&home, &none).unwrap();
    assert_eq!((config.theme.as_str(), config.greyscale), ("sakura", 30));
    let raw: serde_json::Value = serde_json::from_slice(&std::fs::read(&file).unwrap()).unwrap();
    assert_eq!(raw["other"], serde_json::json!([1]));
    assert!(!home.join("config.json.tmp").exists());
    std::fs::write(&file, "[1]").unwrap();
    let err = save_setting(&home, "theme", serde_json::json!("dark")).unwrap_err();
    assert!(err.contains("not a JSON object"), "{err}");
    assert_eq!(std::fs::read_to_string(&file).unwrap(), "[1]");
    drop(dir);
}
