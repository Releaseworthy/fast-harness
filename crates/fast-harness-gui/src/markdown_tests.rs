use slint::StyledText;

use super::{markdown, prepare};

/// A normal reply: a heading, bold, a link, a list, a code block, a quote,
/// a rule and a generic type outside code (out/shots2/reply-select.png
/// showed all of it raw).
const SAMPLE: &str = "## Plan\n\nThis is **bold** and a [link](https://example.com).\n\n\
- item one\n- item two with `code`\n\n```rust\nfn main() {\n    let v: Vec<u8> = vec![];\n}\n```\n\n\
> a quoted line\n\n---\n\nIt returns Vec<T> here.\nDone\n===\n";

#[test]
fn a_reply_with_headings_lists_and_code_renders_formatted() {
    let styled = markdown(SAMPLE);
    assert_ne!(styled, StyledText::from_plain_text(SAMPLE), "fell back to raw text");
    assert!(StyledText::from_markdown(&prepare(SAMPLE)).is_ok());
}

#[test]
fn each_refused_construct_alone_is_rewritten() {
    for text in [
        "# Title",
        "### Steps ###",
        "```\nlet a = `b`;\n```",
        "> quote",
        "***",
        "Text\n---",
        "![shot](a.png)",
        "a <br> b",
        "Option<String>",
    ] {
        assert!(StyledText::from_markdown(text).is_err(), "Slint takes {text:?} now");
        assert!(StyledText::from_markdown(&prepare(text)).is_ok(), "{text:?}");
    }
}

#[test]
fn code_spans_keep_their_angle_brackets_and_headings_turn_bold() {
    assert_eq!(prepare("use `Vec<T>` <here>"), "use `Vec<T>` \\<here>\n");
    assert_eq!(prepare("## **Plan** ##"), "**Plan**\n");
    assert_eq!(prepare("```\n  x = 1\n```"), "`   x = 1 `\n");
}
