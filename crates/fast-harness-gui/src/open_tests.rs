//! Opening a session's log on a worker (D60, gate G10): the UI-thread call
//! returns without parsing, a result that is not the newest is dropped, and
//! an open that fails shows its error and keeps the session the window
//! shows. The test thread is marked as the UI thread, so a log read on it
//! panics in a debug build; no event loop runs here, so the workers' results
//! are handed in by hand.

use std::path::{Path, PathBuf};
use std::rc::Rc;

use fast_harness_core::agent::Conversation;
use fast_harness_core::app::Runtime;
use fast_harness_core::ui_thread;
use fast_harness_core::view::opened::{self, Opened};
use slint::platform::software_renderer::{MinimalSoftwareWindow, RepaintBufferType};
use slint::platform::{Platform, WindowAdapter};
use slint::{ComponentHandle, Model};

use crate::app::{install, with_app, App};
use crate::{open_load, turn, MainWindow};

struct TestPlatform(Rc<MinimalSoftwareWindow>);

impl Platform for TestPlatform {
    fn create_window_adapter(&self) -> Result<Rc<dyn WindowAdapter>, slint::PlatformError> {
        Ok(self.0.clone())
    }
}

fn root(tag: &str) -> PathBuf {
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| d.subsec_nanos());
    std::env::temp_dir().join(format!("fh-open-{tag}-{}-{nanos}", std::process::id()))
}

/// A window on a home with two sessions in `work`.
fn setup(dir: &Path) -> (MainWindow, PathBuf, PathBuf, String, String) {
    let window = MinimalSoftwareWindow::new(RepaintBufferType::NewBuffer);
    let _ = slint::platform::set_platform(Box::new(TestPlatform(window)));
    let ui = MainWindow::new().expect("window");
    let (home, cwd) = (dir.join("home"), dir.join("work"));
    std::fs::create_dir_all(&cwd).unwrap();
    let first = Conversation::create(&home, &cwd, None).unwrap().id;
    let second = Conversation::create(&home, &cwd, None).unwrap().id;
    let rt = Runtime::load_at(home.clone()).unwrap();
    install(App::new(&ui, rt, Some(cwd.clone())));
    (ui, home, cwd, first, second)
}

/// A worker's open, read with the mark lifted (as its own thread would).
fn open_off_ui(home: &Path, cwd: &Path, id: &str) -> Result<Opened, String> {
    ui_thread::mark(false);
    let result = opened::load(home, cwd, id);
    ui_thread::mark(true);
    result
}

#[test]
fn the_open_returns_at_once_and_a_turn_cannot_start_meanwhile() {
    let dir = root("loading");
    let (ui, home, cwd, first, _second) = setup(&dir);
    ui_thread::mark(true);
    // The UI-thread call returns with nothing parsed: a log read here would
    // panic on the mark, so the parse can only happen on the worker.
    with_app(|app, ui| app.open_session(ui, &first));
    assert!(ui.get_loading(), "the view says a session is opening");
    assert_eq!(ui.get_messages().row_count(), 0);
    // A turn cannot be started on the half-opened session.
    assert_eq!(
        with_app(|app, ui| turn::send(app, ui, "hello")),
        Some(false)
    );
    assert_eq!(with_app(|app, _| app.runs.id.clone()), Some(None));
    // The worker's result arrives (by hand: no event loop runs here).
    let result = open_off_ui(&home, &cwd, &first);
    with_app(|app, ui| open_load::arrived(app, ui, 1, &cwd, result));
    assert!(!ui.get_loading());
    assert_eq!(with_app(|app, _| app.runs.id.clone()), Some(Some(first)));
    assert!(with_app(|app, _| app.opens.loading()).is_some_and(|l| !l));
    ui_thread::mark(false);
    let _ = ui.hide();
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn a_second_open_supersedes_the_first_and_its_result_is_dropped() {
    let dir = root("stale");
    let (ui, home, cwd, first, second) = setup(&dir);
    ui_thread::mark(true);
    with_app(|app, ui| app.open_session(ui, &first));
    with_app(|app, ui| app.open_session(ui, &second));
    assert!(ui.get_loading(), "open twice: still loading");
    // The first worker's result comes after the second open was asked: it is
    // dropped, so the first session never reaches the window.
    let stale = open_off_ui(&home, &cwd, &first);
    with_app(|app, ui| open_load::arrived(app, ui, 1, &cwd, stale));
    assert_eq!(
        with_app(|app, _| app.runs.id.clone()),
        Some(None),
        "dropped"
    );
    assert!(ui.get_loading(), "the newest open still runs");
    // The newest result applies, and only the second session is shown.
    let fresh = open_off_ui(&home, &cwd, &second);
    with_app(|app, ui| open_load::arrived(app, ui, 2, &cwd, fresh));
    assert!(!ui.get_loading());
    assert_eq!(with_app(|app, _| app.runs.id.clone()), Some(Some(second)));
    ui_thread::mark(false);
    let _ = ui.hide();
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn an_open_that_fails_shows_the_error_and_keeps_the_session() {
    let dir = root("error");
    let (ui, home, cwd, first, _second) = setup(&dir);
    ui_thread::mark(true);
    with_app(|app, ui| app.open_session(ui, &first));
    let ok = open_off_ui(&home, &cwd, &first);
    with_app(|app, ui| open_load::arrived(app, ui, 1, &cwd, ok));
    let before = with_app(|app, _| app.runs.id.clone());
    assert_eq!(before, Some(Some(first.clone())));
    // A session whose log does not read: the error shows, the run stays.
    let missing = open_off_ui(&home, &cwd, "no-such-session");
    assert!(missing.is_err());
    with_app(|app, ui| app.open_session(ui, "no-such-session"));
    with_app(|app, ui| open_load::arrived(app, ui, 2, &cwd, missing));
    assert!(!ui.get_loading());
    assert!(ui.get_error_text().contains("no-such-session"));
    assert_eq!(with_app(|app, _| app.runs.id.clone()), before);
    ui_thread::mark(false);
    let _ = ui.hide();
    let _ = std::fs::remove_dir_all(&dir);
}
