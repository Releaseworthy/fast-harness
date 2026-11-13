//! Opening a session's log on a worker (D60, gate G10): the UI-thread call
//! returns without parsing, a result that is not the newest is dropped, an
//! open that fails shows its error and keeps the shown session, and no prompt
//! starts while one loads. The test thread is marked as the UI thread, so a
//! log parse here panics in a debug build; the workers' results are handed in
//! by hand, as the channel would deliver them.

use std::path::{Path, PathBuf};
use std::sync::mpsc::{self, Receiver};

use fast_harness_core::agent::Conversation;
use fast_harness_core::app::Runtime;
use fast_harness_core::ui_thread;
use fast_harness_core::view::opened::{self, Opened};
use ratatui::backend::TestBackend;
use ratatui::Terminal;

use super::app::{Msg, Tui};
use super::draw::{self, View};
use super::transcript::Cache;
use super::ui::Action;

fn root(tag: &str) -> PathBuf {
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| d.subsec_nanos());
    std::env::temp_dir().join(format!("fh-open-{tag}-{}-{nanos}", std::process::id()))
}

/// A TUI showing `first`, on a home with three sessions in `work`.
fn tui(root: &Path) -> (Tui, Receiver<Msg>, PathBuf, PathBuf, [String; 3]) {
    let (home, cwd) = (root.join("home"), root.join("work"));
    std::fs::create_dir_all(&cwd).unwrap();
    let id = || Conversation::create(&home, &cwd, None).unwrap().id;
    let ids = [id(), id(), id()];
    let rt = Runtime::load_at(home.clone()).unwrap();
    let opened = rt.open_with_events(&cwd, Some(&ids[0]), false).unwrap();
    let (tx, rx) = mpsc::channel();
    let tui = Tui::new(rt, cwd.clone(), opened, tx).unwrap();
    (tui, rx, home, cwd, ids)
}

/// A worker's open, read with the mark lifted (as its own thread would).
fn open_off_ui(home: &Path, cwd: &Path, id: &str) -> Result<Box<Opened>, String> {
    ui_thread::mark(false);
    let result = opened::load(home, cwd, id).map(Box::new);
    ui_thread::mark(true);
    result
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

fn user_count(tui: &Tui) -> usize {
    (tui.transcript.items.iter())
        .filter(|item| matches!(item, fast_harness_core::view::Item::User { .. }))
        .count()
}

#[test]
fn the_open_returns_at_once_and_nothing_parses_on_the_ui_thread() {
    let root = root("worker");
    let (mut tui, _rx, home, cwd, ids) = tui(&root);
    ui_thread::mark(true);
    // A parse here would panic on the mark, so it can only happen on the
    // worker; the call returns with the shown session and its word.
    tui.act(Action::Resume(ids[1].clone()));
    assert!(tui.ui.loading);
    assert!(screen_text(&mut tui).contains("Loading the session"));
    assert_eq!(
        tui.session, ids[0],
        "the shown session stays until it comes"
    );
    let result = open_off_ui(&home, &cwd, &ids[1]);
    tui.handle(Msg::Opened(1, result));
    assert!(!tui.ui.loading);
    assert_eq!(tui.session, ids[1]);
    assert!(!screen_text(&mut tui).contains("Loading the session"));
    ui_thread::mark(false);
    let _ = std::fs::remove_dir_all(&root);
}

#[test]
fn a_newer_open_supersedes_an_older_and_its_result_is_dropped() {
    let root = root("stale");
    let (mut tui, _rx, home, cwd, ids) = tui(&root);
    ui_thread::mark(true);
    tui.open_session(Some(&ids[1]));
    tui.open_session(Some(&ids[2]));
    assert!(tui.ui.loading, "the newest open still runs");
    // The first worker's result comes after the second was asked: dropped.
    let stale = open_off_ui(&home, &cwd, &ids[1]);
    tui.handle(Msg::Opened(1, stale));
    assert_eq!(tui.session, ids[0], "dropped");
    assert!(tui.ui.loading);
    let fresh = open_off_ui(&home, &cwd, &ids[2]);
    tui.handle(Msg::Opened(2, fresh));
    assert!(!tui.ui.loading);
    assert_eq!(tui.session, ids[2]);
    ui_thread::mark(false);
    let _ = std::fs::remove_dir_all(&root);
}

#[test]
fn an_open_that_fails_shows_the_error_and_keeps_the_session() {
    let root = root("error");
    let (mut tui, _rx, home, cwd, ids) = tui(&root);
    ui_thread::mark(true);
    tui.open_session(Some("no-such-session"));
    let missing = open_off_ui(&home, &cwd, "no-such-session");
    assert!(missing.is_err());
    tui.handle(Msg::Opened(1, missing));
    assert!(!tui.ui.loading);
    assert!(
        tui.ui.status.contains("no-such-session"),
        "{}",
        tui.ui.status
    );
    assert_eq!(tui.session, ids[0]);
    ui_thread::mark(false);
    let _ = std::fs::remove_dir_all(&root);
}

#[test]
fn a_prompt_is_refused_while_a_session_loads() {
    let root = root("send");
    let (mut tui, _rx, _home, _cwd, ids) = tui(&root);
    ui_thread::mark(true);
    tui.open_session(Some(&ids[1]));
    tui.ui.input.insert("hello");
    tui.act(Action::Send("hello".into()));
    assert_eq!(tui.ui.status, "a session is loading");
    assert_eq!(user_count(&tui), 0, "nothing was sent");
    ui_thread::mark(false);
    let _ = std::fs::remove_dir_all(&root);
}

#[test]
fn a_new_session_cancels_an_open_still_running() {
    let root = root("cancel");
    let (mut tui, _rx, home, cwd, ids) = tui(&root);
    ui_thread::mark(true);
    tui.open_session(Some(&ids[1]));
    tui.act(Action::NewSession);
    assert!(!tui.ui.loading);
    let fresh = tui.session.clone();
    assert!(fresh != ids[0] && fresh != ids[1], "a new session at once");
    // The cancelled open's result is dropped, so it never replaces it.
    let result = open_off_ui(&home, &cwd, &ids[1]);
    tui.handle(Msg::Opened(1, result));
    assert_eq!(tui.session, fresh);
    ui_thread::mark(false);
    let _ = std::fs::remove_dir_all(&root);
}
