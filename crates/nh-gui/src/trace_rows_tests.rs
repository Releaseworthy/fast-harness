use super::*;

fn entry(kind: EntryKind, title: &str, body: &str, is_error: bool) -> Entry {
    Entry {
        kind,
        turn: 1,
        step: 1,
        title: title.into(),
        body: body.into(),
        is_error,
    }
}

#[test]
fn collapse_rule() {
    let eight = ["line"; 8].join("\n");
    let nine = ["line"; 9].join("\n");
    assert!(!is_long(&eight));
    assert!(is_long(&nine));
    assert!(!is_long(&"x".repeat(800)));
    assert!(is_long(&"x".repeat(801)));
    assert!(starts_collapsed(
        EntryKind::Header,
        "You are a coding agent."
    ));
    assert!(!starts_collapsed(EntryKind::Header, ""));
    assert!(!starts_collapsed(EntryKind::Response, "short"));
    assert!(starts_collapsed(EntryKind::ToolResult, &nine));
}

#[test]
fn preview_cuts_lines_and_characters() {
    let nine = (1..=9)
        .map(|i| format!("l{i}"))
        .collect::<Vec<_>>()
        .join("\n");
    let shown = preview(EntryKind::ToolResult, &nine);
    assert!(shown.starts_with("l1\n") && shown.contains("l8"));
    assert!(!shown.contains("l9") && shown.ends_with('…'));
    let wide = "é".repeat(900);
    let shown = preview(EntryKind::Response, &wide);
    assert_eq!(shown.chars().filter(|c| *c == 'é').count(), 800);
    assert_eq!(preview(EntryKind::Header, "system prompt"), "");
    assert_eq!(preview(EntryKind::Response, "a\r\nb"), "a\nb");
}

#[test]
fn entry_rows() {
    let row = entry_row(&entry(EntryKind::User, "User", "hello", false));
    assert_eq!((row.kind.as_str(), row.body.as_str()), ("user", "hello"));
    assert!(!row.long && !row.expanded && !row.error);
    assert_eq!(row.copy, "User\n\nhello");
    let failed = entry_row(&entry(
        EntryKind::ToolResult,
        "Tool error bash",
        "boom",
        true,
    ));
    assert_eq!(failed.kind, "result");
    assert!(failed.error);
    let header = entry_row(&entry(EntryKind::Header, "Request header", "sys", false));
    assert!(header.long && header.preview.is_empty());
    let end = entry_row(&entry(EntryKind::TurnEnd, "Turn 1 done", "", false));
    assert_eq!(
        (end.kind.as_str(), end.copy.as_str()),
        ("end", "Turn 1 done")
    );
    assert!(entry_row(&entry(EntryKind::Failure, "Attempt 1 failed", "", false)).error);
    assert_eq!(kind_word(EntryKind::ToolCall), "call");
    assert_eq!(kind_word(EntryKind::Switch), "switch");
}

#[test]
fn step_cells_follow_the_columns() {
    let step = StepRow {
        turn: 1,
        step: 2,
        tools: vec![("read".into(), 5), ("grep".into(), 7)],
        ..StepRow::default()
    };
    let cells = step_cells(&step);
    assert_eq!(cells.len(), 12);
    assert_eq!(cells[10], "read, grep");
    assert_eq!(cells[1], "2");
}
