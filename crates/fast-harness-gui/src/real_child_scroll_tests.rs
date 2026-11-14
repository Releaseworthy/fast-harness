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

use crate::fold_view_tests::shot;
use crate::real_child_tests::{click_child, home, open_parent, pump, window};
use crate::{MainWindow, MsgRow, TurnsUi};

const OWNER: &str = r"C:\Users\Yamasaki\.fast-harness";
const PARENT: &str = "20261113-195004-6140";
const CHILDREN: [&str; 4] = [
    "20261113-195342-6141",
    "20261113-195342-6142",
    "20261113-195342-6143",
    "20261113-195342-6144",
];
/// `watch-y` until the watched row reports: it is not laid out.
const FAR: f32 = -100_000.0;

struct Chat {
    window: Rc<MinimalSoftwareWindow>,
    ui: MainWindow,
    dir: PathBuf,
}

impl Drop for Chat {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.dir);
    }
}

/// The owner's parent session opened in a fresh home, settled.
fn open(tag: &str) -> Chat {
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

fn check(fails: &mut Vec<String>, what: String, before: f32, after: f32) {
    eprintln!("{what}: y {before:.1} -> {after:.1}");
    if (after - before).abs() > 2.0 {
        fails.push(format!("{what}: was at y {before:.1}, now at {after:.1}"));
    }
}

#[derive(Clone, Copy, Debug)]
enum Place {
    Bottom,
    Middle,
    Top,
}

impl Chat {
    /// Pumps until the content height holds for 10 frames (at most 10 s).
    fn settle(&self) {
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
    fn rows(&self) -> Vec<MsgRow> {
        let model = self.ui.get_messages();
        (0..model.row_count())
            .filter_map(|i| model.row_data(i))
            .collect()
    }
    fn content(&self) -> f32 {
        self.ui.get_transcript_content_height()
    }
    fn view(&self) -> f32 {
        self.ui.get_chat_list_height()
    }
    fn y(&self) -> f32 {
        self.ui.get_chat_view_y()
    }
    fn top(&self) -> f32 {
        self.ui.global::<TurnsUi>().get_list_top()
    }
    fn left(&self) -> f32 {
        self.ui.global::<TurnsUi>().get_column_left()
    }
    /// One frame (timers, change handlers, layout, a full render): its time.
    fn frame(&self) -> Duration {
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
    fn move_to(&self, x: f32, y: f32) {
        let position = LogicalPosition::new(x, y);
        self.window
            .dispatch_event(WindowEvent::PointerMoved { position });
    }
    fn press(&self, x: f32, y: f32) {
        self.move_to(x, y);
        let (position, button) = (LogicalPosition::new(x, y), PointerEventButton::Left);
        self.window
            .dispatch_event(WindowEvent::PointerPressed { position, button });
    }
    fn release(&self, x: f32, y: f32) {
        let (position, button) = (LogicalPosition::new(x, y), PointerEventButton::Left);
        self.window
            .dispatch_event(WindowEvent::PointerReleased { position, button });
        self.window.dispatch_event(WindowEvent::PointerExited);
    }
    fn click(&self, x: f32, y: f32) {
        self.press(x, y);
        self.release(x, y);
        pump(&self.window, 300);
    }
    /// A click on the bar's track below (or above) the thumb: one page.
    fn page(&self, down: bool) -> Duration {
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
    fn to_end(&self, down: bool) -> usize {
        for n in 1..3000 {
            let before = self.y();
            self.page(down);
            if (self.y() - before).abs() < 0.5 {
                return n;
            }
        }
        3000
    }
    fn go_to(&self, place: Place) {
        match place {
            Place::Bottom => {
                self.to_end(true);
            }
            Place::Top => {
                self.to_end(false);
            }
            Place::Middle => {
                self.to_end(false);
                let half = (self.content() / self.view() / 0.9 / 2.0) as usize;
                for _ in 0..half {
                    self.page(true);
                }
            }
        }
        pump(&self.window, 200);
    }
    /// The top of row `item` in the view, [`FAR`] when it is not laid out.
    fn row_y(&self, item: i32) -> f32 {
        self.ui.set_chat_watch_item(-1);
        self.ui.set_chat_watch_y(FAR);
        pump(&self.window, 16);
        self.ui.set_chat_watch_item(item);
        pump(&self.window, 32);
        self.ui.get_chat_watch_y()
    }
    /// A row near the view's top row that `want`s, whose top sits in
    /// `band` (shares of the view); the lowest one first when `last`.
    fn pick(
        &self,
        band: (f32, f32),
        last: bool,
        want: impl Fn(&MsgRow) -> bool,
    ) -> Option<(MsgRow, f32)> {
        let rows = self.rows();
        let top = self.ui.global::<TurnsUi>().get_top_row();
        let start = rows.iter().position(|r| r.item >= top).unwrap_or(0);
        let mut near: Vec<&MsgRow> = rows[start..].iter().take(40).filter(|r| want(r)).collect();
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
    fn turn_round_trip(&self, place: Place, fails: &mut Vec<String>) {
        let top = matches!(place, Place::Top);
        let band = if top { (0.0, 0.5) } else { (0.25, 0.75) };
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
        self.click(self.left() + 6.0, self.top() + y + dy);
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
        check(fails, rail, y, at);
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
        check(
            fails,
            format!("{what} open at its line (row {})", line.item),
            at,
            again,
        );
    }
    /// Opens a folded step by a click on its line and folds it again: the
    /// line stays put both times.
    fn step_round_trip(&self, place: Place, fails: &mut Vec<String>) {
        let top = matches!(place, Place::Top);
        let band = if top { (0.0, 0.6) } else { (0.2, 0.7) };
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
            check(fails, what, before, after);
        }
    }
}

/// Pages up to the top, down to the end and up again (the view opens at
/// the end): an error when the view/content ratio (the thumb's share of
/// its track) moves more than 3 %.
fn sweep(chat: &Chat, what: &str) -> Result<(), String> {
    let mut ratios = vec![chat.view() / chat.content()];
    let mut frames = Vec::new();
    for down in [false, true, false] {
        for _ in 0..3000 {
            let before = chat.y();
            frames.push(chat.page(down));
            ratios.push(chat.view() / chat.content());
            if (chat.y() - before).abs() < 0.5 {
                break;
            }
        }
        shot(
            &chat.window,
            &format!("thumb-{what}-{}", if down { "end" } else { "top" }),
        );
    }
    frames.sort();
    let fold = |(lo, hi): (f32, f32), r: &f32| (lo.min(*r), hi.max(*r));
    let (lo, hi) = ratios.iter().fold((f32::MAX, f32::MIN), fold);
    let swing = (hi - lo) / hi * 100.0;
    let (pages, median, slowest) = (
        frames.len(),
        frames[frames.len() / 2],
        frames[frames.len() - 1],
    );
    eprintln!("{what}: {pages} pages, view/content {lo:.4} to {hi:.4} ({swing:.1} %), frame median {median:?}, slowest {slowest:?}");
    if swing <= 3.0 {
        return Ok(());
    }
    Err(format!("{what}: the thumb changes size while scrolling: view/content from {lo:.4} to {hi:.4} ({swing:.1} %) over {pages} pages"))
}

/// Report 2: the thumb kept changing size, on the parent and on the
/// largest child log (6143, 1.1 MB).
#[test]
#[ignore = "reads the owner's sessions under C:\\Users\\Yamasaki"]
fn real_child_scroll_the_thumb_keeps_its_size() {
    let chat = open("thumb");
    let parent = sweep(&chat, "parent");
    click_child(&chat.window, CHILDREN[2]);
    chat.settle();
    let (rows, content) = (chat.rows().len(), chat.content());
    eprintln!(
        "child {}: {rows} rows, content {content:.0} px",
        CHILDREN[2]
    );
    let child = sweep(&chat, "child-6143");
    let fails: Vec<String> = [parent, child]
        .into_iter()
        .filter_map(Result::err)
        .collect();
    assert!(fails.is_empty(), "{}", fails.join("\n"));
}

/// Report 3: a drag of the thumb from the top to the end in small steps
/// moves the view down only, never more than one screen per step.
#[test]
#[ignore = "reads the owner's sessions under C:\\Users\\Yamasaki"]
fn real_child_scroll_a_thumb_drag_moves_the_view_steadily() {
    let chat = open("drag");
    let x = chat.ui.get_transcript_thumb_x();
    chat.press(x, chat.ui.get_transcript_thumb_y());
    chat.move_to(x, chat.top() - 400.0);
    chat.release(x, chat.top() - 400.0);
    pump(&chat.window, 300);
    assert!(
        chat.y().abs() < 1.0,
        "a drag up to the top ends at content-y {}",
        chat.y()
    );
    let (view, content) = (chat.view(), chat.content());
    let steps = ((content / view).ceil() as usize * 2).max(20);
    let (from, to) = (chat.ui.get_transcript_thumb_y(), chat.top() + view);
    chat.press(x, from);
    let mut ys = vec![chat.y()];
    for k in 1..=steps {
        chat.move_to(x, from + (to - from) * k as f32 / steps as f32);
        pump(&chat.window, 32);
        ys.push(chat.y());
    }
    chat.release(x, to);
    pump(&chat.window, 100);
    shot(&chat.window, "drag-end");
    let moved: Vec<f32> = ys.windows(2).map(|w| w[0] - w[1]).collect();
    let largest = |a: (usize, f32), (k, d): (usize, &f32)| if *d > a.1 { (k, *d) } else { a };
    let (worst, jump) = moved.iter().enumerate().fold((0, 0.0), largest);
    let now = chat.content();
    eprintln!("drag: {steps} steps, content {content:.0} px (now {now:.0}), view {view:.0} px, largest step {jump:.0} px at {}", worst + 1);
    if let Some(k) = moved.iter().position(|d| *d < -0.5) {
        panic!(
            "a drag down moved the view back up at step {}: content-y {:.0} -> {:.0}",
            k + 1,
            ys[k],
            ys[k + 1]
        );
    }
    assert!(
        jump <= view + 0.5,
        "a drag step moved the view {jump:.0} px, more than one screen ({view:.0} px), at step {}",
        worst + 1
    );
    let end = view - now;
    assert!(
        (chat.y() - end).abs() <= 1.0,
        "the drag to the track's end stops at content-y {:.0}, not the end {end:.0}",
        chat.y()
    );
}

/// Report 4: a fold or an unfold of a turn or a step keeps the clicked
/// row where it was (within 2 px), at the end while following, in the
/// middle and at the top.
#[test]
#[ignore = "reads the owner's sessions under C:\\Users\\Yamasaki"]
fn real_child_scroll_a_fold_keeps_the_clicked_row_in_place() {
    let chat = open("fold");
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
        chat.go_to(place);
        chat.step_round_trip(place, &mut fails);
    }
    assert!(
        fails.is_empty(),
        "a fold moved the clicked row:\n{}",
        fails.join("\n")
    );
}
