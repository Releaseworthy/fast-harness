//! read and edit answers the model acts on: a read past the end after a
//! read, the lines an own edit changed, the head size, the range line of
//! an overlapping read, and where a failed edit's text is.

mod agent_support;

use std::path::Path;
use std::sync::atomic::AtomicBool;

use agent_support::*;
use fast_harness_core::tools::{builtin, ToolCtx, ToolOutput, ToolSession};
use serde_json::{json, Value};

fn call(session: &ToolSession, cwd: &Path, id: &str, tool: &str, args: Value) -> ToolOutput {
    let cancel = AtomicBool::new(false);
    let ctx = ToolCtx::new(cwd, &cancel)
        .with_call_id(id)
        .with_session(session);
    builtin().run(tool, &args.to_string(), &ctx)
}

fn numbered(count: usize) -> String {
    (1..=count).map(|i| format!("line {i}\n")).collect()
}

fn last_line(text: &str) -> &str {
    text.lines().last().unwrap_or_default()
}

#[test]
fn an_offset_past_the_end_fails_after_a_read_as_before_it() {
    let env = env("rn-past-end");
    std::fs::write(env.cwd.join("a.txt"), numbered(50)).unwrap();
    let args = json!({"path": "a.txt", "offset": 60, "limit": 5});
    let fresh = call(
        &ToolSession::new(None),
        &env.cwd,
        "c0",
        "read",
        args.clone(),
    );
    assert!(fresh.is_error(), "{}", fresh.text());
    let session = ToolSession::new(None);
    let head = call(&session, &env.cwd, "c1", "read", json!({"path": "a.txt"}));
    assert!(head.text().starts_with("1: line 1\n"));
    let again = call(&session, &env.cwd, "c2", "read", args);
    assert!(again.is_error(), "{}", again.text());
    assert_eq!(again.text(), fresh.text());
    assert!(again.text().contains("past the end of the file (50 lines)"));
}

#[test]
fn a_reread_after_an_own_edit_shows_the_changed_lines() {
    let env = env("rn-own-edit");
    std::fs::write(env.cwd.join("a.txt"), numbered(50)).unwrap();
    let session = ToolSession::new(None);
    let read =
        |id: &str| call(&session, &env.cwd, id, "read", json!({"path": "a.txt"})).into_text();
    assert!(read("c1").starts_with("1: line 1\n"));
    // A forgotten newline merges lines 10 and 11.
    let edit = json!({"path": "a.txt", "old_string": "line 10\n", "new_string": "ten"});
    let out = call(&session, &env.cwd, "c2", "edit", edit);
    assert!(!out.is_error(), "{}", out.text());
    let edit = json!({"path": "a.txt", "old_string": "line 40\n", "new_string": "forty\n"});
    assert!(!call(&session, &env.cwd, "c3", "edit", edit).is_error());
    let reread = read("c4");
    assert!(reread.starts_with("[changed since call c1"), "{reread}");
    assert!(reread.contains("\n10: tenline 11\n"), "{reread}");
    assert!(reread.ends_with("\n39: forty\n"), "{reread}");
    assert!(!reread.contains("line 20"), "{reread}");
    assert_eq!(read("c5"), "[unchanged since call c1, c4]");
}

#[test]
fn a_head_read_returns_about_seven_kilobytes() {
    let env = env("rn-head");
    let text: String = (1..=600)
        .map(|i| format!("{i:>4} {}\n", "x".repeat(40)))
        .collect();
    std::fs::write(env.cwd.join("a.txt"), text).unwrap();
    let session = ToolSession::new(None);
    let head = call(&session, &env.cwd, "c1", "read", json!({"path": "a.txt"})).into_text();
    assert!(
        head.len() > 6 * 1024 && head.len() <= 7 * 1024,
        "{}",
        head.len()
    );
}

#[test]
fn an_overlapping_read_ends_with_the_whole_requested_range() {
    let env = env("rn-overlap");
    std::fs::write(env.cwd.join("a.txt"), numbered(300)).unwrap();
    let session = ToolSession::new(None);
    let read = |id: &str, offset: u64, limit: u64| {
        let args = json!({"path": "a.txt", "offset": offset, "limit": limit});
        call(&session, &env.cwd, id, "read", args).into_text()
    };
    read("c1", 1, 60);
    let overlap = read("c2", 50, 31);
    assert!(overlap.contains("\n80: line 80\n"), "{overlap}");
    let want = "[lines 50 to 80 of 300; continue with offset 81]";
    assert_eq!(last_line(&overlap), want);
    read("c3", 120, 20);
    let tail = read("c4", 100, 26);
    assert!(tail.contains("[already shown, unchanged: lines 120-125 (call c3)]\n"));
    let want = "[lines 100 to 125 of 300; continue with offset 126]";
    assert_eq!(last_line(&tail), want);
    read("c5", 290, 20);
    let end = read("c6", 280, 50);
    assert_eq!(last_line(&end), "[lines 280 to 300 of 300]", "{end}");
}

#[test]
fn a_failed_edit_names_the_lines_of_the_nearest_or_every_match() {
    let env = env("rn-edit-miss");
    let code = "fn main() {\n    run();\n}\n\nfn alpha(x: u32) -> u32 {\n    x + 1\n}\n\
                \nfn beta() {\n    run();\n}\n";
    std::fs::write(env.cwd.join("a.rs"), code).unwrap();
    let session = ToolSession::new(None);
    call(&session, &env.cwd, "c1", "read", json!({"path": "a.rs"}));
    let miss = json!({"path": "a.rs", "old_string": "fn alpha(x: u64) -> u32 {\n    x + 1\n}",
        "new_string": "fn alpha() {}"});
    let out = call(&session, &env.cwd, "c2", "edit", miss);
    assert!(out.is_error());
    let text = out.text();
    assert!(text.contains("lines 5-7"), "{text}");
    assert!(text.contains("5: fn alpha(x: u32) -> u32 {"), "{text}");
    let many = json!({"path": "a.rs", "old_string": "    run();\n", "new_string": "    go();\n"});
    let out = call(&session, &env.cwd, "c3", "edit", many);
    assert!(out.is_error());
    assert!(out.text().contains("at lines 2, 10"), "{}", out.text());
}
