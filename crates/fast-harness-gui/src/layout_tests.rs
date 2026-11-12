//! Rows with wrapped text get the height of every wrapped line. Slint 1.18.1
//! measured a repeated plain element (an `if` whose root is a Rectangle or a
//! layout) at its unwrapped width, so a long user message or thinking text
//! got one line and the next trace row drew over it
//! (https://github.com/slint-ui/slint/issues/13655). The views wrap such
//! text in components (`ui/blocks.slint`); the guard below keeps every `if`
//! and `for` that way.

use std::rc::Rc;

use slint::platform::software_renderer::{MinimalSoftwareWindow, RepaintBufferType};
use slint::platform::{Platform, WindowAdapter};
use slint::{ComponentHandle, ModelRc, PhysicalSize, Rgb8Pixel, VecModel};

use crate::rows::markdown;
use crate::{MainWindow, MsgRow, TraceProbe, TraceRow};

const WIDTH: u32 = 1000;
const HEIGHT: u32 = 2000;
/// Wrapped onto many lines at this width; one line is about 17 px.
const MIN_GROWTH: f32 = 60.0;

struct TestPlatform(Rc<MinimalSoftwareWindow>);

impl Platform for TestPlatform {
    fn create_window_adapter(&self) -> Result<Rc<dyn WindowAdapter>, slint::PlatformError> {
        Ok(self.0.clone())
    }
}

fn window() -> (Rc<MinimalSoftwareWindow>, MainWindow) {
    let window = MinimalSoftwareWindow::new(RepaintBufferType::NewBuffer);
    slint::platform::set_platform(Box::new(TestPlatform(window.clone()))).expect("platform");
    window.set_size(PhysicalSize::new(WIDTH, HEIGHT));
    let ui = MainWindow::new().expect("window");
    ui.show().expect("show");
    (window, ui)
}

fn frames(window: &MinimalSoftwareWindow) {
    let mut buffer = vec![Rgb8Pixel { r: 0, g: 0, b: 0 }; (WIDTH * HEIGHT) as usize];
    for _ in 0..3 {
        slint::platform::update_timers_and_animations();
        window.request_redraw();
        window.draw_if_needed(|renderer| {
            renderer.render(&mut buffer, WIDTH as usize);
        });
    }
}

fn long_text() -> String {
    "a long line that wraps in the column ".repeat(60)
}

fn trace_row(kind: &str, body: &str) -> TraceRow {
    TraceRow {
        kind: kind.into(),
        title: "Entry".into(),
        body: body.into(),
        ..TraceRow::default()
    }
}

fn trace_height(ui: &MainWindow, window: &MinimalSoftwareWindow, row: TraceRow) -> f32 {
    let end = trace_row("end", "");
    ui.set_trace_entries(ModelRc::new(VecModel::from(vec![row, end])));
    frames(window);
    ui.global::<TraceProbe>().get_content_height()
}

#[test]
fn trace_rows_grow_with_their_wrapped_text() {
    let (window, ui) = window();
    ui.set_trace_mode("timeline".into());
    ui.set_page("trace".into());
    for kind in ["user", "thinking", "response", "call", "result"] {
        let short = trace_height(&ui, &window, trace_row(kind, "short"));
        let long = trace_height(&ui, &window, trace_row(kind, &long_text()));
        assert!(
            long - short > MIN_GROWTH,
            "a wrapped {kind} body grows its row: {short} px short, {long} px long"
        );
    }
}

/// A shell result through the real mapping, opened: its output wraps.
fn tool_result(body: &str) -> TraceRow {
    use fast_harness_core::trace::detail::{Entry, EntryKind, EntryTool};
    let entry = Entry {
        kind: EntryKind::ToolResult,
        turn: 1,
        step: 1,
        title: "Tool result shell".into(),
        body: body.into(),
        is_error: false,
        tool: Some(EntryTool {
            name: "shell".into(),
            arguments: r#"{"command":"cargo test"}"#.into(),
            output: Some(body.into()),
            duration_ms: None,
        }),
    };
    TraceRow {
        expanded: true,
        ..crate::trace_rows::entry_row(&entry)
    }
}

#[test]
fn trace_tool_cards_grow_with_their_wrapped_text() {
    let (window, ui) = window();
    ui.set_trace_mode("timeline".into());
    ui.set_page("trace".into());
    let short = trace_height(&ui, &window, tool_result("short"));
    let long = trace_height(&ui, &window, tool_result(&long_text()));
    assert!(
        long - short > MIN_GROWTH,
        "a wrapped tool result grows its row: {short} px short, {long} px long"
    );
}
/// A header-only card: the 30 px tile and the 12 px padding above and below.
const HEADER_CARD: f32 = 30.0 + 2.0 * 12.0;

/// A trace call through the real mapping.
fn tool_call(name: &str, arguments: &str) -> TraceRow {
    use fast_harness_core::trace::detail::{Entry, EntryKind, EntryTool};
    let entry = Entry {
        kind: EntryKind::ToolCall,
        turn: 1,
        step: 1,
        title: format!("Tool call {name}"),
        body: arguments.into(),
        is_error: false,
        tool: Some(EntryTool {
            name: name.into(),
            arguments: arguments.into(),
            output: None,
            duration_ms: None,
        }),
    };
    crate::trace_rows::entry_row(&entry)
}

#[test]
fn a_card_with_only_a_header_is_only_that_tall() {
    let (window, ui) = window();
    ui.set_page("chat".into());
    let none = chat_height(&ui, &window, MsgRow::default());
    let mut running = tool("read", r#"{"path":"src/a.rs"}"#, None);
    running.running = true;
    // A tool row's bottom padding is 8 px more than an empty row's.
    let none = none + 8.0;
    let card = chat_height(&ui, &window, running) - none;
    let code = chat_height(&ui, &window, code_lines("short")) - none;
    assert!(card <= HEADER_CARD + 1.0, "a running read is {card} px");
    assert!(
        code > card + 20.0,
        "a code view adds its rows: {card} px, {code} px"
    );
    ui.set_trace_mode("timeline".into());
    ui.set_page("trace".into());
    let none = trace_height(&ui, &window, trace_row("end", ""));
    for (name, args) in [
        ("read", r#"{"path":"src/a.rs"}"#),
        ("shell", r#"{"command":"ls"}"#),
    ] {
        let call = trace_height(&ui, &window, tool_call(name, args)) - none;
        assert!(call <= HEADER_CARD + 1.0, "a {name} call is {call} px");
    }
}

fn chat_height(ui: &MainWindow, window: &MinimalSoftwareWindow, row: MsgRow) -> f32 {
    ui.set_messages(ModelRc::new(VecModel::from(vec![row])));
    frames(window);
    ui.get_transcript_content_height()
}

fn thinking(text: &str) -> MsgRow {
    MsgRow {
        kind: "assistant".into(),
        text: "Answer.".into(),
        body: markdown("Answer."),
        thinking: text.into(),
        expanded: true,
        ..MsgRow::default()
    }
}

/// A tool card through the real mapping, opened.
fn tool(name: &str, arguments: &str, output: Option<&str>) -> MsgRow {
    let item = fast_harness_core::view::Item::Tool {
        id: "t1".into(),
        name: name.into(),
        arguments: arguments.into(),
        summary: String::new(),
        output: output.map(str::to_string),
        is_error: false,
        duration_ms: None,
        live: None,
    };
    MsgRow {
        expanded: true,
        ..crate::rows::msg_row(&item)
    }
}

fn tool_output(text: &str) -> MsgRow {
    tool("shell", r#"{"command":"cargo test"}"#, Some(text))
}

/// Short: one numbered line; long: forty, each one wrapped.
fn code_lines(text: &str) -> MsgRow {
    let count = if text == "short" { 1 } else { 40 };
    let out: Vec<String> = (1..=count).map(|i| format!("{i}: {text}")).collect();
    tool("read", r#"{"path":"src/a.rs"}"#, Some(&out.join("\n")))
}

fn field_value(text: &str) -> MsgRow {
    let args = serde_json::json!({ "prompt": text }).to_string();
    tool("subagent", &args, None)
}

type MakeRow = fn(&str) -> MsgRow;

#[test]
fn chat_rows_grow_with_their_wrapped_text() {
    let (window, ui) = window();
    ui.set_page("chat".into());
    let cases: [(&str, MakeRow); 4] = [
        ("thinking", thinking),
        ("tool output", tool_output),
        ("code lines", code_lines),
        ("field value", field_value),
    ];
    for (name, make) in cases {
        let short = chat_height(&ui, &window, make("short"));
        let long = chat_height(&ui, &window, make(&long_text()));
        assert!(
            long - short > MIN_GROWTH,
            "a wrapped {name} grows its row: {short} px short, {long} px long"
        );
    }
}

/// The root element name of an `if` or `for` line (`if a: Rectangle {` gives
/// `Rectangle`), skipping the colons of the condition.
fn repeated_root(line: &str) -> Option<&str> {
    let text = line.trim_start();
    if !(text.starts_with("if ") || text.starts_with("for ")) {
        return None;
    }
    text.match_indices(':').find_map(|(at, _)| {
        let rest = text[at + 1..].trim_start();
        let name_len = rest
            .find(|c: char| !(c.is_ascii_alphanumeric() || c == '-' || c == '_'))
            .unwrap_or(rest.len());
        let (name, after) = rest.split_at(name_len);
        let opens = after.trim_start().starts_with('{');
        (!name.is_empty() && opens).then_some(name)
    })
}

/// The lines of the block that opens on `lines[0]`, braces inside strings ignored.
fn block<'a>(lines: &[&'a str]) -> Vec<&'a str> {
    let mut depth = 0i32;
    let mut out = Vec::new();
    for line in lines {
        out.push(*line);
        let mut quoted = false;
        for c in line.chars() {
            match c {
                '"' => quoted = !quoted,
                '{' if !quoted => depth += 1,
                '}' if !quoted => depth -= 1,
                _ => {}
            }
        }
        if depth <= 0 {
            break;
        }
    }
    out
}

const PLAIN_ROOTS: [&str; 5] = [
    "Rectangle",
    "HorizontalLayout",
    "VerticalLayout",
    "GridLayout",
    "TouchArea",
];

/// `file:line` of each `if` or `for` whose root is a plain element and that
/// lays out wrapped text (text in a Flickable is not laid out by it).
fn plain_roots_with_wrapped_text(name: &str, source: &str) -> Vec<String> {
    let lines: Vec<&str> = source.lines().collect();
    let mut found = Vec::new();
    for (i, line) in lines.iter().enumerate() {
        let Some(root) = repeated_root(line) else {
            continue;
        };
        let body = block(&lines[i..]);
        let wraps = body.iter().any(|l| is_wrapped(l));
        let scrolled = body.iter().any(|l| l.contains("Flickable"));
        if PLAIN_ROOTS.contains(&root) && wraps && !scrolled {
            found.push(format!("{name}:{}", i + 1));
        }
    }
    found
}

fn is_wrapped(line: &str) -> bool {
    line.contains("wrap: word-wrap") || line.contains("wrap: char-wrap")
}

/// `file:line` of each component that places `@children` and lays out
/// wrapped text: Slint 1.18.1 measured such a component's wrapped text as
/// one line (the tool card, 2026-11-12), so its extra parts are built in.
fn children_next_to_wrapped_text(name: &str, source: &str) -> Vec<String> {
    let lines: Vec<&str> = source.lines().collect();
    let mut found = Vec::new();
    for (i, line) in lines.iter().enumerate() {
        let text = line.trim_start();
        if !(text.starts_with("component ") || text.starts_with("export component ")) {
            continue;
        }
        let body = block(&lines[i..]);
        let children = body.iter().any(|l| l.trim() == "@children");
        if children && body.iter().any(|l| is_wrapped(l)) {
            found.push(format!("{name}:{}", i + 1));
        }
    }
    found
}

/// Every .slint file of the GUI through `check`.
fn scan_ui(check: fn(&str, &str) -> Vec<String>) -> Vec<String> {
    let dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("ui");
    let mut found = Vec::new();
    for entry in std::fs::read_dir(&dir).expect("ui folder") {
        let path = entry.expect("ui entry").path();
        if path.extension().is_some_and(|e| e == "slint") {
            let source = std::fs::read_to_string(&path).expect("slint source");
            let name = path.file_name().unwrap_or_default().to_string_lossy();
            found.extend(check(&name, &source));
        }
    }
    found
}

#[test]
fn the_guard_finds_a_plain_root_with_wrapped_text() {
    let bad = "VerticalLayout {\n    if shown: HorizontalLayout {\n        Text { wrap: word-wrap; }\n    }\n}";
    assert_eq!(plain_roots_with_wrapped_text("x.slint", bad), ["x.slint:2"]);
    let good = "VerticalLayout {\n    if shown: Quote { text: \"{a}\"; }\n    if a ? b : c: Text { wrap: word-wrap; }\n}";
    assert!(plain_roots_with_wrapped_text("x.slint", good).is_empty());
    assert_eq!(
        repeated_root("if root.checked: AccentFill { radius: 4px; }"),
        Some("AccentFill")
    );
    assert_eq!(
        repeated_root("for item[i] in root.rows: MessageView {"),
        Some("MessageView")
    );
}

#[test]
fn wrapped_text_under_if_and_for_sits_in_a_component() {
    let found = scan_ui(plain_roots_with_wrapped_text);
    assert!(
        found.is_empty(),
        "wrap these in a component (ui/blocks.slint): {found:?}"
    );
}

#[test]
fn the_guard_finds_children_next_to_wrapped_text() {
    let bad = "component Line {\n    HorizontalLayout {\n        Text { wrap: word-wrap; }\n        @children\n    }\n}\ncomponent Card {\n    @children\n}";
    assert_eq!(children_next_to_wrapped_text("x.slint", bad), ["x.slint:1"]);
}

#[test]
fn components_with_children_have_no_wrapped_text() {
    let found = scan_ui(children_next_to_wrapped_text);
    assert!(
        found.is_empty(),
        "build the extra parts in instead of @children: {found:?}"
    );
}
