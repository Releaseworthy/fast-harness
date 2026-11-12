use fast_harness_core::trace::detail::{Entry, EntryKind, EntryTool};
use fast_harness_core::view::{tool_detail, ToolKind};
use slint::Model;

use super::*;
use crate::trace_rows::entry_row;
use crate::{CodeLine, ToolView};

fn view(name: &str, args: &str, output: Option<&str>, error: bool) -> ToolView {
    tool_view(name, &tool_detail(name, args, output, error, Some(41)))
}

fn chips(view: &ToolView) -> Vec<String> {
    view.chip_rows
        .iter()
        .flat_map(|row| row.chips.iter().collect::<Vec<_>>())
        .map(|c| format!("{}:{}", c.tone, c.text))
        .collect()
}

fn kinds(view: &ToolView) -> Vec<String> {
    view.lines.iter().map(|l| l.kind.to_string()).collect()
}

fn numbered(n: usize) -> String {
    (1..=n)
        .map(|i| format!("{i}: line {i}"))
        .collect::<Vec<_>>()
        .join("\n")
}

#[test]
fn read_splits_the_path_and_numbers_its_lines() {
    let out = "10: a\n11: b\n12: c\n[showing lines 10-12 of 40; read more with offset]";
    let v = view(
        "read",
        r#"{"path":"crates/x/src/lib.rs","offset":10,"limit":3}"#,
        Some(out),
        false,
    );
    assert_eq!((v.kind.as_str(), v.name.as_str()), ("read", "read"));
    assert_eq!(
        (v.target.as_str(), v.target_dir.as_str()),
        ("lib.rs", "crates/x/src")
    );
    assert!(chips(&v).contains(&"info:lines 10\u{2013}12 of 40".to_string()));
    assert_eq!(v.lines.row_count(), 3);
    assert_eq!(
        v.lines.row_data(0).map(|l| l.new.to_string()),
        Some("10".into())
    );
    assert_eq!(v.digits, 2);
    assert!(!v.two_numbers && !v.more);
}

#[test]
fn write_shows_the_new_file_and_its_size() {
    let args = r#"{"path":"notes.md","content":"one\ntwo\nthree"}"#;
    let v = view("write", args, Some("Wrote notes.md (new file)"), false);
    let c = chips(&v);
    assert!(c.contains(&"info:new file".to_string()), "{c:?}");
    assert!(c.contains(&"success:+3 lines".to_string()), "{c:?}");
    assert_eq!(kinds(&v), ["added", "added", "added"]);
    assert_eq!(v.target_dir.as_str(), "");
}

#[test]
fn edit_shows_the_diff_with_two_numbers() {
    let args = r#"{"path":"src/a.rs","old_string":"a\nb","new_string":"a\nc"}"#;
    let v = view(
        "edit",
        args,
        Some("Replaced 1 occurrence in src/a.rs at line 7"),
        false,
    );
    assert!(v.two_numbers);
    assert!(kinds(&v).contains(&"removed".to_string()));
    assert!(kinds(&v).contains(&"added".to_string()));
    let c = chips(&v);
    assert!(
        c.contains(&"success:+1".to_string()) && c.contains(&"error:\u{2212}1".to_string()),
        "{c:?}"
    );
    let first = v.lines.row_data(0).expect("a line");
    assert_eq!((first.old.as_str(), first.new.as_str()), ("7", "7"));
}

#[test]
fn grep_groups_matches_under_file_headers() {
    let out = "src/a.rs:3: fn x()\nsrc/b.rs:9: fn y()";
    let v = view(
        "grep",
        r#"{"pattern":"fn \\w+","path":"src"}"#,
        Some(out),
        false,
    );
    assert_eq!(kinds(&v), ["header", "context", "header", "context"]);
    assert!(chips(&v).contains(&"info:2 matches in 2 files".to_string()));
    assert_eq!(v.target.as_str(), "fn \\w+");
    assert_eq!(
        v.fields.row_data(0).map(|f| f.label.to_string()),
        Some("path".into())
    );
}

#[test]
fn glob_lists_files_without_numbers_and_shell_keeps_its_text() {
    let v = view(
        "glob",
        r#"{"pattern":"**/*.rs"}"#,
        Some("a.rs\nsrc/ui/b.rs"),
        false,
    );
    assert_eq!((v.lines.row_count(), v.digits), (2, 0));
    let row = |i: usize| v.lines.row_data(i).expect("a file row");
    let parts = |l: CodeLine| (l.kind.to_string(), l.old.to_string(), l.text.to_string());
    assert_eq!(parts(row(0)), ("file".into(), "".into(), "a.rs".into()));
    assert_eq!(
        parts(row(1)),
        ("file".into(), "src/ui/".into(), "b.rs".into())
    );
    let many: Vec<String> = (0..14).map(|i| format!("d/f{i}.rs")).collect();
    let v = view("glob", "{}", Some(&many.join("\n")), false);
    assert_eq!(v.more_label.as_str(), "Show all 14 files");
    let v = view(
        "shell",
        r#"{"command":"cargo test"}"#,
        Some("ok\n[exit code: 0]"),
        false,
    );
    assert_eq!(v.target.as_str(), "$ cargo test");
    assert_eq!(v.text.as_str(), "ok");
    assert!(chips(&v).contains(&"success:exit 0".to_string()));
    assert!(
        chips(&v).contains(&"plain:41 ms".to_string()),
        "{:?}",
        chips(&v)
    );
}

#[test]
fn a_long_shell_command_shows_whole_in_its_field() {
    use slint::Model as _;
    let command = "Remove-Item -Recurse -Force F:\\work\\out\\fh-shots\\an-old-run";
    let args = serde_json::json!({ "command": command }).to_string();
    let v = view("shell", &args, None, false);
    let field = v.fields.row_data(0).expect("the command field");
    assert_eq!(field.label.as_str(), "command");
    assert_eq!(
        (field.value.as_str(), field.preview.as_str()),
        (command, command)
    );
}

#[test]
fn a_running_tool_has_its_arguments_only() {
    let v = tool_view(
        "grep",
        &tool_detail("grep", r#"{"pattern":"x"}"#, None, false, None),
    );
    assert_eq!(v.chip_rows.row_count(), 0);
    assert_eq!(
        (v.lines.row_count(), v.text.as_str(), v.more),
        (0, "", false)
    );
}

#[test]
fn an_error_shows_the_message_and_the_edit_chips() {
    let args = r#"{"path":"a.rs","old_string":"x","new_string":"y"}"#;
    let v = view("edit", args, Some("old_string not found in a.rs"), true);
    assert_eq!(v.text.as_str(), "old_string not found in a.rs");
    assert!(chips(&v).contains(&"success:+1".to_string()));
}

#[test]
fn long_output_is_previewed_and_cut_lines_are_noted() {
    let v = view("read", r#"{"path":"a.rs"}"#, Some(&numbered(25)), false);
    assert_eq!((v.lines.row_count(), v.lines_preview.row_count()), (25, 10));
    assert!(v.more);
    assert_eq!(v.more_label.as_str(), "Show all 25 lines");
    assert_eq!(v.lines_note.as_str(), "");
    let v = view("read", r#"{"path":"a.rs"}"#, Some(&numbered(250)), false);
    assert_eq!(v.lines.row_count(), 200);
    assert_eq!(
        v.lines_note.as_str(),
        "50 more lines not shown; Copy takes all"
    );
    let text = numbered(30);
    let v = view("shell", r#"{"command":"ls"}"#, Some(&text), false);
    assert_eq!(v.text_preview.lines().count(), 10);
    assert_eq!(v.more_label.as_str(), "Show all 30 lines");
}

#[test]
fn paths_fields_lines_and_chips_are_cut_for_drawing() {
    let split = |t: &str| split_target(ToolKind::Read, t);
    assert_eq!(split("C:\\x\\y.rs"), ("y.rs".into(), "C:\\x".into()));
    assert_eq!(split("/y.rs"), ("y.rs".into(), "/".into()));
    assert_eq!(split("y.rs"), ("y.rs".into(), String::new()));
    assert_eq!(
        cut(&"x".repeat(500), LINE_CHARS).chars().count(),
        LINE_CHARS + 1
    );
    assert_eq!(cut("a\tb", 10), "a    b");
    let prompt = "one\ntwo\nthree\nfour\nfive";
    let v = view(
        "subagent",
        &format!(r#"{{"prompt":{prompt:?}}}"#),
        None,
        false,
    );
    let f = v.fields.row_data(0).expect("the prompt field");
    assert_eq!(f.preview.as_str(), "one\ntwo\nthree …");
    assert_eq!(f.value.as_str(), prompt);
    assert_eq!((v.more, v.more_label.as_str()), (true, "Show all"));
    let many: Vec<_> = (0..12)
        .map(|i| fast_harness_core::view::Chip {
            text: format!("chip number {i}"),
            tone: fast_harness_core::view::Tone::Plain,
        })
        .collect();
    assert!(chip_rows(&many, CHIP_ROW_CHARS).len() >= 3);
}

#[test]
fn a_wide_card_fits_the_subagent_chips_on_one_line() {
    let out = "Done.\n[subagent session 3f9c · model gemini-3-flash · 12 steps · 9 tool calls · 48.2k tokens]";
    let v = view("subagent", r#"{"prompt":"go"}"#, Some(out), false);
    assert_eq!(v.chip_rows.row_count(), 2);
    assert_eq!(v.chip_rows_wide.row_count(), 1);
    assert_eq!(chips(&v).len(), 6);
}

#[test]
fn a_trace_result_has_the_duration_chip() {
    let result = entry_row(&tool_entry(EntryKind::ToolResult, Some("ok")));
    assert!(chips(&result.tool).contains(&"plain:3 ms".to_string()));
}

fn tool_entry(kind: EntryKind, output: Option<&str>) -> Entry {
    Entry {
        kind,
        turn: 1,
        step: 1,
        title: "Tool edit".into(),
        body: "{}".into(),
        is_error: false,
        tool: Some(EntryTool {
            name: "edit".into(),
            arguments: r#"{"path":"src/a.rs","old_string":"a","new_string":"b"}"#.into(),
            output: output.map(str::to_string),
            duration_ms: Some(3),
        }),
    }
}

#[test]
fn trace_calls_show_the_diff_and_results_the_message() {
    let call = entry_row(&tool_entry(EntryKind::ToolCall, None));
    assert_eq!(call.tool.kind.as_str(), "edit");
    assert_eq!(kinds(&call.tool), ["removed", "added"]);
    let done = "Replaced 1 occurrence in src/a.rs at line 4";
    let result = entry_row(&tool_entry(EntryKind::ToolResult, Some(done)));
    assert_eq!(result.tool.lines.row_count(), 0);
    assert_eq!(result.tool.text.as_str(), done);
    assert_eq!(result.tool.fields.row_count(), 0);
    let mut plain = tool_entry(EntryKind::Response, None);
    plain.tool = None;
    assert_eq!(entry_row(&plain).tool.kind.as_str(), "");
}
