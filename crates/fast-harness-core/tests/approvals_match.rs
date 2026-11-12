//! The allow list fails closed (PLAN.md D64): compound commands, path
//! escapes, unknown tools, and the layer rule.

mod agent_support;

use std::path::Path;

use agent_support::env;
use fast_harness_core::config::approvals::{
    command_allowed, covered, path_allowed, simple_command, subject, Allow, Approvals,
};
use fast_harness_core::config::layers::{self, Layers};
use fast_harness_core::tools::Effect;

fn words(list: &[&str]) -> Vec<String> {
    list.iter().map(|s| s.to_string()).collect()
}

#[test]
fn an_allowed_prefix_matches_on_a_word_border_only() {
    let allow = words(&["git status", "cargo test"]);
    for command in [
        "git status",
        "git status -s",
        "GIT Status",
        "  cargo   test -p core",
    ] {
        assert!(command_allowed(command, &allow), "{command}");
    }
    for command in ["git statusx", "git", "git log", "cargo", "cargotest", ""] {
        assert!(!command_allowed(command, &allow), "{command}");
    }
}

#[test]
fn every_compound_form_asks_whatever_its_start() {
    let allow = words(&["git status"]);
    let compound = [
        "git status; rm x",
        "git status && rm x",
        "git status || rm x",
        "git status | rm x",
        "git status & rm x",
        "git status\nrm x",
        "git status\r\nrm x",
        "git status `\nrm x",
        "git status $(rm x)",
        "git status (rm x)",
        "git status @(rm x)",
        "git status {rm x}",
        "git status > out.txt",
        "git status 2>&1",
        "git status < in.txt",
        "git status \u{2013}x",
        "git status \t-s",
    ];
    for command in compound {
        assert!(!simple_command(command), "{command:?} is compound");
        assert!(!command_allowed(command, &allow), "{command:?} asks");
    }
}

#[test]
fn paths_are_normalised_before_matching() {
    let dir = env("approvals-paths");
    let cwd = &dir.cwd;
    std::fs::create_dir_all(cwd.join("src")).unwrap();
    let allow = words(&["src"]);
    assert!(path_allowed("src/a.rs", cwd, &allow));
    assert!(path_allowed("src/new/deep.rs", cwd, &allow));
    assert!(path_allowed(
        &cwd.join("src").join("b.rs").to_string_lossy(),
        cwd,
        &allow
    ));
    assert!(
        !path_allowed("src/../secret.txt", cwd, &allow),
        "a .. escape"
    );
    assert!(
        !path_allowed("srcx/a.rs", cwd, &allow),
        "a name that only starts alike"
    );
    assert!(!path_allowed("other/a.rs", cwd, &allow));
    let deep = "../".repeat(40) + "x";
    assert!(!path_allowed(&deep, cwd, &allow), "above the root");
    if cfg!(windows) {
        assert!(
            path_allowed("SRC/A.rs", cwd, &allow),
            "case is ignored on Windows"
        );
        let drive = if cwd.to_string_lossy().starts_with('Z') {
            "Y:\\src\\a.rs"
        } else {
            "Z:\\src\\a.rs"
        };
        assert!(!path_allowed(drive, cwd, &allow), "another drive");
    }
}

#[test]
fn reads_are_covered_and_unknown_or_run_tools_ask() {
    let cwd = Path::new(".");
    let allow = Allow {
        tools: words(&["mcp__gh__list"]),
        ..Allow::default()
    };
    for tool in ["read", "grep", "glob", "find_tools"] {
        assert!(
            covered(&allow, tool, Some(Effect::Read), "{}", cwd),
            "{tool}"
        );
    }
    assert!(covered(
        &allow,
        "mcp__gh__list",
        Some(Effect::Run),
        "{}",
        cwd
    ));
    assert!(!covered(
        &allow,
        "mcp__gh__push",
        Some(Effect::Run),
        "{}",
        cwd
    ));
    assert!(
        !covered(&allow, "nope", None, "{}", cwd),
        "an unknown tool asks"
    );
    let shell = r#"{"command":"git status"}"#;
    assert!(!covered(&allow, "shell", Some(Effect::Run), shell, cwd));
    let allow = Allow {
        commands: words(&["git status"]),
        ..Allow::default()
    };
    assert!(covered(&allow, "shell", Some(Effect::Run), shell, cwd));
    let other = r#"{"command":"git status"}"#;
    assert!(
        !covered(&allow, "pwsh_tool", Some(Effect::Run), other, cwd),
        "only the shell tool"
    );
}

fn write(path: &Path, text: &str) {
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    std::fs::write(path, text).unwrap();
}

#[test]
fn a_project_cannot_turn_approvals_off_or_widen_the_allow_list() {
    let dir = env("approvals-layers");
    write(
        &dir.home.join("config.json"),
        r#"{"approvals": "ask", "allow": {"commands": ["git status"]}}"#,
    );
    let project = r#"{"approvals": "off", "allow": {"commands": ["rm"], "tools": ["x"]}}"#;
    write(&dir.cwd.join(".fast-harness").join("config.json"), project);
    let layered = |trusted| {
        let found = Layers {
            home: dir.home.clone(),
            project: Some(dir.cwd.clone()),
            trusted,
            agents_skipped: Vec::new(),
        };
        layers::load(found, &|_| None).unwrap()
    };
    for trusted in [true, false] {
        let config = layered(trusted).config;
        assert_eq!(config.approvals, Approvals::Ask, "trusted {trusted}");
        assert_eq!(config.allow.commands, ["git status"]);
        assert!(config.allow.tools.is_empty());
    }
    // A project may turn it on.
    write(&dir.home.join("config.json"), "{}");
    write(
        &dir.cwd.join(".fast-harness").join("config.json"),
        r#"{"approvals": "ask"}"#,
    );
    let on = layered(true);
    assert_eq!(on.config.approvals, Approvals::Ask);
    assert_eq!(on.get("approvals").unwrap().1, layers::Source::Project);
    assert_eq!(
        layered(false).config.approvals,
        Approvals::Off,
        "untrusted: nothing"
    );
}

#[test]
fn a_long_summary_keeps_the_end_of_the_command() {
    let filler = "a".repeat(600);
    let command = format!("echo {filler}\nrm -rf C:\\important");
    let raw = serde_json::json!({ "command": command }).to_string();
    let summary = subject("shell", &raw, Path::new(".")).summary;
    assert!(summary.chars().count() <= 320, "{summary}");
    assert!(!summary.contains('\n'), "one line: {summary}");
    assert!(summary.starts_with("run: echo aaa"), "{summary}");
    assert!(summary.ends_with(r"rm -rf C:\important"), "{summary}");
    assert!(summary.contains("characters left out"), "{summary}");
    let short = subject("shell", r#"{"command":"ls"}"#, Path::new(".")).summary;
    assert!(!short.contains("left out"), "{short}");
}
