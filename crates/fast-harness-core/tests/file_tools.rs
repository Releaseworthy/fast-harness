//! read, edit and write on real files: content-hash dedupe and changed
//! lines, the continue line under the generic cap, UTF-16 and BOMs, long
//! lines read by column, and the guards on unread, changed, binary and
//! emptied files.

mod agent_support;

use std::path::Path;
use std::sync::atomic::AtomicBool;
use std::time::{Duration, SystemTime};

use agent_support::*;
use fast_harness_core::tools::{budget, builtin, ToolCtx, ToolOutput, ToolSession};
use serde_json::{json, Value};

fn call(session: &ToolSession, cwd: &Path, id: &str, tool: &str, args: Value) -> ToolOutput {
    let cancel = AtomicBool::new(false);
    let ctx = ToolCtx::new(cwd, &cancel)
        .with_call_id(id)
        .with_session(session);
    builtin().run(tool, &args.to_string(), &ctx)
}

fn text(session: &ToolSession, cwd: &Path, id: &str, tool: &str, args: Value) -> String {
    call(session, cwd, id, tool, args).into_text()
}

fn numbered(count: usize) -> String {
    (1..=count).map(|i| format!("line {i}\n")).collect()
}

#[test]
fn an_own_edit_comes_back_changed_a_touch_keeps_a_reread_unchanged_and_others_changes_come_alone() {
    let env = env("ft-dedupe");
    let file = env.cwd.join("a.txt");
    std::fs::write(&file, numbered(50)).unwrap();
    let session = ToolSession::new(None);
    let read = |id: &str| text(&session, &env.cwd, id, "read", json!({"path": "a.txt"}));
    assert!(read("c1").starts_with("1: line 1\n"));
    let edit = json!({"path": "a.txt", "old_string": "line 10\n", "new_string": "ten\nmore\n"});
    let out = call(&session, &env.cwd, "c2", "edit", edit);
    assert!(!out.is_error(), "{}", out.text());
    // The own edit's lines come back (D87), then the range is unchanged.
    let reread = read("c3");
    assert!(reread.starts_with("[changed since call c1:"), "{reread}");
    assert!(reread.ends_with("]\n10: ten\n11: more\n"), "{reread}");
    // Same content, a later modified time: still unchanged.
    let touched = std::fs::OpenOptions::new().write(true).open(&file).unwrap();
    touched
        .set_modified(SystemTime::now() + Duration::from_secs(60))
        .unwrap();
    drop(touched);
    assert_eq!(read("c4"), "[unchanged since call c1, c3]");
    // Someone else changes line 30: only that line comes back.
    let other = std::fs::read_to_string(&file)
        .unwrap()
        .replace("line 30\n", "thirty\n");
    std::fs::write(&file, other).unwrap();
    let diff = read("c5");
    assert!(
        diff.starts_with("[changed since call c1, c3: only the changed lines"),
        "{diff}"
    );
    assert!(diff.ends_with("]\n31: thirty\n"), "{diff}");
    assert_eq!(read("c6"), "[unchanged since call c1, c3, c5]");
}

#[test]
fn the_continue_line_survives_the_generic_cap() {
    let env = env("ft-page");
    let long: String = (1..=2000)
        .map(|i| format!("{i:05} {}\n", "y".repeat(70)))
        .collect();
    std::fs::write(env.cwd.join("big.txt"), long).unwrap();
    let session = ToolSession::new(None);
    let cancel = AtomicBool::new(false);
    let read = |id: &str, args: Value| {
        let ctx = ToolCtx::new(&env.cwd, &cancel)
            .with_call_id(id)
            .with_session(&session)
            .with_budget(8192);
        builtin().run("read", &args.to_string(), &ctx).into_text()
    };
    // Real call ids are long: the notes naming them grow with them.
    let (one, two) = (
        "toolu_01AAAAAAAAAAAAAAAAAAAAAAAA",
        "toolu_01BBBBBBBBBBBBBBBBBBBBBBBB",
    );
    read(one, json!({"path": "big.txt", "offset": 1, "limit": 60}));
    read(two, json!({"path": "big.txt", "offset": 300, "limit": 60}));
    // Lines 30-60 and 300-349 were shown: notes before and after the page.
    for (id, args) in [
        ("c3", json!({"path": "big.txt", "offset": 30, "limit": 320})),
        ("c4", json!({"path": "big.txt"})),
        (
            "c5",
            json!({"path": "big.txt", "offset": 1000, "limit": 1000}),
        ),
    ] {
        let out = read(id, args);
        let cut = out.contains(budget::SPILL_MARK) || out.contains("[output cut");
        assert!(!cut, "{id}: cut by the cap");
        assert!(out.contains("continue with offset"), "{id}: {out}");
        assert!(out.len() <= 8192, "{id}: {} bytes", out.len());
    }
}

#[test]
fn utf16_and_bom_files_read_clean_and_keep_their_encoding() {
    let env = env("ft-utf16");
    let session = ToolSession::new(None);
    let utf16 = |text: &str, bom: bool| -> Vec<u8> {
        let mut out = if bom { vec![0xFF, 0xFE] } else { Vec::new() };
        text.encode_utf16()
            .for_each(|u| out.extend_from_slice(&u.to_le_bytes()));
        out
    };
    std::fs::write(env.cwd.join("w.ini"), utf16("[a]\r\nkey=1\r\n", true)).unwrap();
    std::fs::write(env.cwd.join("n.ini"), utf16("plain=1\r\nnext=2\r\n", false)).unwrap();
    std::fs::write(env.cwd.join("b.txt"), b"\xEF\xBB\xBFfirst\nsecond\n").unwrap();
    let read = |id: &str, path: &str| text(&session, &env.cwd, id, "read", json!({"path": path}));
    assert_eq!(read("c1", "w.ini"), "1: [a]\n2: key=1\n");
    assert_eq!(read("c2", "n.ini"), "1: plain=1\n2: next=2\n");
    assert_eq!(read("c3", "b.txt"), "1: first\n2: second\n");
    let edit = |id: &str, path: &str, old: &str, new: &str| {
        let args = json!({"path": path, "old_string": old, "new_string": new});
        let out = call(&session, &env.cwd, id, "edit", args);
        assert!(!out.is_error(), "{}", out.text());
    };
    edit("c4", "w.ini", "key=1\r\n", "key=2\r\nnew=3\r\n");
    let want = utf16("[a]\r\nkey=2\r\nnew=3\r\n", true);
    assert_eq!(std::fs::read(env.cwd.join("w.ini")).unwrap(), want);
    edit("c5", "b.txt", "second", "2nd");
    assert_eq!(
        std::fs::read(env.cwd.join("b.txt")).unwrap(),
        b"\xEF\xBB\xBFfirst\n2nd\n"
    );
    let args = json!({"path": "w.ini", "content": "[b]\nk=v\n"});
    let out = text(&session, &env.cwd, "c6", "write", args);
    assert!(out.ends_with("; kept UTF-16LE, CRLF)"), "{out}");
    let want = utf16("[b]\r\nk=v\r\n", true);
    assert_eq!(std::fs::read(env.cwd.join("w.ini")).unwrap(), want);
}

#[test]
fn edit_and_write_need_a_read_of_the_current_content() {
    let env = env("ft-guard");
    let file = env.cwd.join("g.txt");
    std::fs::write(&file, "alpha\n").unwrap();
    let session = ToolSession::new(None);
    let edit = json!({"path": "g.txt", "old_string": "alpha", "new_string": "beta"});
    let write = json!({"path": "g.txt", "content": "gamma\n"});
    let refused = |id: &str, tool: &str, args: &Value, part: &str| {
        let out = call(&session, &env.cwd, id, tool, args.clone());
        assert!(
            out.is_error() && out.text().contains(part),
            "{id}: {}",
            out.text()
        );
    };
    refused(
        "c1",
        "edit",
        &edit,
        "was not read in this session; read it first",
    );
    refused("c2", "write", &write, "was not read in this session");
    text(&session, &env.cwd, "c3", "read", json!({"path": "g.txt"}));
    std::fs::write(&file, "alpha changed\n").unwrap();
    refused(
        "c4",
        "edit",
        &edit,
        "changed since this session last read or wrote it (call c3)",
    );
    refused(
        "c5",
        "write",
        &write,
        "changed since this session last read",
    );
    text(&session, &env.cwd, "c6", "read", json!({"path": "g.txt"}));
    assert!(!call(&session, &env.cwd, "c7", "edit", edit).is_error());
    assert!(!call(&session, &env.cwd, "c8", "write", write).is_error());
    // A new file needs no read; another session (a subagent) read nothing.
    let new = json!({"path": "new.txt", "content": "x"});
    assert!(!call(&session, &env.cwd, "c9", "write", new).is_error());
    let child = ToolSession::new(None);
    let edit = json!({"path": "g.txt", "old_string": "gamma", "new_string": "delta"});
    let out = call(&child, &env.cwd, "k1", "edit", edit);
    assert!(out.is_error(), "{}", out.text());
}

#[test]
fn write_over_a_binary_or_with_empty_content_needs_its_flag() {
    let env = env("ft-flags");
    std::fs::write(env.cwd.join("tool.exe"), b"MZ\x90\x00\x03\x00\x00\x00").unwrap();
    std::fs::write(env.cwd.join("t.txt"), "keep me\n").unwrap();
    let session = ToolSession::new(None);
    let out = call(
        &session,
        &env.cwd,
        "c1",
        "write",
        json!({"path": "tool.exe", "content": "x"}),
    );
    assert!(
        out.is_error() && out.text().contains("overwrite_binary: true"),
        "{}",
        out.text()
    );
    let args = json!({"path": "tool.exe", "content": "x", "overwrite_binary": true});
    assert!(!call(&session, &env.cwd, "c2", "write", args).is_error());
    text(&session, &env.cwd, "c3", "read", json!({"path": "t.txt"}));
    let out = call(
        &session,
        &env.cwd,
        "c4",
        "write",
        json!({"path": "t.txt", "content": ""}),
    );
    assert!(
        out.is_error() && out.text().contains("allow_empty: true"),
        "{}",
        out.text()
    );
    assert_eq!(
        std::fs::read_to_string(env.cwd.join("t.txt")).unwrap(),
        "keep me\n"
    );
    let args = json!({"path": "t.txt", "content": "", "allow_empty": true});
    assert!(!call(&session, &env.cwd, "c5", "write", args).is_error());
    assert_eq!(std::fs::read(env.cwd.join("t.txt")).unwrap(), b"");
}
