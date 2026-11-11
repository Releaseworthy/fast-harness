//! The chat box pickers and the trace timeline.

use fast_harness_core::llm::anthropic::efforts;
use fast_harness_core::trace::detail::{self, Entry, EntryKind};
use fast_harness_core::trace::TraceEvent;
use fast_harness_core::types::Message;
use fast_harness_core::view::Transcript;
use ratatui::backend::TestBackend;
use ratatui::crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use ratatui::Terminal;
use serde_json::json;

use super::draw::{self, View};
use super::picker::{next_effort, PickerKind};
use super::theme::Styles;
use super::timeline::{self, TraceView};
use super::transcript::Cache;
use super::ui::{Action, Screen, Ui};
use super::wrap::text_width;

fn press(ui: &mut Ui, code: KeyCode, modifiers: KeyModifiers) -> Action {
    ui.key(KeyEvent::new(code, modifiers))
}

fn ctrl(ui: &mut Ui, letter: char) -> Action {
    press(ui, KeyCode::Char(letter), KeyModifiers::CONTROL)
}

fn key(ui: &mut Ui, code: KeyCode) -> Action {
    press(ui, code, KeyModifiers::NONE)
}

fn screen(ui: &mut Ui, width: u16, height: u16) -> Vec<String> {
    let transcript = Transcript::new();
    let mut cache = Cache::default();
    let mut view = View {
        transcript: &transcript,
        cache: &mut cache,
        session: "s-1",
    };
    let mut terminal = Terminal::new(TestBackend::new(width, height)).unwrap();
    terminal
        .draw(|frame| draw::draw(frame, ui, &mut view, 0))
        .unwrap();
    let buffer = terminal.backend().buffer();
    let cells: Vec<&str> = buffer.content().iter().map(|c| c.symbol()).collect();
    cells
        .chunks(usize::from(width))
        .map(|row| row.concat())
        .collect()
}

fn entry(kind: EntryKind, title: &str, body: &str) -> Entry {
    Entry {
        kind,
        turn: 1,
        step: 1,
        title: title.into(),
        body: body.into(),
        is_error: false,
    }
}

/// One turn with every entry kind; the thinking body has 12 lines.
pub fn sample_entries() -> Vec<Entry> {
    let thinking: Vec<String> = (1..=12).map(|i| format!("thought {i}")).collect();
    let mut failed = entry(EntryKind::ToolResult, "Result read", "no such file");
    failed.is_error = true;
    vec![
        entry(EntryKind::Turn, "Turn 1", ""),
        entry(EntryKind::User, "User", "fix the bug please"),
        entry(EntryKind::Header, "Request header", "SYSTEM PROMPT TEXT"),
        entry(EntryKind::Request, "Request 3 messages", ""),
        entry(EntryKind::Thinking, "Thinking", &thinking.join("\n")),
        entry(EntryKind::Response, "Response", "Here is the fix."),
        entry(EntryKind::ToolCall, "Tool read", "{\"path\":\"a.rs\"}"),
        failed,
        entry(EntryKind::Answer, "Answer end_turn", ""),
        entry(EntryKind::Retry, "Retry 1 in 500 ms", "overloaded"),
        entry(EntryKind::Failure, "Failure", "network down"),
        entry(EntryKind::Switch, "Account switch a → b", ""),
        entry(EntryKind::TurnEnd, "Turn end", ""),
    ]
}

fn texts(lines: &[ratatui::text::Line<'_>]) -> Vec<String> {
    lines.iter().map(|l| l.to_string()).collect()
}

#[test]
fn ctrl_p_picks_a_model_and_fits_the_effort() {
    let mut ui = Ui::new("claude-opus-5-5".into(), Some("max".into()));
    assert_eq!(ctrl(&mut ui, 'p'), Action::FetchModels);
    let picker = ui.picker.as_ref().unwrap();
    assert_eq!(picker.kind, PickerKind::Model);
    assert_eq!(picker.items[picker.selected].0, "claude-opus-5-5");
    let rows = screen(&mut ui, 100, 30);
    assert!(rows
        .iter()
        .any(|r| r.contains("› Claude Opus 5.5") && r.contains('✓')));
    assert!(rows.iter().any(|r| r.contains("Enter pick")));
    // Other keys are ignored while the picker is open.
    assert_eq!(key(&mut ui, KeyCode::Char('x')), Action::None);
    assert!(ui.input.is_empty());
    key(&mut ui, KeyCode::End);
    key(&mut ui, KeyCode::Home);
    key(&mut ui, KeyCode::PageDown);
    key(&mut ui, KeyCode::PageUp);
    let index = ui.picker.as_ref().unwrap();
    assert_eq!(index.items[0], (String::new(), "Claude".to_string()));
    // Home and PageUp land past the "Claude" title row, on row 1.
    assert_eq!(index.selected, 1);
    let target = index.items.iter().position(|i| i.0 == "claude-opus-4-5");
    for _ in 1..target.unwrap() {
        key(&mut ui, KeyCode::Down);
    }
    let picked = key(&mut ui, KeyCode::Enter);
    assert_eq!(picked, Action::SetModel("claude-opus-4-5".into()));
    assert!(ui.picker.is_none());
    ui.set_model("claude-opus-4-5".into());
    assert_eq!(ui.effort.as_deref(), Some("high"));
    assert!(
        ui.status.contains("Claude Opus 4.5 · effort high"),
        "{}",
        ui.status
    );
    let rows = screen(&mut ui, 100, 30);
    assert!(rows
        .iter()
        .any(|r| r.contains(" Claude Opus 4.5 · effort high ")));
}

#[test]
fn ctrl_e_lists_the_efforts_of_the_model_and_esc_closes() {
    let mut ui = Ui::new("claude-opus-4-5".into(), Some("high".into()));
    ctrl(&mut ui, 'e');
    let picker = ui.picker.as_ref().unwrap();
    let items: Vec<&str> = picker.items.iter().map(|i| i.0.as_str()).collect();
    assert_eq!(items, ["low", "medium", "high", "off"]);
    assert_eq!(picker.selected, 2);
    key(&mut ui, KeyCode::Up);
    assert_eq!(
        key(&mut ui, KeyCode::Enter),
        Action::SetEffort("medium".into())
    );
    ui.model = "claude-opus-5-5".into();
    ctrl(&mut ui, 'e');
    let items = ui.picker.as_ref().unwrap().items.len();
    assert_eq!(items, efforts("claude-opus-5-5").len());
    assert_eq!(key(&mut ui, KeyCode::Esc), Action::None);
    assert!(ui.picker.is_none());
    assert_eq!(ui.screen, Screen::Chat);
    assert_eq!(
        next_effort("claude-opus-4-5", Some("off")).as_deref(),
        Some("low")
    );
    assert_eq!(
        next_effort("claude-opus-5-5", Some("high")).as_deref(),
        Some("xhigh")
    );
    ui.running = true;
    ctrl(&mut ui, 'p');
    assert!(ui.picker.is_some(), "the picker opens while a turn runs");
}

#[test]
fn timeline_titles_and_cut_bodies() {
    let st = Styles::default();
    let lines = texts(&timeline::lines(&sample_entries(), false, 60, &st));
    assert!(lines[0].starts_with("── Turn 1 ─"));
    assert_eq!(lines[1], "  › User");
    assert_eq!(lines[2], "    fix the bug please");
    assert!(!lines.iter().any(|l| l.contains("SYSTEM PROMPT")));
    assert!(lines.contains(&"  → Request 3 messages".to_string()));
    assert!(lines.contains(&"    thought 8".to_string()));
    assert!(!lines.iter().any(|l| l.contains("thought 9")));
    assert!(lines.contains(&"    … 4 more lines (f shows all)".to_string()));
    for title in [
        "  ✗ Result read",
        "  ← Answer",
        "  ↻ Retry",
        "  ✗ Failure",
        "  ⇄ Account",
    ] {
        assert!(lines.iter().any(|l| l.starts_with(title)), "{title}");
    }
    let full = texts(&timeline::lines(&sample_entries(), true, 60, &st));
    assert!(full.iter().any(|l| l.contains("SYSTEM PROMPT TEXT")));
    assert!(full.contains(&"    thought 12".to_string()));
    let narrow = texts(&timeline::lines(&sample_entries(), false, 12, &st));
    assert!(narrow.iter().all(|l| text_width(l) <= 12));
}

#[test]
fn trace_screen_toggles_full_and_steps() {
    let mut ui = Ui::new("m".into(), None);
    ui.screen = Screen::Trace;
    let mut view = TraceView::default();
    view.set(sample_entries(), vec!["turn step model".into()], None);
    ui.trace = view;
    let rows = screen(&mut ui, 80, 20);
    assert!(rows.iter().any(|r| r.contains("bodies cut to 8 lines")));
    assert!(rows.iter().any(|r| r.contains("f full bodies")));
    key(&mut ui, KeyCode::Char('f'));
    let rows = screen(&mut ui, 80, 20);
    assert!(rows.iter().any(|r| r.contains("SYSTEM PROMPT TEXT")));
    assert!(rows.iter().any(|r| r.contains("full bodies")));
    key(&mut ui, KeyCode::Char('s'));
    let rows = screen(&mut ui, 80, 20);
    assert!(rows.iter().any(|r| r.contains("turn step model")));
    assert!(!rows.iter().any(|r| r.contains("SYSTEM PROMPT")));
    key(&mut ui, KeyCode::Char('s'));
    screen(&mut ui, 80, 20);
    key(&mut ui, KeyCode::End);
    assert!(ui.trace_top > 0);
    key(&mut ui, KeyCode::Home);
    assert_eq!(ui.trace_top, 0);
    key(&mut ui, KeyCode::Esc);
    assert_eq!(ui.screen, Screen::Chat);
}

#[test]
fn the_timeline_of_a_session_log_shows_what_the_user_sent() {
    let event = |seq, kind: &str, data| TraceEvent {
        kind: kind.into(),
        seq,
        time: 0,
        data,
    };
    let user = serde_json::to_value(Message::user("hello there")).unwrap();
    let events = [
        event(1, "turn/start", json!({"turn": 1})),
        event(2, "user/message", json!({"turn": 1, "message": user})),
    ];
    let st = Styles::default();
    let lines = texts(&timeline::lines(&detail::entries(&events), false, 40, &st));
    assert!(lines[0].starts_with("── Turn 1"));
    assert_eq!(lines[1..], ["  › User", "    hello there"]);
}
