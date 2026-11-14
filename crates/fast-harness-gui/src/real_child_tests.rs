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
    let ui = crate::chat_list::new_window().expect("window");
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
/// owner's, 2026-11-13, strings cut to 120 characters, accounts and
/// signatures replaced, the answer replaced by a long filler): 32 steps of
/// tool calls folded to their lines, then the answer step, whose usage
/// footer (folded, no height) is the last row.
const CHILD_LOG: &str = include_str!("fixtures/finished-child.jsonl");

/// Writes `log` (the fixture or a part of it) as child `child` of `parent`.
fn write_child(home: &Path, cwd: &Path, parent: &str, child: &str, log: &str) {
    let json_cwd = cwd.to_string_lossy().replace('\\', "\\\\");
    let log = log
        .replacen("CHILD_ID", child, 1)
        .replacen("PARENT_ID", parent, 1);
    let path = session::dir(home, cwd).join(format!("{child}.jsonl"));
    std::fs::write(path, log.replacen("CHILD_CWD", &json_cwd, 1)).expect("child log");
}

/// A home with a parent session and the fixture child; returns both ids.
fn fixture_home(dir: &Path, cwd: &Path, log: &str) -> (PathBuf, String, String) {
    let home = dir.join("home");
    std::fs::create_dir_all(cwd).expect("cwd");
    let conv = fast_harness_core::agent::Conversation::create(&home, cwd, None);
    let parent = conv.expect("parent").id;
    let child = format!("{parent}-child");
    write_child(&home, cwd, &parent, &child, log);
    (home, parent, child)
}

/// Opens session `id` as `App::open_session` does when its log must be
/// read, with the worker's part run on this thread (the plain test platform
/// has no event loop proxy, so a spawned worker's result never comes).
/// Returns the open's generation.
fn open_here(home: &Path, cwd: &Path, id: &str) -> u64 {
    let generation = with_app(|app, ui| {
        ui.set_loading(true);
        app.opens.ask(Some(id)).map(|a| a.generation)
    });
    let generation = generation.flatten().expect("an open starts");
    arrive(home, cwd, id, generation);
    generation
}

/// The open worker of `generation` for session `id` ended.
fn arrive(home: &Path, cwd: &Path, id: &str, generation: u64) {
    let result = fast_harness_core::view::opened::load(home, cwd, id);
    with_app(|app, ui| crate::open_load::arrived(app, ui, generation, cwd, result));
}

/// A click on child `child`'s row (`subagents_view::open_child`) whose
/// open runs here: the view stack gets the child, then its log opens.
fn click_here(home: &Path, cwd: &Path, parent: &str, child: &str) -> u64 {
    with_app(|app, _| {
        let (child, parent) = (child.to_string(), parent.to_string());
        let (back, label) = ("parent session".to_string(), "subagent".to_string());
        let view = crate::subagents_view::Viewing {
            child,
            parent,
            back,
            label,
        };
        app.subagents.viewing.push(view);
    });
    open_here(home, cwd, child)
}
/// The app on a plain test window, the parent shown and its children read.
fn show_parent(home: &Path, cwd: &Path, parent: &str) -> (Rc<MinimalSoftwareWindow>, MainWindow) {
    let (window, ui) = crate::turn_nav_view_tests::window(WIDTH, HEIGHT);
    let rt = Runtime::load_at(home.to_path_buf()).expect("runtime");
    install(App::new(&ui, rt, Some(cwd.to_path_buf())));
    open_here(home, cwd, parent);
    let found = session::children(home, cwd, parent).expect("children");
    with_app(|app, _| app.subagents.children = found);
    crate::turn_nav_view_tests::frames(&window, 200);
    (window, ui)
}

/// The chat draws its rows down to its end, at the bottom; saves `name`.png.
fn assert_end_drawn(window: &MinimalSoftwareWindow, ui: &MainWindow, what: &str, name: &str) {
    crate::fold_view_tests::shot(window, name);
    assert!(
        ui.get_messages().row_count() > 0,
        "{what}: no rows: {}",
        state(ui)
    );
    crate::child_view_tests::assert_drawn(window, ui, &format!("{what}: {}", state(ui)));
    assert_at_bottom(ui, what);
}

/// A finished child opened from its row draws its rows down to its end
/// (2026-11-14: the owner saw it empty; its folded rows of no height left
/// Slint's ListView estimate at 0).
#[test]
fn a_finished_child_opened_from_its_row_draws_at_the_bottom() {
    let dir = temp("fixture");
    let cwd = dir.join("work");
    let (home, parent, child) = fixture_home(&dir, &cwd, CHILD_LOG);
    let (window, ui) = show_parent(&home, &cwd, &parent);
    click_here(&home, &cwd, &parent, &child);
    crate::turn_nav_view_tests::frames(&window, 600);
    assert!(with_app(|app, ui| crate::subagents_view::render(app, ui)).is_some());
    assert_eq!(with_app(|app, _| app.runs.id.clone()), Some(Some(child)));
    assert_end_drawn(&window, &ui, "the finished child", "fixture-child");
    let _ = std::fs::remove_dir_all(&dir);
}

/// A child shown while it runs gets its live events (`turn::deliver`, as
/// `child_pump` hands them); when it ends (`child_pump::ended`) its whole
/// log opens again (`open_load::reload`) and draws down to its end.
#[test]
fn a_running_child_that_ends_draws_its_whole_log() {
    let dir = temp("running");
    let cwd = dir.join("work");
    let last_step = CHILD_LOG.rfind("{\"type\":\"step/start\"").expect("a step");
    let (home, parent, child) = fixture_home(&dir, &cwd, &CHILD_LOG[..last_step]);
    let (window, ui) = show_parent(&home, &cwd, &parent);
    let opened = click_here(&home, &cwd, &parent, &child);
    with_app(|app, _| {
        let run = app.runs.get_mut(&child).expect("the child's run");
        run.busy = true;
        app.subagents.pumped = Some(child.clone());
    });
    use fast_harness_core::events::AgentEvent;
    crate::turn::deliver(&child, AgentEvent::StepStart { turn: 1, step: 32 });
    crate::turn::deliver(&child, AgentEvent::TextDelta("Writing the report.".into()));
    crate::turn_nav_view_tests::frames(&window, 300);
    assert_end_drawn(&window, &ui, "the running child", "running-child");
    write_child(&home, &cwd, &parent, &child, CHILD_LOG);
    // `ended` asks the reload (the next generation); its worker's part:
    with_app(|app, ui| crate::child_pump::ended(app, ui, &child));
    arrive(&home, &cwd, &child, opened + 1);
    crate::turn_nav_view_tests::frames(&window, 600);
    assert!(
        with_app(|app, _| !app.runs.shown.busy).unwrap_or(false),
        "the run is idle"
    );
    assert_end_drawn(&window, &ui, "the ended child", "running-child-ended");
    let _ = std::fs::remove_dir_all(&dir);
}

/// The same through the real workers (`App::open_session`, the subagents
/// scan): ignored, since the event loop proxy it installs is one per
/// process and would run other tests' worker results on this thread.
#[test]
#[ignore = "installs a process-wide event loop proxy"]
fn a_finished_child_opened_through_the_workers_draws() {
    let dir = temp("workers");
    let cwd = dir.join("work");
    let (home, parent, child) = fixture_home(&dir, &cwd, CHILD_LOG);
    let (window, ui) = window();
    open_parent(&window, &ui, &home, &cwd, &parent, 1);
    click_child(&window, &child);
    assert_end_drawn(&window, &ui, "the finished child", "workers-child");
    let _ = std::fs::remove_dir_all(&dir);
}

const OWNER: &str = r"C:\Users\Yamasaki\.fast-harness";
const PARENT: &str = "20261113-195004-6140";

/// The owner's own logs (read-only, copied), through the real workers:
/// the parent and every child opened from its row draw down to their end.
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
    assert_end_drawn(&window, &ui, "the parent", "owner-parent");
    for child in &children {
        click_child(&window, child);
        assert_end_drawn(&window, &ui, child, &format!("owner-{child}"));
        with_app(crate::subagents_view::back);
        until(&window, "back to the parent", |app, ui| {
            app.runs.id.as_deref() == Some(PARENT) && !ui.get_loading()
        });
    }
    let _ = std::fs::remove_dir_all(&dir);
}
/// The chat shows its end: the last row ends at the list's bottom.
pub fn assert_at_bottom(ui: &MainWindow, what: &str) {
    let bottom = ui.global::<crate::TurnsUi>().get_list_top() + ui.get_chat_list_height();
    let end = ui.get_chat_end();
    assert!(
        (end - bottom).abs() <= 2.0,
        "{what}: not at the bottom: ends at {end}, list bottom {bottom}"
    );
}
