//! The tool card model, fed with the outputs the tools really print.

use fast_harness_core::view::diff::line_diff;
use fast_harness_core::view::tool_detail::{duration, size};
use fast_harness_core::view::{
    tool_detail, tool_summary, Field, Line, LineKind, Tone, ToolDetail, ToolKind, MAX_LINES,
};
use serde_json::{json, Value};

fn card(name: &str, args: Value, out: Option<&str>, err: bool) -> ToolDetail {
    tool_detail(name, &args.to_string(), out, err, None)
}

fn chips(detail: &ToolDetail) -> Vec<(&str, Tone)> {
    detail
        .chips
        .iter()
        .map(|c| (c.text.as_str(), c.tone))
        .collect()
}

fn fields(detail: &ToolDetail) -> Vec<(&str, &str)> {
    detail.fields.iter().map(pair).collect()
}

fn pair(field: &Field) -> (&str, &str) {
    (field.label.as_str(), field.value.as_str())
}

fn line(kind: LineKind, old: Option<u64>, new: Option<u64>, text: &str) -> Line {
    Line {
        kind,
        old,
        new,
        text: text.into(),
    }
}

#[test]
fn words_and_kinds() {
    let kinds = ["read", "write", "edit", "grep", "glob", "shell", "subagent"];
    for name in kinds {
        assert_eq!(ToolKind::of(name).word(), name);
    }
    assert_eq!(ToolKind::of("find_tools"), ToolKind::Find);
    assert_eq!(
        (ToolKind::Find.word(), ToolKind::of("mcp").word()),
        ("find", "other")
    );
    let tones = [
        Tone::Plain,
        Tone::Success,
        Tone::Error,
        Tone::Info,
        Tone::Warning,
    ];
    let words: Vec<&str> = tones.iter().map(|t| t.word()).collect();
    assert_eq!(words, ["plain", "success", "error", "info", "warning"]);
    let kinds = [LineKind::Context, LineKind::Added, LineKind::Removed];
    let words: Vec<&str> = kinds.iter().map(|k| k.word()).collect();
    assert_eq!(words, ["context", "added", "removed"]);
    assert_eq!(
        (LineKind::Header.word(), LineKind::Gap.word()),
        ("header", "gap")
    );
}

#[test]
fn sizes_and_durations() {
    assert_eq!(size(1023), "1023 B");
    assert_eq!(size(1536), "1.5 KB");
    assert_eq!(size(1_572_864), "1.5 MB");
    assert_eq!(duration(850), "850 ms");
    assert_eq!(duration(2500), "2.5 s");
    assert_eq!(duration(185_000), "3 min 5 s");
    let read = |ms| tool_detail("read", r#"{"path":"a"}"#, Some("1: x"), false, ms);
    assert_eq!(read(Some(0)).chips.len(), 1, "no chip under 1 ms");
    let last = read(Some(1200)).chips.pop().unwrap();
    assert_eq!((last.text.as_str(), last.tone), ("1.2 s", Tone::Plain));
}

#[test]
fn read_numbers_lines_and_names_the_window() {
    let all = card(
        "read",
        json!({"path": "F:\\src\\a.rs"}),
        Some("1: fn a() {}\n2: x: y\n"),
        false,
    );
    assert_eq!(all.kind, ToolKind::Read);
    assert_eq!(all.target, "F:\\src\\a.rs");
    let want = [
        line(LineKind::Context, None, Some(1), "fn a() {}"),
        line(LineKind::Context, None, Some(2), "x: y"),
    ];
    assert_eq!((all.lines.as_slice(), all.lines_total), (&want[..], 2));
    assert_eq!(chips(&all), [("all 2 lines", Tone::Info)]);
    assert!(all.fields.is_empty() && all.text.is_empty());
    let note = "2: two\n[showing lines 2-2 of 3; 1 more lines, continue with offset=3]\n";
    let part = card(
        "read",
        json!({"path": "a", "offset": 2, "limit": 1}),
        Some(note),
        false,
    );
    assert_eq!(chips(&part), [("lines 2\u{2013}2 of 3", Tone::Info)]);
    assert!(fields(&part).is_empty());
    let tail = card(
        "read",
        json!({"path": "a", "offset": 5}),
        Some("5: e\n6: f\n"),
        false,
    );
    assert_eq!(chips(&tail), [("lines 5\u{2013}6", Tone::Info)]);
    let empty = card("read", json!({"path": "a"}), Some("(empty file)"), false);
    assert_eq!(chips(&empty), [("empty file", Tone::Plain)]);
    let missing = card(
        "read",
        json!({"path": "a"}),
        Some("cannot read a: gone"),
        true,
    );
    assert!(missing.chips.is_empty() && missing.lines.is_empty());
    assert_eq!(missing.text, "cannot read a: gone");
}

#[test]
fn write_shows_the_new_content_and_what_it_replaced() {
    let args = json!({"path": "a/b.txt", "content": "one\ntwo\n"});
    let new = card(
        "write",
        args.clone(),
        Some("Wrote 8 bytes (2 lines) to a/b.txt (new file)"),
        false,
    );
    let want = [
        line(LineKind::Added, None, Some(1), "one"),
        line(LineKind::Added, None, Some(2), "two"),
    ];
    assert_eq!(new.lines, want);
    let added = ("+2 lines", Tone::Success);
    assert_eq!(
        chips(&new),
        [("new file", Tone::Info), added, ("8 B", Tone::Plain)]
    );
    let out = "Wrote 8 bytes (2 lines) to a/b.txt (replaced 3 bytes, 1 line)";
    let replaced = card("write", args.clone(), Some(out), false);
    assert_eq!(chips(&replaced)[0], ("replaced 1 line", Tone::Warning));
    let old = card(
        "write",
        args.clone(),
        Some("Wrote 8 bytes to a/b.txt"),
        false,
    );
    assert_eq!(chips(&old), [added, ("8 B", Tone::Plain)]);
    let running = card("write", args, None, false);
    assert_eq!(
        (chips(&running), running.lines.len()),
        (vec![added, ("8 B", Tone::Plain)], 2)
    );
}

#[test]
fn edit_shows_a_numbered_diff() {
    let args = json!({"path": "f.rs", "old_string": "a\nb\nc\n", "new_string": "a\nB\nc\nd\n"});
    let out = "Replaced 1 occurrence in f.rs at line 10";
    let one = card("edit", args.clone(), Some(out), false);
    let want = [
        line(LineKind::Context, Some(10), Some(10), "a"),
        line(LineKind::Removed, Some(11), None, "b"),
        line(LineKind::Added, None, Some(11), "B"),
        line(LineKind::Context, Some(12), Some(12), "c"),
        line(LineKind::Added, None, Some(13), "d"),
    ];
    assert_eq!(one.lines, want);
    let counts = [("+2", Tone::Success), ("\u{2212}1", Tone::Error)];
    assert_eq!(
        chips(&one),
        [counts[0], counts[1], ("line 10", Tone::Plain)]
    );
    assert!(one.fields.is_empty());
    let error = card(
        "edit",
        args.clone(),
        Some("old_string not found in f.rs"),
        true,
    );
    assert_eq!(
        (error.lines.len(), error.text.as_str()),
        (5, "old_string not found in f.rs")
    );
    assert_eq!(chips(&error), counts);
    let old = card("edit", args, Some("Replaced 1 occurrence in f.rs"), false);
    assert_eq!((old.lines[0].old, old.lines[0].new), (None, None));
    assert_eq!(chips(&old), counts);
}

#[test]
fn edit_counts_of_zero_get_no_chip() {
    let insert = json!({"path": "f.rs", "old_string": "a\n", "new_string": "a\nb\n"});
    let out = "Replaced 1 occurrence in f.rs at line 4";
    let only_added = card("edit", insert, Some(out), false);
    assert_eq!(
        chips(&only_added),
        [("+1", Tone::Success), ("line 4", Tone::Plain)]
    );
    let delete = json!({"path": "f.rs", "old_string": "a\nb\n", "new_string": "a\n"});
    let only_removed = card("edit", delete, Some(out), false);
    assert_eq!(
        chips(&only_removed),
        [("\u{2212}1", Tone::Error), ("line 4", Tone::Plain)]
    );
}

#[test]
fn edit_with_replace_all_names_every_line() {
    let args = json!({"path": "f.rs", "old_string": "x", "new_string": "y", "replace_all": true});
    let out = "Replaced 12 occurrences in f.rs at lines 1, 2, 3, 4, 5, 6, 7, 8, 9, 10, ...";
    let all = card("edit", args.clone(), Some(out), false);
    assert_eq!(fields(&all), [("replace all", "yes")]);
    let lines = ("lines 1, 2, 3, 4, 5, 6, 7, 8, 9, 10, ...", Tone::Plain);
    assert_eq!(chips(&all)[2..], [lines, ("12 replacements", Tone::Info)]);
    let old = card("edit", args, Some("Replaced 2 occurrences in f.rs"), false);
    assert_eq!(chips(&old)[2..], [("2 replacements", Tone::Info)]);
}

#[test]
fn grep_groups_matches_by_file() {
    let args = json!({"pattern": "fn x", "path": "src", "glob": "*.rs", "ignore_case": true, "max_results": 3});
    let out = "F:\\a\\b.rs:12: fn x\nF:\\a\\b.rs:20: fn x2\nsrc/c.rs:3: x: fn x\n\
        [results cut at 3 matches; narrow the pattern, path or glob, or raise max_results]\n";
    let found = card("grep", args.clone(), Some(out), false);
    assert_eq!(found.target, "fn x");
    let want = [
        line(LineKind::Header, None, None, "F:\\a\\b.rs"),
        line(LineKind::Context, None, Some(12), "fn x"),
        line(LineKind::Context, None, Some(20), "fn x2"),
        line(LineKind::Header, None, None, "src/c.rs"),
        line(LineKind::Context, None, Some(3), "x: fn x"),
    ];
    assert_eq!(found.lines, want);
    let cut = ("cut at 3", Tone::Warning);
    assert_eq!(chips(&found), [("3 matches in 2 files", Tone::Info), cut]);
    let want = [
        ("path", "src"),
        ("glob", "*.rs"),
        ("ignore case", "yes"),
        ("max results", "3"),
    ];
    assert_eq!(fields(&found), want);
    let running = card("grep", json!({"pattern": "x"}), None, false);
    assert_eq!(
        (fields(&running), running.chips.len()),
        (vec![("path", ".")], 0)
    );
    let none = card("grep", json!({"pattern": "x"}), Some("No matches."), false);
    assert_eq!(chips(&none), [("no matches", Tone::Plain)]);
    let one = card("grep", json!({"pattern": "x"}), Some("a.rs:1: x\n"), false);
    assert_eq!(chips(&one), [("1 match in 1 file", Tone::Info)]);
}

#[test]
fn glob_lists_files_and_what_it_hid() {
    let out = "a.rs\nb.rs\n[3 more files not shown; narrow the pattern or path]\n";
    let cut = card("glob", json!({"pattern": "**/*.rs"}), Some(out), false);
    assert_eq!(
        (cut.target.as_str(), fields(&cut)),
        ("**/*.rs", vec![("path", ".")])
    );
    assert_eq!(cut.lines[1], line(LineKind::Context, None, None, "b.rs"));
    assert_eq!((cut.lines.len(), cut.lines_total), (2, 5));
    assert_eq!(chips(&cut), [("showing 2 of 5", Tone::Warning)]);
    let none = card(
        "glob",
        json!({"pattern": "x", "path": "src"}),
        Some("No files found."),
        false,
    );
    assert_eq!(
        (chips(&none), fields(&none)),
        (vec![("no files", Tone::Plain)], vec![("path", "src")])
    );
    let one = card("glob", json!({"pattern": "x"}), Some("a.rs\n"), false);
    assert_eq!(chips(&one), [("1 file", Tone::Info)]);
}
#[test]
fn a_long_shell_command_gets_its_field() {
    let long = "Remove-Item -Recurse -Force F:\\work\\out\\fh-shots\\old-run";
    let card_long = card("shell", json!({"command": long}), None, false);
    assert_eq!(card_long.target, long);
    assert_eq!(fields(&card_long), [("command", long)]);
    let edge = "x".repeat(48);
    let card_edge = card("shell", json!({"command": edge}), None, false);
    assert!(card_edge.fields.is_empty());
}

#[test]
fn shell_reads_its_bracket_lines() {
    let args = json!({"command": "\ncargo test\necho done", "timeout_ms": 5000});
    let ok = card("shell", args, Some("ok\n[exit code: 0]"), false);
    assert_eq!(ok.target, "cargo test");
    let want = [
        ("command", "\ncargo test\necho done"),
        ("timeout", "5000 ms"),
    ];
    assert_eq!(fields(&ok), want);
    assert_eq!(
        (ok.text.as_str(), chips(&ok)),
        ("ok", vec![("exit 0", Tone::Success)])
    );
    let args = json!({"command": "cargo build"});
    let failed = card("shell", args.clone(), Some("boom\n[exit code: 101]"), true);
    assert!(failed.fields.is_empty());
    assert_eq!(
        (failed.text.as_str(), chips(&failed)),
        ("boom", vec![("exit 101", Tone::Error)])
    );
    let out = "partial\n[100 more bytes of output dropped]\n\
        [timed out after 5000 ms (timeout_ms=5000); process tree killed]\n[exit code: none]";
    let slow = card("shell", args.clone(), Some(out), true);
    let want = [
        ("output cut", Tone::Warning),
        ("timed out", Tone::Error),
        ("no exit code", Tone::Warning),
    ];
    assert_eq!(
        (slow.text.as_str(), chips(&slow)),
        ("partial", want.to_vec())
    );
    let out = "(no output)\n[cancelled; process tree killed]\n[exit code: none]";
    let stopped = card("shell", args.clone(), Some(out), true);
    let want = [
        ("no output", Tone::Plain),
        ("cancelled", Tone::Warning),
        ("no exit code", Tone::Warning),
    ];
    assert_eq!(
        (stopped.text.as_str(), chips(&stopped)),
        ("", want.to_vec())
    );
}

#[test]
fn subagent_shows_the_answer_and_the_session_line() {
    let args = json!({"prompt": "Find the bug\nin x", "description": "Bug hunt"});
    let out = "The answer.\n\n[subagent session s1 · model m · 3 steps · tools read, grep · in 120 out 45]";
    let done = card("subagent", args, Some(out), false);
    assert_eq!(done.target, "Bug hunt");
    assert_eq!(fields(&done), [("prompt", "Find the bug\nin x")]);
    let want = [
        ("session s1", Tone::Plain),
        ("model m", Tone::Info),
        ("3 steps", Tone::Plain),
        ("tools read, grep", Tone::Plain),
        ("in 120 out 45", Tone::Plain),
    ];
    assert_eq!(
        (done.text.as_str(), chips(&done)),
        ("The answer.", want.to_vec())
    );
    let args = json!({"prompt": "\nFind the bug\nin x"});
    let out = "partial\n\nsubagent ended Error: boom\n\n[subagent session s2]";
    let failed = card("subagent", args, Some(out), true);
    assert_eq!(failed.target, "Find the bug");
    let want = [("session s2", Tone::Plain), ("ended Error", Tone::Error)];
    assert_eq!(
        (failed.text.as_str(), chips(&failed)),
        ("partial", want.to_vec())
    );
}

#[test]
fn find_names_what_it_loaded() {
    let out = "Loaded: read, edit\nYou can call them now.";
    let found = card(
        "find_tools",
        json!({"names": ["read", "edit"]}),
        Some(out),
        false,
    );
    assert_eq!(
        (found.target.as_str(), found.text.as_str()),
        ("read, edit", out)
    );
    assert_eq!(chips(&found), [("loaded 2 tools", Tone::Info)]);
    let out = "No tool matched. Tools: read, edit.";
    let none = card(
        "find_tools",
        json!({"query": "search files"}),
        Some(out),
        false,
    );
    assert_eq!(none.target, "\"search files\"");
    assert_eq!(chips(&none), [("no match", Tone::Warning)]);
}

#[test]
fn other_tools_list_their_arguments() {
    let args = json!({"b": 1, "a": "text", "c": {"d": [1, 2]}});
    let other = card("mcp_thing", args.clone(), Some("done"), false);
    assert_eq!(other.kind, ToolKind::Other);
    assert_eq!(other.target, tool_summary("mcp_thing", &args.to_string()));
    let want = [("a", "text"), ("b", "1"), ("c", "{\"d\":[1,2]}")];
    assert_eq!(
        (fields(&other), other.text.as_str()),
        (want.to_vec(), "done")
    );
}

#[test]
fn streaming_arguments_give_only_a_summary() {
    let partial = "{\"path\": \"src/ma";
    let streaming = tool_detail("read", partial, None, false, Some(40));
    assert_eq!(streaming.target, tool_summary("read", partial));
    assert!(streaming.chips.is_empty() && streaming.fields.is_empty());
    assert!(streaming.lines.is_empty() && streaming.text.is_empty());
    let failed = tool_detail("read", partial, Some("bad arguments"), true, None);
    assert_eq!(failed.text, "bad arguments");
}

#[test]
fn lines_stop_at_the_limit_and_count_the_rest() {
    let content: String = (1..=250).map(|i| format!("l{i}\n")).collect();
    let big = card(
        "write",
        json!({"path": "a", "content": content}),
        None,
        false,
    );
    assert_eq!((big.lines.len(), big.lines_total), (MAX_LINES, 250));
    assert_eq!(big.lines[MAX_LINES - 1].new, Some(200));
    let out: String = (1..=300).map(|i| format!("{i}: x\n")).collect();
    let read = card("read", json!({"path": "a"}), Some(&out), false);
    assert_eq!((read.lines.len(), read.lines_total), (MAX_LINES, 300));
    assert_eq!(chips(&read), [("all 300 lines", Tone::Info)]);
}

fn kinds(diff: &[(LineKind, String)]) -> String {
    let mark = |kind: &LineKind| match kind {
        LineKind::Added => '+',
        LineKind::Removed => '-',
        _ => ' ',
    };
    diff.iter()
        .map(|(kind, text)| format!("{}{text}", mark(kind)))
        .collect::<Vec<_>>()
        .join("|")
}

#[test]
fn line_diff_keeps_common_lines() {
    let change = line_diff("a\nb\nc\nd\ne", "a\nx\nc\ny\ne");
    assert_eq!(kinds(&change), " a|-b|+x| c|-d|+y| e");
    assert_eq!(kinds(&line_diff("a\nc", "a\nb\nc")), " a|+b| c");
    assert_eq!(kinds(&line_diff("a\nb\nc", "a\nc")), " a|-b| c");
    assert_eq!(kinds(&line_diff("a\nb", "a\nb\n")), " a| b");
    assert_eq!(kinds(&line_diff("", "")), "");
    assert_eq!(kinds(&line_diff("", "n")), "+n");
}

#[test]
fn line_diff_falls_back_on_a_large_middle() {
    let side = |tag: &str| {
        let mid: Vec<String> = (0..2001).map(|i| format!("{tag}{i}")).collect();
        format!("top\n{}\nsame\nbottom", mid.join("\n"))
    };
    let mut old = side("o");
    old = old.replacen("o1000\n", "keep\n", 1);
    let new = side("n").replacen("n1000\n", "keep\n", 1);
    let diff = line_diff(&old, &new);
    assert_eq!(diff.len(), 1 + 2001 * 2 + 2);
    assert_eq!(diff[0], (LineKind::Context, "top".to_string()));
    assert!(diff[1..2002]
        .iter()
        .all(|(kind, _)| *kind == LineKind::Removed));
    assert!(diff[2002..4003]
        .iter()
        .all(|(kind, _)| *kind == LineKind::Added));
    let tail: Vec<&str> = diff[4003..].iter().map(|(_, text)| text.as_str()).collect();
    assert_eq!(tail, ["same", "bottom"]);
}
