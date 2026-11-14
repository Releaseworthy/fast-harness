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
use crate::{MainWindow, MsgRow, Rails, TurnsUi};

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
    for turn in [true, false] {
        let (m, rows, w) = (mirror.clone(), model.clone(), ui.as_weak());
        let fold = move |head| {
            let ui = w.upgrade().expect("the window");
            crate::appearance::fold(&ui, &mut m.borrow_mut(), &*rows, head, turn);
        };
        if turn {
            ui.on_turn_toggle(fold);
        } else {
            ui.on_step_toggle(fold);
        }
    }
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
    /// A click that leaves the pointer where it is (no `PointerExited`): a
    /// rail that folds under the pointer never hears it leave.
    fn press(&self, x: usize, y: usize) {
        let position = LogicalPosition::new(x as f32, y as f32);
        let button = PointerEventButton::Left;
        self.window
            .dispatch_event(WindowEvent::PointerMoved { position });
        // The hover is drawn (its `changed` handlers run) before the press.
        frames(&self.window, 100);
        self.window
            .dispatch_event(WindowEvent::PointerPressed { position, button });
        self.window
            .dispatch_event(WindowEvent::PointerReleased { position, button });
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
    // The last turn: its line sits on the last row.
    let line = chat
        .model
        .row_data(chat.model.row_count() - 1)
        .expect("row")
        .turn_line;
    assert!(line.starts_with("1 step · Answer 2"), "{line}");
    let after = chat.bottom();
    // 100 px, not 150: the answer rows lost their speaker tile and name
    // (22 px and a gap, 2026-11-14); with them back this passed at 150.
    assert!(
        after + 100 < before,
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
    // The turn rail's line sits in the gutter, 25 px left of the text.
    let x = chat.left() - 25;
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
fn rails_sit_in_the_gutter_and_the_text_starts_at_the_column() {
    let chat = chat();
    chat.mirror.borrow_mut().toggle_step(1, &*chat.model);
    frames(&chat.window, 100);
    let (shot, left, top) = (Shot::take(&chat.window), chat.left(), chat.top());
    // The turn rail 25 px left of the text column, unbroken over each turn.
    for x in [left - 25, left - 24] {
        let runs = shot.runs(x, left, top);
        assert_eq!(runs.len(), 2, "turn rail at {x}: {runs:?}");
        assert!(runs.iter().all(|r| r.2), "a turn rail breaks: {runs:?}");
    }
    // The open run's rail 11 px left of the text column.
    let turn = shot.runs(left - 25, left, top)[0];
    let step: Vec<_> = (shot.runs(left - 11, left, top).into_iter())
        .filter(|r| r.0 < turn.1)
        .collect();
    assert_eq!(step.len(), 1, "one open run: {step:?}");
    // No rail or row indents the text: the first mark right of the
    // gutter in turn 1 sits at the column's left edge (the right inset).
    let first = (turn.0..turn.1)
        .filter_map(|y| (left - 6..left + 200).find(|&x| shot.at(x, y) != shot.at(left - 4, y)))
        .min()
        .expect("a text row");
    assert!(
        (left.saturating_sub(1)..=left + 3).contains(&first),
        "text at {first}, column {left}"
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

/// The middle row of a run.
fn mid(run: (usize, usize, bool)) -> usize {
    (run.0 + run.1) / 2
}

/// A rail folded by a click under the pointer (2026-11-15: the owner saw
/// the reopened turn's rail drawn as if hovered): it rests, 2 px wide and
/// faint like the untouched turn's.
#[test]
fn a_turn_rail_folded_under_the_pointer_rests_when_the_turn_opens() {
    let chat = chat();
    let (x, left, top) = (chat.left() - 25, chat.left(), chat.top());
    let runs = Shot::take(&chat.window).runs(x, left, top);
    chat.press(x + 1, mid(runs[0]));
    assert!(chat.hidden(1..5), "the rail folds the turn");
    let shot = Shot::take(&chat.window);
    let y = (top..shot.runs(x, left, top)[0].0)
        .find(|&y| (left..left + 12).any(|x| shot.at(x, y) != shot.at(left - 4, y)))
        .expect("the turn line");
    chat.click(left + 60, y + 4);
    assert!(!chat.hidden(1..2), "the turn line opens the turn again");
    let shot = Shot::take(&chat.window);
    let runs = shot.runs(x, left, top);
    assert_eq!(runs.len(), 2, "{runs:?}");
    let (a, b) = (mid(runs[0]), mid(runs[1]));
    assert_eq!(shot.at(x, a), shot.at(x, b), "the reopened rail is lit");
    assert_eq!(shot.at(x - 1, a), shot.at(left - 4, a), "3 px wide");
}

/// The owner's order in the real window (2026-11-15): the pointer on the
/// turn rail lights it, the turn folds, the pointer goes to the turn line
/// and the turn opens again. The folded rail is gone before it hears the
/// pointer leave (no hover-off runs: the fold comes through the callback
/// with no pointer event), so a lit state left in `Rails` came back on the
/// reopened rows, dark and wide.
#[test]
fn a_rail_lit_when_its_turn_folds_rests_when_the_turn_opens() {
    let chat = chat();
    let (x, left, top) = (chat.left() - 25, chat.left(), chat.top());
    let shot = Shot::take(&chat.window);
    let rest = shot.at(x, mid(shot.runs(x, left, top)[1]));
    let move_to = |x: usize, y: usize| {
        let position = LogicalPosition::new(x as f32, y as f32);
        chat.window
            .dispatch_event(WindowEvent::PointerMoved { position });
        frames(&chat.window, 100);
    };
    move_to(x + 1, mid(shot.runs(x, left, top)[0]));
    let head = chat.ui.global::<Rails>().get_turn();
    assert!(head >= 0, "the hover lights the rail");
    chat.ui.invoke_turn_toggle(head);
    frames(&chat.window, 100);
    assert!(chat.hidden(1..5), "the turn folds");
    move_to(left + 60, top + 4);
    chat.ui.invoke_turn_toggle(head);
    frames(&chat.window, 100);
    let shot = Shot::take(&chat.window);
    let runs = shot.runs(x, left, top);
    assert_eq!(runs.len(), 2, "{runs:?}");
    for run in runs {
        assert_eq!(shot.at(x, mid(run)), rest, "the reopened rail rests");
    }
    assert_eq!(chat.ui.global::<Rails>().get_turn(), -1, "no rail is lit");
}

#[test]
fn a_step_rail_folded_under_the_pointer_rests_when_the_step_opens() {
    let chat = chat();
    chat.mirror.borrow_mut().toggle_step(1, &*chat.model);
    frames(&chat.window, 100);
    let (x, left, top) = (chat.left() - 11, chat.left(), chat.top());
    let shot = Shot::take(&chat.window);
    let turn = shot.runs(left - 25, left, top)[0];
    let open = |shot: &Shot| -> Vec<_> {
        (shot.runs(x, left, top).into_iter())
            .filter(|r| r.0 < turn.1)
            .collect()
    };
    let y = mid(open(&shot)[0]);
    let rest = shot.at(x, y);
    chat.press(x, y);
    assert!(open(&Shot::take(&chat.window)).is_empty(), "the rail folds");
    chat.mirror.borrow_mut().toggle_step(1, &*chat.model);
    frames(&chat.window, 100);
    let shot = Shot::take(&chat.window);
    assert_eq!(open(&shot).len(), 1, "the step opens again");
    assert_eq!(shot.at(x, y), rest, "the reopened rail is lit");
    assert_eq!(shot.at(x - 1, y), shot.at(left - 4, y), "3 px wide");
}
