//! A finished subagent opened through the window's real paths (2026-11-14,
//! the owner saw a finished child's chat view empty): a home holding a
//! parent log and its child logs, the parent opened as a session pick
//! does (`App::open_session`, a worker reads it), the children listed by
//! the subagents scan (a worker too), then a child opened as a click on
//! its row does (`subagents_view::open_child`). The workers hand their
//! results to the UI thread through `slint::invoke_from_event_loop`, which
//! this test platform queues and [`pump`] runs between frames.

use std::path::{Path, PathBuf};
use std::rc::Rc;
use std::sync::Mutex;
use std::time::{Duration, Instant};

use fast_harness_core::app::Runtime;
use fast_harness_core::session;
use slint::platform::software_renderer::{MinimalSoftwareWindow, RepaintBufferType};
use slint::platform::{EventLoopProxy, Platform, WindowAdapter};
use slint::{ComponentHandle, Model, PhysicalSize, Rgb8Pixel};

use crate::app::{install, with_app, App};
use crate::turn_nav_view_tests::{HEIGHT, WIDTH};
use crate::MainWindow;

type Job = Box<dyn FnOnce() + Send>;

/// The closures the workers sent, run by [`pump`] on the UI thread.
static QUEUE: Mutex<Vec<Job>> = Mutex::new(Vec::new());

struct Proxy;

impl EventLoopProxy for Proxy {
    fn quit_event_loop(&self) -> Result<(), slint::EventLoopError> {
        Ok(())
    }

    fn invoke_from_event_loop(&self, event: Job) -> Result<(), slint::EventLoopError> {
        QUEUE
            .lock()
            .map_err(|_| slint::EventLoopError::NoEventLoopProvider)?
            .push(event);
        Ok(())
    }
}

struct LoopPlatform(Rc<MinimalSoftwareWindow>);

impl Platform for LoopPlatform {
    fn create_window_adapter(&self) -> Result<Rc<dyn WindowAdapter>, slint::PlatformError> {
        Ok(self.0.clone())
    }

    fn new_event_loop_proxy(&self) -> Option<Box<dyn EventLoopProxy>> {
        Some(Box::new(Proxy))
    }
}

/// A window whose workers reach the UI thread.
pub fn window() -> (Rc<MinimalSoftwareWindow>, MainWindow) {
    let window = MinimalSoftwareWindow::new(RepaintBufferType::NewBuffer);
    // The event loop proxy is one per process: a later test of this binary
    // gets a plain platform, and the first one's proxy still feeds [`QUEUE`].
    if slint::platform::set_platform(Box::new(LoopPlatform(window.clone()))).is_err() {
        let plain = crate::turn_nav_view_tests::TestPlatform(window.clone());
        slint::platform::set_platform(Box::new(plain)).expect("platform");
    }
    window.set_size(PhysicalSize::new(WIDTH, HEIGHT));
    let ui = MainWindow::new().expect("window");
    ui.show().expect("show");
    (window, ui)
}

/// Runs the queued worker results, the timers and a frame, for `ms`.
pub fn pump(window: &MinimalSoftwareWindow, ms: u64) {
    let size = window.size();
    let mut buffer = vec![Rgb8Pixel { r: 0, g: 0, b: 0 }; (size.width * size.height) as usize];
    for _ in 0..ms.div_ceil(16) {
        std::thread::sleep(Duration::from_millis(16));
        let jobs: Vec<Job> = QUEUE
            .lock()
            .map(|mut q| q.drain(..).collect())
            .unwrap_or_default();
        jobs.into_iter().for_each(|job| job());
        slint::platform::update_timers_and_animations();
        window.request_redraw();
        window.draw_if_needed(|renderer| {
            renderer.render(&mut buffer, size.width as usize);
        });
    }
}

/// Pumps until `done` holds, at most 60 s.
pub fn until(window: &MinimalSoftwareWindow, what: &str, done: impl Fn(&App, &MainWindow) -> bool) {
    let start = Instant::now();
    while !with_app(|app, ui| done(app, ui)).unwrap_or(false) {
        assert!(
            start.elapsed() < Duration::from_secs(60),
            "timed out: {what}"
        );
        pump(window, 50);
    }
}

/// A fresh home under `dir` with `files` of folder `from` copied into the
/// sessions folder of `cwd`, and the owner's config when given.
pub fn home(dir: &Path, cwd: &Path, from: &Path, files: &[&str], config: Option<&Path>) -> PathBuf {
    let home = dir.join("home");
    let sessions = session::dir(&home, cwd);
    std::fs::create_dir_all(&sessions).expect("sessions dir");
    for name in files {
        std::fs::copy(from.join(name), sessions.join(name)).expect("copy log");
    }
    if let Some(config) = config {
        std::fs::copy(config, home.join("config.json")).expect("copy config");
    }
    home
}

/// The app on `home` in `cwd`, with `parent` opened as a pick does and its
/// children listed (`count` of them).
pub fn open_parent(
    window: &MinimalSoftwareWindow,
    ui: &MainWindow,
    home: &Path,
    cwd: &Path,
    parent: &str,
    count: usize,
) {
    let rt = Runtime::load_at(home.to_path_buf()).expect("runtime");
    install(App::new(ui, rt, Some(cwd.to_path_buf())));
    pump(window, 200);
    with_app(|app, ui| app.open_session(ui, parent));
    until(window, "the parent opens", |app, ui| {
        app.runs.id.as_deref() == Some(parent) && !ui.get_loading()
    });
    until(window, "the children are listed", |app, _| {
        app.subagents.children.len() == count
    });
    pump(window, 300);
}

/// Clicks child `id`'s row and waits for its log to show.
pub fn click_child(window: &MinimalSoftwareWindow, id: &str) {
    with_app(|app, ui| crate::subagents_view::open_child(app, ui, id));
    until(window, "the child opens", |app, ui| {
        app.runs.id.as_deref() == Some(id) && !ui.get_loading()
    });
    pump(window, 600);
}

/// What the chat shows now, for a failing assertion.
pub fn state(ui: &MainWindow) -> String {
    let (rows, error) = (ui.get_messages().row_count(), ui.get_error_text());
    let (page, end) = (ui.get_page(), ui.get_chat_end());
    format!("rows {rows}, page {page}, chat_end {end}, error {error:?}")
}

fn temp(tag: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("fh-real-child-{tag}-{}", session::new_id()));
    std::fs::create_dir_all(&dir).expect("temp dir");
    dir
}

/// The owner's own logs (read-only, copied): every child of the parent
/// session opened from its row draws its rows.
/// The child's log: a trimmed copy of a real finished subagent's log (the
/// owner's, 2026-11-13, strings cut to 300 characters, accounts and
/// signatures replaced): six steps of tool calls, then the answer step.
/// Its last row, the answer step's usage footer, is folded to no height.
const CHILD_LOG: &str = include_str!("fixtures/finished-child.jsonl");

/// Opens the child row and asserts it draws; saves `name`.png.
fn assert_child_drawn(window: &MinimalSoftwareWindow, ui: &MainWindow, child: &str, name: &str) {
    click_child(window, child);
    crate::fold_view_tests::shot(window, name);
    assert!(
        ui.get_messages().row_count() > 0,
        "{child}: no rows: {}",
        state(ui)
    );
    crate::child_view_tests::assert_drawn(window, ui, &format!("{child}: {}", state(ui)));
}

/// A finished child opened from its row in the subagents popover draws its
/// rows, also when its last row has no height (2026-11-14: the owner saw
/// the view empty).
#[test]
fn a_finished_child_opened_from_its_row_draws() {
    let dir = temp("fixture");
    let (home, cwd) = (dir.join("home"), dir.join("work"));
    std::fs::create_dir_all(&cwd).expect("cwd");
    let parent = fast_harness_core::agent::Conversation::create(&home, &cwd, None)
        .expect("parent")
        .id;
    let child = format!("{parent}-child");
    let json_cwd = cwd.to_string_lossy().replace('\\', "\\\\");
    let log = CHILD_LOG
        .replacen("CHILD_ID", &child, 1)
        .replacen("PARENT_ID", &parent, 1);
    let path = session::dir(&home, &cwd).join(format!("{child}.jsonl"));
    std::fs::write(path, log.replacen("CHILD_CWD", &json_cwd, 1)).expect("child log");
    let (window, ui) = window();
    open_parent(&window, &ui, &home, &cwd, &parent, 1);
    assert_child_drawn(&window, &ui, &child, "fixture-child");
    let _ = std::fs::remove_dir_all(&dir);
}

const OWNER: &str = r"C:\Users\Yamasaki\.fast-harness";
const PARENT: &str = "20261113-195004-6140";

/// The owner's own logs (read-only, copied): every child of the parent
/// session opened from its row draws its rows.
#[test]
#[ignore = "reads the owner's sessions under C:\\Users\\Yamasaki"]
fn the_owners_finished_children_draw() {
    let dir = temp("owner");
    let cwd = PathBuf::from(r"F:\work\fast-harness-general");
    let from = session::dir(Path::new(OWNER), &cwd);
    let children: Vec<String> = (6141..=6144)
        .map(|n| format!("20261113-195342-{n}"))
        .collect();
    let mut files = vec![format!("{PARENT}.jsonl"), "index.json".to_string()];
    files.extend(children.iter().map(|c| format!("{c}.jsonl")));
    let names: Vec<&str> = files.iter().map(String::as_str).collect();
    let config = Path::new(OWNER).join("config.json");
    let home = home(&dir, &cwd, &from, &names, Some(&config));
    let (window, ui) = window();
    open_parent(&window, &ui, &home, &cwd, PARENT, children.len());
    crate::fold_view_tests::shot(&window, "owner-parent");
    for child in &children {
        assert_child_drawn(&window, &ui, child, &format!("owner-{child}"));
        with_app(crate::subagents_view::back);
        until(&window, "back to the parent", |app, ui| {
            app.runs.id.as_deref() == Some(PARENT) && !ui.get_loading()
        });
    }
    let _ = std::fs::remove_dir_all(&dir);
}
