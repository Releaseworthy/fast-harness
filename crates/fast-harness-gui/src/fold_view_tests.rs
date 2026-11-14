//! Folding a turn or a step (D80, D73) in a real (software rendered)
//! window with the view at the top, the middle and the bottom: the rows
//! after the fold draw at once (2026-11-14: a folded turn's rows kept their
//! height and drew blank), unfolding draws the rows again, and a navigator
//! click after a fold brings the turn's prompt to the top.

use std::rc::Rc;

use fast_harness_core::view::{Approvals, Transcript};
use slint::platform::software_renderer::MinimalSoftwareWindow;
use slint::platform::{PointerEventButton, WindowEvent};
use slint::{ComponentHandle, LogicalPosition, Model, Rgb8Pixel, VecModel};

use crate::mirror::Mirror;
use crate::turn_nav::turns;
use crate::turn_nav_view_tests::{frames, session, show, tick, window, HEIGHT, WIDTH};
use crate::{MainWindow, MsgRow, TurnsUi};

/// Normal gaps between rows are well under this; a folded row that keeps
/// its height leaves 100 px and more.
const GAP: usize = 60;

#[derive(Clone, Copy, Debug)]
enum Place {
    Top,
    Middle,
    Bottom,
}

struct Chat {
    window: Rc<MinimalSoftwareWindow>,
    ui: MainWindow,
    view: Transcript,
    model: Rc<VecModel<MsgRow>>,
    mirror: Mirror,
}

fn chat(place: Place) -> Chat {
    let (window, ui) = window(WIDTH, HEIGHT);
    let view = session(8);
    let model = show(&ui, &view);
    let mut mirror = Mirror::new(false);
    mirror.reset_with(&view.items, &Approvals::default(), &*model);
    frames(&window, 100);
    let chat = Chat {
        window,
        ui,
        view,
        model,
        mirror,
    };
    match place {
        Place::Top => chat.wheel(100_000.0),
        Place::Middle => chat.wheel(900.0),
        Place::Bottom => {}
    }
    chat
}

impl Chat {
    fn wheel(&self, delta_y: f32) {
        let position = self.centre();
        self.window
            .dispatch_event(WindowEvent::PointerMoved { position });
        for _ in 0..10 {
            self.window.dispatch_event(WindowEvent::PointerScrolled {
                position,
                delta_x: 0.0,
                delta_y: delta_y / 10.0,
            });
            frames(&self.window, 50);
        }
        self.window.dispatch_event(WindowEvent::PointerExited);
        frames(&self.window, 300);
    }
    fn centre(&self) -> LogicalPosition {
        let nav = self.ui.global::<TurnsUi>();
        let y = nav.get_list_top() + self.ui.get_chat_list_height() / 2.0;
        LogicalPosition::new(nav.get_column_left() + 100.0, y)
    }
    /// Navigator tick `index` clicked.
    fn go(&self, index: usize) {
        let (position, button) = (tick(&self.ui, index), PointerEventButton::Left);
        self.window
            .dispatch_event(WindowEvent::PointerPressed { position, button });
        self.window
            .dispatch_event(WindowEvent::PointerReleased { position, button });
        self.window.dispatch_event(WindowEvent::PointerExited);
        frames(&self.window, 300);
    }
    /// The user row of the turn at the top of the view.
    fn top_turn(&self) -> usize {
        let top = self.ui.global::<TurnsUi>().get_top_row() as usize;
        let rows: Vec<usize> = turns(&self.view.items).iter().map(|t| t.row).collect();
        rows.into_iter().filter(|&r| r <= top).max().unwrap_or(0)
    }
    fn fold_turn(&mut self, user: usize) {
        self.mirror.toggle_turn(user, &*self.model);
        frames(&self.window, 300);
    }
    fn fold_step(&mut self, head: usize) {
        self.mirror.toggle_step(head, &*self.model);
        frames(&self.window, 300);
    }
    /// For each line of the list: does the column draw anything there.
    fn lines(&self) -> Vec<bool> {
        let size = self.window.size();
        let width = size.width as usize;
        let mut pixels = vec![Rgb8Pixel { r: 0, g: 0, b: 0 }; width * size.height as usize];
        self.window.request_redraw();
        self.window.draw_if_needed(|r| {
            r.render(&mut pixels, width);
        });
        let nav = self.ui.global::<TurnsUi>();
        let (left, top) = (nav.get_column_left() as usize, nav.get_list_top() as usize);
        let end = top + self.ui.get_chat_list_height() as usize;
        let at = |x: usize, y: usize| pixels[y * width + x];
        (top..end.min(size.height as usize))
            .map(|y| (left..left + 600).any(|x| at(x, y) != at(left - 4, y)))
            .collect()
    }
    /// Saves the window as `name`.png under `FH_FOLD_SHOTS` when it is set
    /// (the before and after shots of the fold fix).
    fn shot(&self, name: &str) {
        let Some(dir) = std::env::var_os("FH_FOLD_SHOTS") else {
            return;
        };
        let size = self.window.size();
        let mut pixels = vec![Rgb8Pixel { r: 0, g: 0, b: 0 }; (size.width * size.height) as usize];
        self.window.request_redraw();
        self.window.draw_if_needed(|r| {
            r.render(&mut pixels, size.width as usize);
        });
        let rgb: Vec<u8> = pixels.iter().flat_map(|p| [p.r, p.g, p.b]).collect();
        let png = crate::shots_png::encode(size.width, size.height, &rgb);
        let path = std::path::Path::new(&dir).join(format!("{name}.png"));
        std::fs::write(path, png).expect("shot");
    }
    /// The longest blank stretch between two drawn lines of the list.
    fn longest_gap(&self) -> usize {
        let lines = self.lines();
        let first = lines.iter().position(|&d| d).unwrap_or(0);
        let last = lines.iter().rposition(|&d| d).unwrap_or(0);
        let mut longest = 0;
        let mut run = 0;
        for &drawn in &lines[first..=last] {
            run = if drawn { 0 } else { run + 1 };
            longest = longest.max(run);
        }
        longest
    }
    /// The list draws over most of its height (the end of the chat may
    /// leave space below its last row only at the bottom).
    fn assert_drawn(&self, what: &str) {
        let gap = self.longest_gap();
        assert!(gap < GAP, "{what}: a blank gap of {gap} px in the list");
        let lines = self.lines();
        let last = lines.iter().rposition(|&d| d).unwrap_or(0);
        assert!(
            last + GAP > lines.len(),
            "{what}: the list stops drawing at {last} of {} px",
            lines.len()
        );
    }
}

/// One window per test: the platform is set once per thread.
fn turn_fold(place: Place) {
    {
        let mut chat = chat(place);
        chat.assert_drawn(&format!("{place:?} before"));
        let user = chat.top_turn();
        chat.fold_turn(user);
        assert!(chat.model.row_data(user + 2).expect("row").turn_hidden);
        chat.assert_drawn(&format!("{place:?} turn {user} folded"));
        chat.fold_turn(user);
        assert!(!chat.model.row_data(user + 2).expect("row").turn_hidden);
        chat.assert_drawn(&format!("{place:?} turn {user} open again"));
    }
}

fn step_fold(place: Place) {
    {
        let mut chat = chat(place);
        // The turn's first row is the step line of its one step.
        let head = chat.top_turn() + 1;
        assert_ne!(chat.model.row_data(head).expect("row").step_summary, "");
        chat.fold_step(head);
        chat.assert_drawn(&format!("{place:?} step {head} open"));
        chat.fold_step(head);
        chat.assert_drawn(&format!("{place:?} step {head} folded again"));
    }
}

/// The first, a middle and the last turn folded in turn, each opened again
/// before the next: the list draws over its height every time (2026-11-14:
/// the owner saw the whole chat frame empty after a fold).
fn each_turn_fold(place: Place) {
    let mut chat = chat(place);
    let rows: Vec<usize> = turns(&chat.view.items).iter().map(|t| t.row).collect();
    for user in [rows[0], rows[rows.len() / 2], rows[rows.len() - 1]] {
        chat.fold_turn(user);
        chat.shot(&format!("{place:?}-fold-{user}"));
        chat.assert_drawn(&format!("{place:?} turn {user} folded"));
        chat.fold_turn(user);
        chat.assert_drawn(&format!("{place:?} turn {user} open again"));
    }
}

#[test]
fn the_first_middle_and_last_turn_fold_at_the_top() {
    each_turn_fold(Place::Top);
}

#[test]
fn the_first_middle_and_last_turn_fold_in_the_middle() {
    each_turn_fold(Place::Middle);
}

#[test]
fn the_first_middle_and_last_turn_fold_at_the_bottom() {
    each_turn_fold(Place::Bottom);
}

#[test]
fn a_navigator_click_after_a_fold_brings_the_prompt_to_the_top() {
    let mut chat = chat(Place::Bottom);
    let rows: Vec<usize> = turns(&chat.view.items).iter().map(|t| t.row).collect();
    chat.go(1);
    chat.fold_turn(rows[1]);
    for index in [0, 5, 1, 2] {
        chat.go(index);
        let top = chat.ui.global::<TurnsUi>().get_top_row() as usize;
        assert_eq!(top, rows[index], "tick {index} after the fold");
        // The prompt's bubble starts within the row's top gap.
        let first = chat.lines().iter().position(|&d| d).unwrap_or(usize::MAX);
        assert!(first < 30, "tick {index}: the first line draws at {first}");
        chat.assert_drawn(&format!("tick {index} after the fold"));
    }
}

#[test]
fn a_turn_folds_and_opens_at_the_top() {
    turn_fold(Place::Top);
}

#[test]
fn a_turn_folds_and_opens_in_the_middle() {
    turn_fold(Place::Middle);
}

#[test]
fn a_turn_folds_and_opens_at_the_bottom() {
    turn_fold(Place::Bottom);
}

#[test]
fn a_step_opens_and_folds_at_the_top() {
    step_fold(Place::Top);
}

#[test]
fn a_step_opens_and_folds_in_the_middle() {
    step_fold(Place::Middle);
}

#[test]
fn a_step_opens_and_folds_at_the_bottom() {
    step_fold(Place::Bottom);
}
