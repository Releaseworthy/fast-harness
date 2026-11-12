//! The two GUI bugs of the owner's report checked on the TUI: a trace body of
//! thousands of lines must scroll without lines drawn over each other, and a
//! transcript scrolled up must stay put while long output arrives.

use fast_harness_core::trace::detail::{Entry, EntryKind};
use fast_harness_core::view::{Item, NoteKind, Transcript};
use ratatui::backend::TestBackend;
use ratatui::crossterm::event::{KeyCode, KeyEvent, KeyModifiers, MouseEvent, MouseEventKind};
use ratatui::Terminal;

use super::draw::{self, View};
use super::timeline::TraceView;
use super::transcript::Cache;
use super::ui::{Screen, Ui};

fn screen(ui: &mut Ui, transcript: &Transcript, width: u16, height: u16) -> Vec<String> {
    let mut cache = Cache::default();
    let mut view = View {
        transcript,
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

/// The numbers of the `row NNNNN` body lines on screen, top to bottom.
fn numbers(rows: &[String]) -> Vec<usize> {
    rows.iter()
        .filter_map(|r| r.split("row ").nth(1))
        .filter_map(|r| r.get(..5)?.parse().ok())
        .collect()
}

#[test]
fn a_huge_trace_body_scrolls_without_overlapping_lines() {
    let body: String = (0..5000).map(|i| format!("row {i:05}\n")).collect();
    let mut ui = Ui::new("m".into(), None);
    ui.screen = Screen::Trace;
    let mut view = TraceView::default();
    let entry = Entry {
        kind: EntryKind::ToolResult,
        turn: 1,
        step: 1,
        title: "bash".into(),
        body,
        is_error: false,
        tool: None,
    };
    view.set(vec![entry], Vec::new(), None);
    ui.trace = view;
    let empty = Transcript::new();
    screen(&mut ui, &empty, 80, 20);
    ui.key(KeyEvent::new(KeyCode::Char('f'), KeyModifiers::NONE));
    for top in [0, 2500, 4990, usize::MAX] {
        ui.trace_top = top;
        ui.trace_scroll(0);
        let shown = numbers(&screen(&mut ui, &empty, 80, 20));
        assert!(shown.len() >= 10, "top {top}: {shown:?}");
        let run: Vec<usize> = (shown[0]..shown[0] + shown.len()).collect();
        assert_eq!(shown, run, "top {top}: each line once, in order");
    }
    let last = numbers(&screen(&mut ui, &empty, 80, 20));
    assert_eq!(last.last(), Some(&4999), "the end is reachable");
}

fn note(i: usize) -> Item {
    Item::Note {
        kind: NoteKind::Info,
        text: format!("note {i}"),
    }
}

#[test]
fn a_scrolled_up_transcript_stays_while_long_output_arrives() {
    let mut ui = Ui::new("m".into(), None);
    let mut t = Transcript::new();
    t.items = (0..50).map(note).collect();
    screen(&mut ui, &t, 40, 12);
    for _ in 0..3 {
        ui.mouse(MouseEvent {
            kind: MouseEventKind::ScrollUp,
            column: 0,
            row: 0,
            modifiers: KeyModifiers::NONE,
        });
    }
    let before = screen(&mut ui, &t, 40, 12);
    let long: String = (0..3000).map(|i| format!("out {i}\n")).collect();
    for _ in 0..5 {
        t.items.push(Item::Assistant {
            turn: 1,
            step: 1,
            text: long.clone(),
            thinking: String::new(),
        });
        let now = screen(&mut ui, &t, 40, 12);
        assert_eq!(now[..8], before[..8], "the view does not move");
    }
    ui.key(KeyEvent::new(KeyCode::End, KeyModifiers::NONE));
    let end = screen(&mut ui, &t, 40, 12);
    assert!(end.iter().any(|r| r.contains("out 2999")), "{end:?}");
}
