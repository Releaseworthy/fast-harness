//! A finished subagent's session shown in the chat (D79): one long turn of
//! folded steps, as its log opens again when the child ends
//! (`child_pump::ended` -> `open_load::reload` -> `show_run`), draws at the
//! bottom of the view (2026-11-14: the owner saw the view empty).

use std::path::{Path, PathBuf};
use std::rc::Rc;

use fast_harness_core::agent::Conversation;
use fast_harness_core::app::Runtime;
use fast_harness_core::events::{AgentEvent, TurnOutcome};
use fast_harness_core::view::opened::{self, Opened};
use fast_harness_core::view::Transcript;
use slint::platform::software_renderer::MinimalSoftwareWindow;
use slint::{ComponentHandle, Model, Rgb8Pixel};

use crate::app::{install, with_app, App};
use crate::turn_nav_view_tests::{frames, window, HEIGHT, WIDTH};
use crate::{open_load, MainWindow, TurnsUi};

fn temp(tag: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!(
        "fh-child-view-{tag}-{}",
        fast_harness_core::session::new_id()
    ));
    std::fs::create_dir_all(&dir).expect("temp dir");
    dir
}

/// A child's one turn of `steps` steps, each thinking, saying a word and
/// running two tools, then its answer; the turn ended.
fn child_turn(steps: u32) -> Transcript {
    let mut view = Transcript::new();
    view.push_user("Review the GUI's view model and report what to change".to_string());
    view.apply(&AgentEvent::TurnStart { turn: 1 });
    for step in 1..=steps {
        view.apply(&AgentEvent::StepStart { turn: 1, step });
        view.apply(&AgentEvent::ThinkingDelta("Which file next.".into()));
        view.apply(&AgentEvent::TextDelta(format!("Step {step}: reading.")));
        for (id, name) in [("a", "grep"), ("b", "read")] {
            let id = format!("{id}{step}");
            let arguments = r#"{"path":"src/app.rs"}"#.into();
            let (name, content) = (name.into(), "fn main() {}".into());
            view.apply(&AgentEvent::ToolCall {
                id: id.clone(),
                name,
                arguments,
            });
            let result = AgentEvent::ToolResult {
                id,
                content,
                is_error: false,
                duration_ms: 40,
            };
            view.apply(&result);
        }
    }
    view.apply(&AgentEvent::TextDelta("The view model is fine. ".repeat(6)));
    view.apply(&AgentEvent::TurnEnd {
        turn: 1,
        usage: Default::default(),
        duration_ms: 9_000,
        outcome: TurnOutcome::Done,
    });
    view
}

/// For each line of the chat list: does the column draw anything there.
pub fn lines(window: &MinimalSoftwareWindow, ui: &MainWindow) -> Vec<bool> {
    let size = window.size();
    let width = size.width as usize;
    let mut pixels = vec![Rgb8Pixel { r: 0, g: 0, b: 0 }; width * size.height as usize];
    window.request_redraw();
    window.draw_if_needed(|r| {
        r.render(&mut pixels, width);
    });
    let nav = ui.global::<TurnsUi>();
    let (left, top) = (nav.get_column_left() as usize, nav.get_list_top() as usize);
    let end = (top + ui.get_chat_list_height() as usize).min(size.height as usize);
    let at = |x: usize, y: usize| pixels[y * width + x];
    (top..end)
        .map(|y| (left..left + 600).any(|x| at(x, y) != at(left - 4, y)))
        .collect()
}

/// The list draws down to its end: no blank stretch of 60 px and more.
pub fn assert_drawn(window: &MinimalSoftwareWindow, ui: &MainWindow, what: &str) {
    let lines = lines(window, ui);
    let last = lines.iter().rposition(|&d| d).unwrap_or(0);
    assert!(
        last + 60 > lines.len(),
        "{what}: the list stops drawing at {last} of {}",
        lines.len()
    );
    let mut run = 0;
    for &drawn in &lines {
        run = if drawn { 0 } else { run + 1 };
        assert!(run < 60, "{what}: a blank gap of {run} px");
    }
}

/// A window on a fresh home, its app in `cwd`, and a parent session with a
/// child session in `child_cwd`.
fn setup(dir: &Path, child_cwd: &Path) -> (Rc<MinimalSoftwareWindow>, MainWindow, PathBuf, String) {
    let (window, ui) = window(WIDTH, HEIGHT);
    let (home, cwd) = (dir.join("home"), dir.join("work"));
    std::fs::create_dir_all(&cwd).expect("cwd");
    std::fs::create_dir_all(child_cwd).expect("child cwd");
    let parent = Conversation::create(&home, child_cwd, None)
        .expect("parent")
        .id;
    let child = Conversation::create_at(&home, child_cwd, Some(&parent), 1)
        .expect("child")
        .id;
    let rt = Runtime::load_at(home.clone()).expect("runtime");
    install(App::new(&ui, rt, Some(cwd)));
    (window, ui, home, child)
}

/// The child's log opened (as the reload's worker would) with `view` as
/// its transcript, applied to the window.
fn reopened(home: &Path, cwd: &Path, child: &str, view: Transcript) {
    let generation = with_app(|app, _| app.opens.ask(Some(child)).map(|a| a.generation));
    let generation = generation.flatten().expect("an open starts");
    let result = opened::load(home, cwd, child).map(|o| Opened { conv: o.conv, view });
    with_app(|app, ui| open_load::arrived(app, ui, generation, cwd, result));
}

#[test]
fn a_finished_child_draws_down_to_the_end_of_the_view() {
    let dir = temp("end");
    let child_cwd = dir.join("work");
    let (window, ui, home, child) = setup(&dir, &child_cwd);
    reopened(&home, &child_cwd, &child, child_turn(30));
    frames(&window, 300);
    assert_eq!(with_app(|app, _| app.runs.id.clone()), Some(Some(child)));
    assert!(ui.get_messages().row_count() > 30, "the child's rows show");
    let bottom = ui.global::<TurnsUi>().get_list_top() + ui.get_chat_list_height();
    assert!((ui.get_chat_end() - bottom).abs() <= 2.0, "at the bottom");
    assert_drawn(&window, &ui, "the finished child");
    crate::fold_view_tests::shot(&window, "finished-child");
    let _ = std::fs::remove_dir_all(&dir);
}

/// The reload reads the log under the window's folder (`app.cwd`): a child
/// whose session lives under another folder fails to read there, which
/// shows the error and keeps the rows shown; it never empties the view.
#[test]
fn a_child_log_under_another_folder_keeps_the_rows_shown() {
    let dir = temp("cwd");
    let child_cwd = dir.join("other");
    let (window, ui, home, child) = setup(&dir, &child_cwd);
    reopened(&home, &child_cwd, &child, child_turn(30));
    frames(&window, 200);
    let rows = ui.get_messages().row_count();
    let app_cwd = with_app(|app, _| app.cwd.clone()).expect("app");
    reopened(&home, &app_cwd, &child, Transcript::new());
    frames(&window, 200);
    assert!(!ui.get_error_text().is_empty(), "the failed read shows");
    assert_eq!(ui.get_messages().row_count(), rows, "the rows stay");
    assert_drawn(&window, &ui, "after the failed read");
    let _ = std::fs::remove_dir_all(&dir);
}
