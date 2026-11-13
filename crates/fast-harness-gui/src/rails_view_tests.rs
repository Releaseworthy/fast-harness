//! Turn and step rails (D80) in a real (software rendered) window: a turn
//! folds through its user row and through a click on its rail, the rails
//! sit at their indents (turn rail at the column edge, step rail under the
//! step line's chevron, the open step's rows right of it) and every rail
//! is one unbroken line over its turn or step.

use std::cell::RefCell;
use std::rc::Rc;

use fast_harness_core::view::{Approvals, Transcript};
use slint::platform::software_renderer::MinimalSoftwareWindow;
use slint::platform::{PointerEventButton, WindowEvent};
use slint::{ComponentHandle, LogicalPosition, Model, Rgb8Pixel, VecModel};

use crate::mirror::Mirror;
use crate::turn_nav::turns;
use crate::turn_nav_view_tests::{frames, session, window, WIDTH};
use crate::{MainWindow, MsgRow, TurnsUi};

/// Tall enough for every row of two turns: the list measures them all.
const TALL: u32 = 2400;

struct Shot {
    pixels: Vec<Rgb8Pixel>,
    width: usize,
}

impl Shot {
    fn take(window: &MinimalSoftwareWindow) -> Shot {
        let size = window.size();
        let mut pixels = vec![Rgb8Pixel { r: 0, g: 0, b: 0 }; (size.width * size.height) as usize];
        window.request_redraw();
        window.draw_if_needed(|r| {
            r.render(&mut pixels, size.width as usize);
        });
        Shot {
            pixels,
            width: size.width as usize,
        }
    }
    fn at(&self, x: usize, y: usize) -> Rgb8Pixel {
        self.pixels[y * self.width + x]
    }
    /// The runs (first y, last y, uniform colour) of at least 20 px of
    /// pixels unlike the margin left of the column at column `x`.
    fn runs(&self, x: usize, left: usize, top: usize) -> Vec<(usize, usize, bool)> {
        // Above the composer.
        let height = self.pixels.len() / self.width - 300;
        let marked = |y: usize| self.at(x, y) != self.at(left - 4, y);
        let mut runs = Vec::new();
        let mut y = top;
        while y < height {
            if !marked(y) {
                y += 1;
                continue;
            }
            let start = y;
            while y < height && marked(y) {
                y += 1;
            }
            let uniform = (start..y).all(|i| self.at(x, i) == self.at(x, start));
            if y - start >= 20 {
                runs.push((start, y - 1, uniform));
            }
        }
        runs
    }
}

struct Chat {
    window: Rc<MinimalSoftwareWindow>,
    ui: MainWindow,
    view: Transcript,
    model: Rc<VecModel<MsgRow>>,
    mirror: Rc<RefCell<Mirror>>,
}

/// Two turns shown as the app shows them, the fold callbacks wired to the
/// mirror as `appearance.rs` wires them.
fn chat() -> Chat {
    let (window, ui) = window(WIDTH, TALL);
    let view = session(2);
    let model = Rc::new(VecModel::<MsgRow>::default());
    let mirror = Rc::new(RefCell::new(Mirror::new(false)));
    mirror
        .borrow_mut()
        .reset_with(&view.items, &Approvals::default(), &*model);
    ui.set_messages(model.clone().into());
    let (m, rows) = (mirror.clone(), model.clone());
    ui.on_turn_toggle(move |head| m.borrow_mut().toggle_turn(head as usize, &*rows));
    let (m, rows) = (mirror.clone(), model.clone());
    ui.on_step_toggle(move |head| m.borrow_mut().toggle_step(head as usize, &*rows));
    frames(&window, 100);
    Chat {
        window,
        ui,
        view,
        model,
        mirror,
    }
}

impl Chat {
    fn left(&self) -> usize {
        self.ui.global::<TurnsUi>().get_column_left() as usize
    }
    fn top(&self) -> usize {
        self.ui.global::<TurnsUi>().get_list_top() as usize + 1
    }
    fn click(&self, x: usize, y: usize) {
        let position = LogicalPosition::new(x as f32, y as f32);
        let button = PointerEventButton::Left;
        self.window
            .dispatch_event(WindowEvent::PointerMoved { position });
        self.window
            .dispatch_event(WindowEvent::PointerPressed { position, button });
        self.window
            .dispatch_event(WindowEvent::PointerReleased { position, button });
        self.window.dispatch_event(WindowEvent::PointerExited);
        frames(&self.window, 100);
    }
    /// The lowest transcript row that draws anything in the column.
    fn bottom(&self) -> usize {
        let (shot, left) = (Shot::take(&self.window), self.left());
        let height = shot.pixels.len() / shot.width - 300;
        (self.top()..height)
            .filter(|&y| (left..left + 600).any(|x| shot.at(x, y) != shot.at(left - 4, y)))
            .max()
            .unwrap_or(0)
    }
    fn hidden(&self, rows: std::ops::Range<usize>) -> bool {
        rows.map(|i| self.model.row_data(i).expect("row"))
            .all(|r| r.turn_hidden && !r.turn_rail)
    }
}

#[test]
fn a_turn_folds_by_its_user_row_and_its_rows_hide() {
    let chat = chat();
    let before = chat.bottom();
    // The navigator's turn row is the user row: the turn starts after it.
    let user = turns(&chat.view.items)[1].row;
    chat.mirror.borrow_mut().toggle_turn(user, &*chat.model);
    frames(&chat.window, 100);
    assert!(chat.hidden(user + 1..chat.model.row_count()));
    let line = chat.model.row_data(user + 1).expect("row").turn_line;
    assert!(line.starts_with("1 step · Answer 2"), "{line}");
    let after = chat.bottom();
    assert!(
        after + 150 < before,
        "the turn's rows still draw: {before} -> {after}"
    );
}

#[test]
fn the_rows_after_a_folded_turn_fill_the_view() {
    let (window, ui) = window(WIDTH, crate::turn_nav_view_tests::HEIGHT);
    let view = session(6);
    let model = Rc::new(VecModel::<MsgRow>::default());
    let mut mirror = Mirror::new(false);
    mirror.reset_with(&view.items, &Approvals::default(), &*model);
    ui.set_messages(model.clone().into());
    frames(&window, 100);
    for turn in turns(&view.items).iter().take(2) {
        mirror.toggle_turn(turn.row, &*model);
        frames(&window, 100);
    }
    let shot = Shot::take(&window);
    let left = ui.global::<TurnsUi>().get_column_left() as usize;
    let drawn = |y: usize| (left..left + 600).any(|x| shot.at(x, y) != shot.at(left - 4, y));
    let blank = (300..440).filter(|&y| !drawn(y)).count();
    assert!(
        blank < 80,
        "{blank} of 140 rows blank below two folded turns"
    );
}

#[test]
fn a_click_on_the_turn_rail_folds_the_turn_and_its_line_opens_it() {
    let chat = chat();
    let x = chat.left() + 5;
    let runs = Shot::take(&chat.window).runs(x, chat.left(), chat.top());
    assert_eq!(runs.len(), 2, "one turn rail per turn: {runs:?}");
    chat.click(x + 1, (runs[0].0 + runs[0].1) / 2);
    assert!(chat.hidden(1..5) && !chat.hidden(6..10));
    let runs = Shot::take(&chat.window).runs(x, chat.left(), chat.top());
    assert_eq!(runs.len(), 1, "the folded turn has no rail: {runs:?}");
    // The folded turn's line: its chevron is the first mark at the edge.
    let shot = Shot::take(&chat.window);
    let left = chat.left();
    let y = (chat.top()..runs[0].0)
        .find(|&y| (left..left + 12).any(|x| shot.at(x, y) != shot.at(left - 4, y)))
        .expect("the turn line");
    chat.click(left + 60, y + 4);
    assert!(!chat.hidden(1..2), "the turn line opens the turn again");
}

#[test]
fn rails_sit_at_their_indents_and_run_unbroken() {
    let chat = chat();
    chat.mirror.borrow_mut().toggle_step(1, &*chat.model);
    frames(&chat.window, 100);
    let (shot, left, top) = (Shot::take(&chat.window), chat.left(), chat.top());
    // The turn rail at the column edge, the step rail one indent in.
    for x in [left + 5, left + 6] {
        let runs = shot.runs(x, left, top);
        assert_eq!(runs.len(), 2, "turn rail at {x}: {runs:?}");
        assert!(runs.iter().all(|r| r.2), "a turn rail breaks: {runs:?}");
    }
    assert!(shot.runs(left + 4, left, top).is_empty() && shot.runs(left + 7, left, top).is_empty());
    // Turn 1's step is open; turn 2's rows (folded step) start at its
    // indent and draw their speaker tile across that column.
    let turn = shot.runs(left + 5, left, top)[0];
    let in_turn = |x: usize| -> Vec<_> {
        (shot.runs(x, left, top).into_iter())
            .filter(|r| r.0 < turn.1)
            .collect()
    };
    let step = in_turn(left + 25);
    assert_eq!(step.len(), 1, "one open step: {step:?}");
    assert!(step[0].2, "the step rail breaks: {step:?}");
    assert_eq!(in_turn(left + 26).len(), 1);
    let (start, end) = (step[0].0, step[0].1);
    // The step line's chevron centred over the step rail.
    let marks: Vec<usize> = (start - 20..start - 4)
        .flat_map(|y| (left + 18..left + 34).map(move |x| (x, y)))
        .filter(|&(x, y)| shot.at(x, y) != shot.at(left - 4, y))
        .map(|(x, _)| x)
        .collect();
    assert!(!marks.is_empty(), "no chevron above the step rail");
    let centre = marks.iter().sum::<usize>() as f32 / marks.len() as f32;
    assert!(
        (centre - 26.0 - left as f32).abs() <= 1.5,
        "chevron at {centre}, rail at {}",
        left + 26
    );
    // The open step's rows start two indents in, right of the step rail.
    let first = (start..end)
        .filter_map(|y| (left + 27..left + 200).find(|&x| shot.at(x, y) != shot.at(left - 4, y)))
        .min()
        .expect("a detail row");
    assert!(
        (left + 38..=left + 42).contains(&first),
        "details at {first}, column {left}"
    );
}

/// The rails shot's order (2026-11-14: rows after the fold stayed blank
/// until a scroll): a navigator click to turn 2, the fold of turn 2, a
/// click back to turn 1. The folded rows kept their height (max-height 0
/// lost to their wrapped text's min height): they now build no content.
#[test]
fn the_rows_after_a_turn_folded_while_scrolled_draw_at_once() {
    use crate::turn_nav_view_tests::tick;
    use slint::platform::{PointerEventButton, WindowEvent};
    let (window, ui) = window(WIDTH, crate::turn_nav_view_tests::HEIGHT);
    let view = session(8);
    let model = crate::turn_nav_view_tests::show(&ui, &view);
    let mut mirror = Mirror::new(false);
    mirror.reset_with(&view.items, &Approvals::default(), &*model);
    frames(&window, 100);
    let go = |index: usize| {
        let (position, button) = (tick(&ui, index), PointerEventButton::Left);
        window.dispatch_event(WindowEvent::PointerPressed { position, button });
        window.dispatch_event(WindowEvent::PointerReleased { position, button });
        window.dispatch_event(WindowEvent::PointerExited);
        frames(&window, 300);
    };
    go(1);
    let heads: Vec<usize> = turns(&view.items).iter().map(|t| t.row).collect();
    mirror.toggle_turn(heads[1], &*model);
    frames(&window, 100);
    go(0);
    let shot = Shot::take(&window);
    let left = ui.global::<TurnsUi>().get_column_left() as usize;
    let drawn = |y: usize| (left..left + 600).any(|x| shot.at(x, y) != shot.at(left - 4, y));
    let blank = (320..560).filter(|&y| !drawn(y)).count();
    assert!(
        blank < 120,
        "{blank} of 240 rows blank below the folded turn"
    );
}
