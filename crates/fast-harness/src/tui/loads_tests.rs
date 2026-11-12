//! The Sessions and Trace screens read on workers (D60): the screen opens
//! at once, fills when its `Msg` comes, keeps a key typed meanwhile, and a
//! result for a closed screen changes nothing. The test thread is marked as
//! the UI thread, so a list or a log read on it panics (debug builds).

use std::path::{Path, PathBuf};
use std::sync::mpsc::{self, Receiver};
use std::time::Duration;

use fast_harness_core::agent::Conversation;
use fast_harness_core::app::Runtime;
use fast_harness_core::ui_thread;
use fast_harness_core::view::{Item, NoteKind, Transcript};
use ratatui::backend::TestBackend;
use ratatui::crossterm::event::{Event, KeyCode, KeyEvent, KeyModifiers};
use ratatui::Terminal;

use super::app::{Msg, Tui};
use super::draw::{self, View};
use super::transcript::Cache;
use super::ui::{Action, Screen};

fn root(tag: &str) -> PathBuf {
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| d.subsec_nanos());
    std::env::temp_dir().join(format!("fh-loads-{tag}-{}-{nanos}", std::process::id()))
}

/// A TUI on a home with three sessions in `work`.
fn tui(root: &Path) -> (Tui, Receiver<Msg>) {
    let (home, cwd) = (root.join("home"), root.join("work"));
    std::fs::create_dir_all(&cwd).unwrap();
    for _ in 0..2 {
        Conversation::create(&home, &cwd, None).unwrap();
    }
    let rt = Runtime::load_at(home).unwrap();
    let opened = rt.open_with_events(&cwd, None, false).unwrap();
    let (tx, rx) = mpsc::channel();
    let tui = Tui::new(rt, cwd, opened, tx).unwrap();
    (tui, rx)
}

/// Waits for the next Sessions or Trace message, applying the others.
fn arrive(tui: &mut Tui, rx: &Receiver<Msg>) -> Msg {
    loop {
        let msg = rx
            .recv_timeout(Duration::from_secs(10))
            .expect("a load message");
        if matches!(msg, Msg::Sessions(..) | Msg::Trace(..)) {
            return msg;
        }
        tui.handle(msg);
    }
}

fn screen_text(tui: &mut Tui) -> String {
    let mut terminal = Terminal::new(TestBackend::new(90, 16)).unwrap();
    let mut cache = Cache::default();
    let mut view = View {
        transcript: &tui.transcript,
        cache: &mut cache,
        session: "s-1",
    };
    let ui = &mut tui.ui;
    terminal
        .draw(|frame| draw::draw(frame, ui, &mut view, 0))
        .unwrap();
    let buffer = terminal.backend().buffer();
    buffer.content().iter().map(|c| c.symbol()).collect()
}

fn down() -> Msg {
    Msg::Term(Event::Key(KeyEvent::new(KeyCode::Down, KeyModifiers::NONE)))
}

#[test]
fn the_sessions_screen_opens_before_its_list_and_keeps_a_key_typed_meanwhile() {
    let root = root("sessions");
    let (mut tui, rx) = tui(&root);
    ui_thread::mark(true);
    tui.act(Action::Open(Screen::Sessions));
    assert_eq!(tui.ui.screen, Screen::Sessions, "open at once");
    assert!(screen_text(&mut tui).contains("Loading sessions..."));
    let msg = arrive(&mut tui, &rx);
    tui.handle(msg);
    assert_eq!(tui.ui.sessions.len(), 3);
    assert!(!screen_text(&mut tui).contains("Loading sessions"));
    // Opened again: the last list shows while the new one is read, and a
    // key typed in between moves the selection, kept when the list comes.
    tui.act(Action::Open(Screen::Sessions));
    tui.handle(down());
    assert_eq!(tui.ui.selected, 1);
    let msg = arrive(&mut tui, &rx);
    tui.handle(msg);
    assert_eq!((tui.ui.selected, tui.ui.sessions.len()), (1, 3));
    ui_thread::mark(false);
    let _ = std::fs::remove_dir_all(&root);
}

#[test]
fn a_result_for_a_closed_screen_or_an_older_read_changes_nothing() {
    let root = root("closed");
    let (mut tui, rx) = tui(&root);
    ui_thread::mark(true);
    tui.act(Action::Open(Screen::Sessions));
    tui.ui.screen = Screen::Chat;
    let msg = arrive(&mut tui, &rx);
    tui.handle(msg);
    assert!(tui.ui.sessions.is_empty(), "closed: dropped");
    tui.act(Action::Open(Screen::Trace));
    tui.act(Action::Open(Screen::Trace));
    let first = arrive(&mut tui, &rx);
    let second = arrive(&mut tui, &rx);
    let (Msg::Trace(a, _), Msg::Trace(b, _)) = (&first, &second) else {
        panic!("two trace reads");
    };
    let (older, newer) = if a < b {
        (first, second)
    } else {
        (second, first)
    };
    tui.handle(older);
    assert!(tui.ui.trace.loading, "the older read is dropped");
    tui.handle(newer);
    assert!(!tui.ui.trace.loading);
    ui_thread::mark(false);
    let _ = std::fs::remove_dir_all(&root);
}

#[test]
fn the_trace_screen_shows_loading_until_its_read_comes() {
    let root = root("trace");
    let (mut tui, rx) = tui(&root);
    ui_thread::mark(true);
    tui.act(Action::Open(Screen::Trace));
    assert!(screen_text(&mut tui).contains("Loading the session log..."));
    let msg = arrive(&mut tui, &rx);
    tui.handle(msg);
    assert!(!screen_text(&mut tui).contains("Loading the session log"));
    ui_thread::mark(false);
    let _ = std::fs::remove_dir_all(&root);
}

#[test]
fn an_opened_session_shows_its_damaged_line_note_once() {
    let view = Transcript::opened(&[], &[], vec!["line 3 was damaged".into()]);
    let notes: Vec<&Item> = (view.items.iter())
        .filter(
            |i| matches!(i, Item::Note { kind: NoteKind::Error, text } if text.contains("line 3")),
        )
        .collect();
    assert_eq!(notes.len(), 1);
}
