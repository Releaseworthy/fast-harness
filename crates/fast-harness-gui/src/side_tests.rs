//! The sidebar and the Trace page read on workers (D60). The test thread is
//! marked as the UI thread, so a session list or a log read on it panics in
//! a debug build: expanding a workspace, a turn end (`refresh_sessions`),
//! opening a session and opening the Trace page must not read there. No
//! event loop runs here, so the workers' results are handed in by hand.

use std::path::{Path, PathBuf};
use std::rc::Rc;

use fast_harness_core::agent::Conversation;
use fast_harness_core::app::Runtime;
use fast_harness_core::session::{self, slug};
use fast_harness_core::ui_thread;
use slint::platform::software_renderer::{MinimalSoftwareWindow, RepaintBufferType};
use slint::platform::{Platform, WindowAdapter};
use slint::{ComponentHandle, Model};

use crate::app::{install, with_app, App};
use crate::{trace_load, trace_rows, workspaces, MainWindow};

struct TestPlatform(Rc<MinimalSoftwareWindow>);

impl Platform for TestPlatform {
    fn create_window_adapter(&self) -> Result<Rc<dyn WindowAdapter>, slint::PlatformError> {
        Ok(self.0.clone())
    }
}

fn kinds(ui: &MainWindow) -> Vec<String> {
    let rows = ui.get_side_rows();
    (0..rows.row_count())
        .filter_map(|i| rows.row_data(i))
        .map(|r| r.kind.to_string())
        .collect()
}

/// A worker's list, read with the mark lifted (as a worker thread would).
fn list_off_ui(home: &Path, cwd: &Path) -> crate::listings::Listed {
    ui_thread::mark(false);
    let list = session::list(home, cwd).map_err(|e| e.to_string());
    ui_thread::mark(true);
    list
}

fn setup(root: &Path) -> (MainWindow, PathBuf, PathBuf, String) {
    let window = MinimalSoftwareWindow::new(RepaintBufferType::NewBuffer);
    let _ = slint::platform::set_platform(Box::new(TestPlatform(window)));
    let ui = MainWindow::new().expect("window");
    let (home, cwd) = (root.join("home"), root.join("work"));
    std::fs::create_dir_all(&cwd).unwrap();
    let first = Conversation::create(&home, &cwd, None).unwrap().id;
    Conversation::create(&home, &cwd, None).unwrap();
    let rt = Runtime::load_at(home.clone()).unwrap();
    install(App::new(&ui, rt, Some(cwd.clone())));
    (ui, home, cwd, first)
}

#[test]
fn lists_and_the_trace_page_load_off_the_ui_thread_and_stale_results_drop() {
    let root = std::env::temp_dir().join(format!("fh-side-{}", std::process::id()));
    let (ui, home, cwd, first) = setup(&root);
    ui_thread::mark(true);
    let key = slug(&cwd);
    with_app(|app, ui| app.refresh_sessions(ui));
    assert_eq!(
        kinds(&ui),
        ["workspace", "loading"],
        "expanded: loading row"
    );
    let list = list_off_ui(&home, &cwd);
    with_app(|app, ui| workspaces::listed(app, ui, &key, 1, list));
    assert_eq!(kinds(&ui), ["workspace", "session", "session"]);
    // A turn end asks again; the rows stay until the new list comes.
    with_app(|app, ui| app.refresh_sessions(ui));
    assert_eq!(kinds(&ui), ["workspace", "session", "session"]);
    // Collapsed meanwhile: the late list changes nothing.
    let path = cwd.to_string_lossy().to_string();
    with_app(|app, ui| workspaces::select(app, ui, &path));
    let list = list_off_ui(&home, &cwd);
    with_app(|app, ui| workspaces::listed(app, ui, &key, 2, list));
    assert_eq!(kinds(&ui), ["workspace"]);
    // Opening a session parses its log once, in the reopen.
    with_app(|app, ui| app.open_session(ui, &first));
    ui.set_page("trace".into());
    with_app(|app, ui| app.refresh_trace(ui));
    assert_eq!(ui.get_trace_message(), trace_load::LOADING);
    with_app(|app, ui| app.refresh_trace(ui));
    let log = session::log_path(&home, &cwd, &first);
    ui_thread::mark(false);
    let (older, newer) = (trace_rows::load(&log), trace_rows::load(&log));
    ui_thread::mark(true);
    let shown = with_app(|app, ui| trace_load::loaded(app, ui, &first, 1, older));
    assert_eq!(shown, Some(false), "an older read is dropped");
    let shown = with_app(|app, ui| trace_load::loaded(app, ui, &first, 2, newer));
    assert_eq!(shown, Some(true));
    assert_ne!(ui.get_trace_message(), trace_load::LOADING);
    ui_thread::mark(false);
    let _ = ui.hide();
    let _ = std::fs::remove_dir_all(&root);
}
