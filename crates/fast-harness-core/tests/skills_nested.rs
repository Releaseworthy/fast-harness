//! Phase 19 (PLAN.md D43): skills in the index, the system prompt line,
//! skill bodies from `find_tools`, nested AGENTS.md on first touch, and
//! both kept through resume and compaction.

mod agent_support;

use std::fs;
use std::path::Path;
use std::sync::atomic::AtomicBool;
use std::sync::Arc;

use agent_support::*;
use fast_harness_core::agent::nested::{self, MARK as NESTED_MARK};
use fast_harness_core::agent::{Agent, Context, Conversation};
use fast_harness_core::app::{doctor, load_settings};
use fast_harness_core::config::index::ExtIndex;
use fast_harness_core::config::layers::Layers;
use fast_harness_core::config::trust;
use fast_harness_core::events::TurnOutcome;
use fast_harness_core::http::ScriptedTransport;
use fast_harness_core::session::compacted::KEPT;
use fast_harness_core::tools::skills::{self, SKILL_BYTES};
use fast_harness_core::tools::{Extensions, ToolCtx};
use fast_harness_core::trace::kind;
use fast_harness_core::types::{Block, Message};

fn put(path: &Path, text: &str) {
    fs::create_dir_all(path.parent().unwrap()).unwrap();
    fs::write(path, text).unwrap();
}

fn skill(dir: &Path, name: &str, description: &str, body: &str) {
    let text = format!("---\nname: {name}\ndescription: {description}\n---\n{body}\n");
    put(&dir.join("skills").join(name).join("SKILL.md"), &text);
}

fn find(ext: &Extensions, cwd: &Path, args: &str) -> String {
    let registry = ext.builtin();
    let cancel = AtomicBool::new(false);
    let out = registry.run("find_tools", args, &ToolCtx::new(cwd, &cancel));
    assert!(!out.is_error(), "{}", out.text());
    out.into_text()
}

fn results(messages: &[Message]) -> Vec<String> {
    let blocks = messages.iter().flat_map(|m| m.content.iter());
    let texts = blocks.filter_map(|block| match block {
        Block::ToolResult { content, .. } => Some(content.clone()),
        _ => None,
    });
    texts.collect()
}

#[test]
fn untrusted_project_skills_do_not_load_and_trusted_ones_do() {
    let env = env("skills-trust");
    skill(
        &env.home,
        "pdf-tools",
        "Extract PDF text and fill forms",
        "Use pdftotext.",
    );
    let project = env.cwd.join(".fast-harness");
    skill(
        &project,
        "deploy",
        "Deploy the site with wrangler",
        "Run the deploy script.",
    );
    let untrusted = ExtIndex::build(&Layers::find(&env.home, &env.cwd));
    let names: Vec<&str> = untrusted
        .skills
        .iter()
        .map(|i| i.entry.name.as_str())
        .collect();
    assert_eq!(names, vec!["pdf-tools"]);
    trust::trust(&env.home, &env.cwd).unwrap();
    let trusted = ExtIndex::build(&Layers::find(&env.home, &env.cwd));
    let names: Vec<&str> = trusted
        .skills
        .iter()
        .map(|i| i.entry.name.as_str())
        .collect();
    assert_eq!(names, vec!["pdf-tools", "deploy"]);
    let ext = Extensions::new(trusted);
    assert_eq!(
        ext.skills_line(),
        "Skills: pdf-tools, deploy (load one with find_tools)"
    );
    let (mut context, _) = Context::read_at(&env.home, &env.cwd, "m", "2026-11-12");
    context.skills = ext.skills_line();
    assert!(context
        .system()
        .contains("\nSkills: pdf-tools, deploy (load one"));
    let settings = load_settings(&env.home, Some(&env.cwd), &|_| None).unwrap();
    let report = doctor(&settings);
    assert!(report.contains("skills:\n  pdf-tools (user)"), "{report}");
    assert!(report.contains("  deploy (project)"), "{report}");
}

#[test]
fn find_tools_returns_a_skill_body_with_its_folder_capped() {
    let env = env("skills-find");
    skill(
        &env.home,
        "pdf-tools",
        "Extract PDF text and fill forms",
        "Use pdftotext.",
    );
    let long = "x".repeat(SKILL_BYTES + 5000);
    skill(&env.home, "huge", "A very long skill", &long);
    let ext = Extensions::new(ExtIndex::build(&Layers::user_only(&env.home)));
    let registry = ext.builtin();
    let spec = &registry.specs()[0];
    assert!(spec.description.contains("also match the skills"));
    let by_name = find(&ext, &env.cwd, r#"{"names":["pdf-tools","read"]}"#);
    let folder = env.home.join("skills").join("pdf-tools");
    assert!(by_name.starts_with("Loaded: read"), "{by_name}");
    assert!(by_name.contains(&format!("[skill pdf-tools: {}]", folder.display())));
    assert!(by_name.contains("Use pdftotext.") && !by_name.contains("description:"));
    let by_query = find(&ext, &env.cwd, r#"{"query":"fill pdf forms"}"#);
    assert!(by_query.contains("Use pdftotext."), "{by_query}");
    let huge = find(&ext, &env.cwd, r#"{"names":["huge"]}"#);
    assert!(huge.contains("cut at 20 KB"));
    assert!(huge.len() < SKILL_BYTES + 1024, "{}", huge.len());
}

#[test]
fn nested_agents_md_joins_the_first_touching_result_once_and_survives() {
    let env = env("nested-agents");
    put(
        &env.cwd.join("sub").join("AGENTS.md"),
        "Nested rule: tabs only.",
    );
    for name in ["a.txt", "b.txt", "c.txt"] {
        put(&env.cwd.join("sub").join(name), "text");
    }
    put(&env.cwd.join("top.txt"), "top");
    skill(&env.home, "pdf-tools", "Extract PDF text", "Use pdftotext.");
    let ext = Arc::new(Extensions::new(ExtIndex::build(&Layers::user_only(
        &env.home,
    ))));
    let transport = Arc::new(ScriptedTransport::new(vec![
        tool_reply(&[("t0", "find_tools", r#"{"names":["read","pdf-tools"]}"#)]),
        tool_reply(&[("t1", "read", r#"{"path":"top.txt"}"#)]),
        tool_reply(&[("t2", "read", r#"{"path":"sub/a.txt"}"#)]),
        tool_reply(&[("t3", "read", r#"{"path":"sub/b.txt"}"#)]),
        text_reply("done"),
        tool_reply(&[("t4", "read", r#"{"path":"sub/c.txt"}"#)]),
        text_reply("again"),
        text_reply("## Task\nsummary"),
    ]));
    let mut parts = parts(&transport, key_auth(), &env.cwd);
    parts.extensions = Arc::clone(&ext);
    let agent = Agent::new(parts, Arc::new(ext.builtin()));
    let cancel = AtomicBool::new(false);
    let mut conv = Conversation::create(&env.home, &env.cwd, None).unwrap();
    let outcome = agent.run_turn(&mut conv, "go", &cancel, &mut |_| {});
    assert_eq!(outcome, TurnOutcome::Done);
    let texts = results(&conv.messages);
    let file = env.cwd.join("sub").join("AGENTS.md");
    let mark = format!("{NESTED_MARK}{}]", file.display());
    assert!(!texts[1].contains(NESTED_MARK), "top.txt is not below sub");
    assert!(texts[2].contains(&mark) && texts[2].contains("tabs only"));
    assert!(!texts[3].contains(NESTED_MARK), "only once");
    let id = conv.id.clone();
    let mut resumed = Conversation::resume(&env.home, &env.cwd, &id).unwrap();
    assert_eq!(
        nested::loaded(&resumed.messages),
        vec![file.display().to_string()]
    );
    assert_eq!(skills::loaded(&resumed.messages), vec!["pdf-tools"]);
    agent.run_turn(&mut resumed, "more", &cancel, &mut |_| {});
    let last = results(&resumed.messages).pop().unwrap();
    assert!(
        !last.contains(NESTED_MARK),
        "rebuilt from the log on resume"
    );
    let outcome = agent.run_turn(&mut resumed, "/compact", &cancel, &mut |_| {});
    assert_eq!(outcome, TurnOutcome::Done);
    let Block::Text { text } = &resumed.messages[0].content[0] else {
        panic!("summary first");
    };
    assert!(text.contains(KEPT) && text.contains(&mark) && text.contains("[skill pdf-tools: "));
    let log = events(&resumed.log_path);
    let compaction = first(&log, kind::COMPACTION);
    assert_eq!(compaction.data["kept"].as_array().unwrap().len(), 2);
    let again = Conversation::resume(&env.home, &env.cwd, &id).unwrap();
    assert_eq!(nested::loaded(&again.messages).len(), 1);
    assert_eq!(skills::loaded(&again.messages), vec!["pdf-tools"]);
}
