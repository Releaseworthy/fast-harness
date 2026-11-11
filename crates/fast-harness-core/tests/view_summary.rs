use fast_harness_core::view::tool_summary;

#[test]
fn built_in_tools() {
    assert_eq!(
        tool_summary("read", r#"{"path":"src/a.rs","limit":5}"#),
        "src/a.rs"
    );
    assert_eq!(
        tool_summary("write", r#"{"path":"b.txt","content":"x"}"#),
        "b.txt"
    );
    assert_eq!(
        tool_summary("edit", r#"{"path":"c.rs","old_string":"a"}"#),
        "c.rs"
    );
    assert_eq!(
        tool_summary("shell", r#"{"command":"\n  cargo test\n  echo done"}"#),
        "cargo test"
    );
    assert_eq!(
        tool_summary("grep", r#"{"pattern":"fn main","path":"src"}"#),
        "fn main in src"
    );
    assert_eq!(tool_summary("grep", r#"{"pattern":"todo"}"#), "todo");
    assert_eq!(tool_summary("glob", r#"{"pattern":"**/*.rs"}"#), "**/*.rs");
    assert_eq!(
        tool_summary(
            "subagent",
            r#"{"description":"Audit view","prompt":"long"}"#
        ),
        "Audit view"
    );
    assert_eq!(
        tool_summary("subagent", r#"{"prompt":"Look at\nthe   code"}"#),
        "Look at the code"
    );
}

#[test]
fn long_values_are_clipped_to_80_chars() {
    let prompt = "word ".repeat(40);
    let summary = tool_summary("subagent", &format!(r#"{{"prompt":"{prompt}"}}"#));
    assert_eq!(summary.chars().count(), 80);
    let path = "é".repeat(100);
    let summary = tool_summary("read", &format!(r#"{{"path":"{path}"}}"#));
    assert_eq!(summary, "é".repeat(80));
}

#[test]
fn other_tools_and_bad_json_fall_back() {
    assert_eq!(tool_summary("web", r#"{"q": 1}"#), r#"web {"q": 1}"#);
    assert_eq!(tool_summary("read", "{not json"), "read {not json");
    assert_eq!(tool_summary("read", r#"{"path":7}"#), r#"read {"path":7}"#);
    assert_eq!(
        tool_summary("shell", r#"{"command":"  \n "}"#),
        r#"shell {"command":" \n "}"#
    );
    assert_eq!(tool_summary("glob", "[1,2]"), "glob [1,2]");
    assert_eq!(tool_summary("grep", ""), "grep");
    let long = format!("{{\"x\":\"{}\"}}", "a".repeat(200));
    let summary = tool_summary("mcp", &long);
    assert_eq!(summary.chars().count(), "mcp ".len() + 80);
}
