//! The chat list on the owner's real session (2026-11-14, his reports on
//! the ListView build at be88016): the scroll bar's thumb changed size as
//! the view moved, a drag of the thumb jumped, and a fold or an unfold
//! moved the view away from the clicked row. The parent log and its four
//! children are copied (read-only source) into a temp home and opened
//! through the window's real paths, as `real_child_tests` does; the name
//! holds `real_child`, so the real-session gate runs these too.

use std::path::{Path, PathBuf};
use std::rc::Rc;
use std::time::{Duration, Instant};

use fast_harness_core::session;
use slint::platform::software_renderer::MinimalSoftwareWindow;
use slint::platform::{PointerEventButton, WindowEvent};
use slint::{ComponentHandle, LogicalPosition, Model, Rgb8Pixel};

use fast_harness_core::view::Approvals;
use slint::{FilterModel, ModelRc, VecModel};
use std::cell::RefCell;

use crate::fold_view_tests::shot;
use crate::mirror::Mirror;
use crate::real_child_tests::{home, open_parent, pump, window};
use crate::{MainWindow, MsgRow, TurnsUi};

const OWNER: &str = r"C:\Users\Yamasaki\.fast-harness";
const PARENT: &str = "20261113-195004-6140";
pub(crate) const CHILDREN: [&str; 4] = [
    "20261113-195342-6141",
    "20261113-195342-6142",
    "20261113-195342-6143",
    "20261113-195342-6144",
];
/// `watch-y` until the watched row reports: it is not laid out.
const FAR: f32 = -100_000.0;

pub(crate) struct Chat {
    pub(crate) window: Rc<MinimalSoftwareWindow>,
    pub(crate) ui: MainWindow,
    dir: PathBuf,
}

impl Drop for Chat {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.dir);
    }
}

/// The owner's parent session opened in a fresh home, settled.
pub(crate) fn open(tag: &str) -> Chat {
    let id = session::new_id();
    let dir = std::env::temp_dir().join(format!("fh-real-child-scroll-{tag}-{id}"));
    let cwd = PathBuf::from(r"F:\work\fast-harness-general");
    let from = session::dir(Path::new(OWNER), &cwd);
    let mut files = vec![format!("{PARENT}.jsonl"), "index.json".to_string()];
    files.extend(CHILDREN.iter().map(|c| format!("{c}.jsonl")));
    let names: Vec<&str> = files.iter().map(String::as_str).collect();
    let config = Path::new(OWNER).join("config.json");
    let home = home(&dir, &cwd, &from, &names, Some(&config));
    let (window, ui) = window();
    let start = Instant::now();
    open_parent(&window, &ui, &home, &cwd, PARENT, CHILDREN.len());
    let chat = Chat { window, ui, dir };
    chat.settle();
    let (rows, content, view) = (chat.rows().len(), chat.content(), chat.view());
    let took = start.elapsed();
    eprintln!("{tag}: {rows} rows, content {content:.0} px, view {view:.0} px, open {took:?}");
    chat
}

fn top_pad(row: &MsgRow) -> f32 {
    match row.kind.as_str() {
        "user" => 16.0,
        "footer" => 0.0,
        _ => 8.0,
    }
}

#[derive(Clone, Copy, Debug)]
pub(crate) enum Place {
    Bottom,
    Middle,
    Top,
}

impl Chat {
    /// Pumps until the content height holds for 10 frames (at most 10 s).
    pub(crate) fn settle(&self) {
        let start = Instant::now();
        let (mut last, mut same) = (f32::NAN, 0);
        while same < 10 && start.elapsed() < Duration::from_secs(10) {
            pump(&self.window, 16);
            let now = self.content();
            same = if (now - last).abs() < 0.5 {
                same + 1
            } else {
                0
            };
            last = now;
        }
    }
    pub(crate) fn rows(&self) -> Vec<MsgRow> {
        let model = self.ui.get_messages();
        (0..model.row_count())
            .filter_map(|i| model.row_data(i))
            .collect()
    }
    pub(crate) fn content(&self) -> f32 {
        self.ui.get_transcript_content_height()
    }
    pub(crate) fn view(&self) -> f32 {
        self.ui.get_chat_list_height()
    }
    pub(crate) fn y(&self) -> f32 {
        self.ui.get_chat_view_y()
    }
    pub(crate) fn top(&self) -> f32 {
        self.ui.global::<TurnsUi>().get_list_top()
    }
    pub(crate) fn left(&self) -> f32 {
        self.ui.global::<TurnsUi>().get_column_left()
    }
    /// One frame (timers, change handlers, layout, a full render): its time.
    pub(crate) fn frame(&self) -> Duration {
        let size = self.window.size();
        let mut buffer = vec![Rgb8Pixel { r: 0, g: 0, b: 0 }; (size.width * size.height) as usize];
        let start = Instant::now();
        slint::platform::update_timers_and_animations();
        self.window.request_redraw();
        self.window.draw_if_needed(|r| {
            r.render(&mut buffer, size.width as usize);
        });
        start.elapsed()
    }
    pub(crate) fn move_to(&self, x: f32, y: f32) {
        let position = LogicalPosition::new(x, y);
        self.window
            .dispatch_event(WindowEvent::PointerMoved { position });
    }
    pub(crate) fn press(&self, x: f32, y: f32) {
        self.move_to(x, y);
        let (position, button) = (LogicalPosition::new(x, y), PointerEventButton::Left);
        self.window
            .dispatch_event(WindowEvent::PointerPressed { position, button });
    }
    pub(crate) fn release(&self, x: f32, y: f32) {
        let (position, button) = (LogicalPosition::new(x, y), PointerEventButton::Left);
        self.window
            .dispatch_event(WindowEvent::PointerReleased { position, button });
        self.window.dispatch_event(WindowEvent::PointerExited);
    }
    pub(crate) fn click(&self, x: f32, y: f32) {
        self.press(x, y);
        self.release(x, y);
        pump(&self.window, 300);
    }
    /// A click on the bar's track below (or above) the thumb: one page.
    pub(crate) fn page(&self, down: bool) -> Duration {
        let x = self.ui.get_transcript_thumb_x();
        let y = if down {
            self.top() + self.view() - 3.0
        } else {
            self.top() + 3.0
        };
        self.press(x, y);
        self.release(x, y);
        let took = self.frame();
        pump(&self.window, 48);
        took
    }
    /// Pages to the top or the end; the number of pages.
    pub(crate) fn to_end(&self, down: bool) -> usize {
        for n in 1..3000 {
            let before = self.y();
            self.page(down);
            if (self.y() - before).abs() < 0.5 {
                return n;
            }
        }
        3000
    }
    pub(crate) fn go_to(&self, place: Place) {
        match place {
            Place::Bottom => {
                self.to_end(true);
            }
            Place::Top => {
                self.to_end(false);
            }
            Place::Middle => {
                self.to_end(false);
                let rows = self.rows();
                let lines = rows
                    .iter()
                    .filter(|r| r.turn_rail && !r.step_summary.is_empty());
                let lines: Vec<i32> = lines.map(|r| r.item).collect();
                let Some(&mid) = lines.get(lines.len() / 2) else {
                    return;
                };
                for _ in 0..200 {
                    let y = self.row_y(mid);
                    if (0.0..=0.6 * self.view()).contains(&y) {
                        break;
                    }
                    self.wheel(-120.0);
                }
            }
        }
        pump(&self.window, 200);
    }
    /// A wheel step over the list's middle, the view settled after it.
    pub(crate) fn wheel(&self, delta_y: f32) {
        let position = LogicalPosition::new(self.left() + 100.0, self.top() + self.view() / 2.0);
        self.window
            .dispatch_event(WindowEvent::PointerMoved { position });
        let event = WindowEvent::PointerScrolled {
            position,
            delta_x: 0.0,
            delta_y,
        };
        self.window.dispatch_event(event);
        self.window.dispatch_event(WindowEvent::PointerExited);
        pump(&self.window, 200);
    }
    /// The clicked row (or the line in its place) kept its top within 2 px.
    /// Only an end of the list may move it, the view then at that end: a row
    /// cannot keep its place with too few rows above it (the view at the
    /// top, the row moved up) or below it (the list's end at the view's
    /// bottom, the row moved down) without blank space.
    pub(crate) fn check(
        &self,
        place: Place,
        fails: &mut Vec<String>,
        what: String,
        before: f32,
        after: f32,
    ) {
        eprintln!("{what}: y {before:.1} -> {after:.1}");
        if (after - before).abs() <= 2.0 {
            return;
        }
        let (end, bottom) = (self.ui.get_chat_end(), self.top() + self.view());
        let at_top = self.y().abs() < 0.5 && after < before && after > FAR;
        if at_top || ((end - bottom).abs() <= 2.0 && after > before) {
            eprintln!("{what} ({place:?}): moved by an end of the list (top {at_top}, end {end:.0}, bottom {bottom:.0})");
            return;
        }
        fails.push(format!("{what}: was at y {before:.1}, now at {after:.1}"));
    }
    /// The top of row `item` in the view, [`FAR`] when it is not laid out.
    pub(crate) fn row_y(&self, item: i32) -> f32 {
        self.ui.set_chat_watch_item(-1);
        self.ui.set_chat_watch_y(FAR);
        pump(&self.window, 16);
        self.ui.set_chat_watch_item(item);
        pump(&self.window, 32);
        self.ui.get_chat_watch_y()
    }
    /// A row that `want`s, whose top sits in
    /// `band` (shares of the view); the lowest one first when `last`.
    pub(crate) fn pick(
        &self,
        band: (f32, f32),
        last: bool,
        want: impl Fn(&MsgRow) -> bool,
    ) -> Option<(MsgRow, f32)> {
        let rows = self.rows();
        let mut near: Vec<&MsgRow> = rows.iter().filter(|r| want(r)).collect();
        if last {
            near.reverse();
        }
        let view = self.view();
        near.into_iter().find_map(|r| {
            let y = self.row_y(r.item);
            (y >= band.0 * view && y <= band.1 * view).then(|| (r.clone(), y))
        })
    }
    /// Folds a turn by a click on its rail, then opens it by a click on
    /// its line: the line takes the clicked row's place, then stays put.
    pub(crate) fn turn_round_trip(&self, place: Place, fails: &mut Vec<String>) {
        let top = matches!(place, Place::Top);
        let band = if top { (0.0, 0.5) } else { (-6.0, 0.95) };
        let pick = self.pick(band, !top, |r| {
            r.turn_rail && (!top || r.item == r.turn_head)
        });
        let Some((row, y)) = pick else {
            fails.push(format!("{place:?}: no open turn's row in view"));
            return;
        };
        let follow = self.ui.get_transcript_follow();
        let what = format!("{place:?} (following {follow}) turn {}", row.turn_head);
        shot(&self.window, &format!("fold-{place:?}-turn-before"));
        let dy = if row.item == row.turn_head { 20.0 } else { 4.0 };
        self.click(self.left() + 6.0, self.top() + y.max(0.0) + dy);
        let rows = self.rows();
        let line = rows
            .iter()
            .find(|r| r.turn_head == row.turn_head && !r.turn_line.is_empty());
        let Some(line) = line.cloned() else {
            fails.push(format!(
                "{what}: a click on row {}'s rail folded nothing",
                row.item
            ));
            return;
        };
        let at = self.row_y(line.item);
        shot(&self.window, &format!("fold-{place:?}-turn-folded"));
        let rail = format!(
            "{what} fold at row {} (its line on row {})",
            row.item, line.item
        );
        self.check(place, fails, rail, y, at);
        let follow = self.ui.get_transcript_follow();
        let what = format!("{place:?} (following {follow}) turn {}", row.turn_head);
        let indent = if line.turn_rail { 20.0 } else { 0.0 };
        self.click(
            self.left() + indent + 60.0,
            self.top() + at + top_pad(&line) + 13.0,
        );
        let rows = self.rows();
        if rows
            .iter()
            .any(|r| r.turn_head == row.turn_head && !r.turn_line.is_empty())
        {
            fails.push(format!("{what}: a click on its line did not open it"));
            return;
        }
        let anchor = rows
            .iter()
            .rev()
            .find(|r| r.item <= line.item)
            .map_or(line.item, |r| r.item);
        let again = self.row_y(anchor);
        shot(&self.window, &format!("fold-{place:?}-turn-opened"));
        self.check(
            place,
            fails,
            format!("{what} open at its line (row {})", line.item),
            at,
            again,
        );
    }
    /// Opens a folded step by a click on its line and folds it again: the
    /// line stays put both times.
    pub(crate) fn step_round_trip(&self, place: Place, fails: &mut Vec<String>) {
        let top = matches!(place, Place::Top);
        let band = if top { (0.0, 0.6) } else { (0.0, 0.95) };
        let pick = self.pick(band, !top, |r| !r.step_summary.is_empty() && !r.step_open);
        let Some((row, y)) = pick else {
            fails.push(format!("{place:?}: no folded step's line in view"));
            return;
        };
        let indent = if row.turn_rail { 20.0 } else { 0.0 };
        let (x, pad) = (self.left() + indent + 60.0, top_pad(&row) + 13.0);
        let is_open = |want: bool| {
            self.rows()
                .iter()
                .any(|r| r.item == row.item && r.step_open == want)
        };
        for opening in [true, false] {
            let (follow, verb) = (
                self.ui.get_transcript_follow(),
                if opening { "open" } else { "fold" },
            );
            let what = format!("{place:?} (following {follow}) step {} {verb}", row.item);
            let before = if opening { y } else { self.row_y(row.item) };
            self.click(x, self.top() + before + pad);
            if !is_open(opening) {
                fails.push(format!("{what}: a click on its line did nothing"));
                return;
            }
            let after = self.row_y(row.item);
            shot(&self.window, &format!("fold-{place:?}-step-{verb}"));
            self.check(place, fails, what, before, after);
        }
    }
}

impl Chat {
    /// The shown session's rows through a mirror of the test's own, whose
    /// toggles the clicks reach: the app's toggle handlers are connected by
    /// `wire` at startup, which a test window does not run.
    pub(crate) fn own_rows(&self) {
        let items =
            crate::app::with_app(|app, _| app.runs.shown.view.items.clone()).expect("the app");
        let model = Rc::new(VecModel::<MsgRow>::default());
        let mirror = Rc::new(RefCell::new(Mirror::new(false)));
        mirror
            .borrow_mut()
            .reset_with(&items, &Approvals::default(), &*model);
        let shown = FilterModel::new(model.clone(), crate::rows::shows);
        self.ui.set_messages(ModelRc::new(shown));
        let (m, rows) = (mirror.clone(), model.clone());
        self.ui
            .on_turn_toggle(move |head| m.borrow_mut().toggle_turn(head as usize, &*rows));
        let (m, rows) = (mirror, model);
        self.ui
            .on_step_toggle(move |head| m.borrow_mut().toggle_step(head as usize, &*rows));
        self.settle();
    }
}
/// Report 4: a fold or an unfold of a turn or a step keeps the clicked
/// row where it was (within 2 px), at the end while following, in the
/// middle and at the top.
#[test]
#[ignore = "reads the owner's sessions under C:\\Users\\Yamasaki"]
fn real_child_scroll_a_fold_keeps_the_clicked_row_in_place() {
    let chat = open("fold");
    chat.own_rows();
    assert!(
        chat.ui.get_transcript_follow(),
        "the session opens following its end"
    );
    let mut fails = Vec::new();
    for place in [Place::Bottom, Place::Middle, Place::Top] {
        if !matches!(place, Place::Bottom) {
            chat.go_to(place);
        }
        chat.turn_round_trip(place, &mut fails);
        // The parent's last turn has no steps: the child below has.
        if !matches!(place, Place::Bottom) {
            chat.go_to(place);
            chat.step_round_trip(place, &mut fails);
        }
    }
    // The largest child (6143) opens at its end, following: a step there.
    crate::real_child_tests::click_child(&chat.window, CHILDREN[2]);
    chat.own_rows();
    chat.step_round_trip(Place::Bottom, &mut fails);
    assert!(
        fails.is_empty(),
        "a fold moved the clicked row:\n{}",
        fails.join("\n")
    );
}
