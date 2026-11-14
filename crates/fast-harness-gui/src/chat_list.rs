//! The chat list (2026-11-14): the transcript draws through its own
//! virtual list, not Slint's ListView. That one's content height was the
//! average height of the rows it had laid out times the row count
//! (i-slint-core 1.18.1 `update_visible_instances`), and the chat's rows
//! run from 24 px to about 2900 px: the scroll bar's thumb changed size as
//! the view moved, a drag of the thumb jumped and a fold moved the view.
//! Here each row's height is measured once, at the list's width, by a
//! hidden copy of the row (`probe` in ui/transcript.slint) and kept; a row
//! not measured yet counts with an estimate from its kind and text (never
//! from other rows) until a timer measures it, the rows nearest the view
//! first, a few milliseconds per frame. The rows' tops are the prefix sums
//! of these heights (`tops`, the last one the content height), and the
//! view lays out only the rows in and around it (`window`).
//!
//! The view keeps its place through every change: following, it holds the
//! end; after a toggle (`pin`), the clicked row, or the line that took its
//! place, stays at its height on screen; else the row at the view's top
//! stays put while rows above it change height.

use std::cell::RefCell;
use std::collections::{HashMap, HashSet};
use std::rc::{Rc, Weak};
use std::time::{Duration, Instant};

use slint::{ComponentHandle, Model, ModelRc, Timer, TimerMode, VecModel};

use crate::{MainWindow, MsgRow, Theme, TurnsUi};

/// Rows laid out above and below the view, in view heights.
const OVERSCAN: f32 = 0.5;
/// The background measuring's time per frame, and its frame.
const BUDGET: Duration = Duration::from_millis(6);
const TICK: Duration = Duration::from_millis(16);

/// A row's height and the row it was found for.
struct Known {
    row: MsgRow,
    height: f32,
    exact: bool,
}

/// The clicked row before a toggle: its key, the toggled step's or turn's
/// head, and its top on screen.
struct Anchor {
    key: i64,
    head: i32,
    turn: bool,
    screen: f32,
}

/// What a sync keeps in place.
enum Hold {
    End,
    Row { key: i64, screen: f32 },
    Toggle(Anchor),
}

#[derive(Default)]
pub struct ChatList {
    known: HashMap<i64, Known>,
    rows: Vec<MsgRow>,
    keys: Vec<i64>,
    tops: Vec<f32>,
    shown: Rc<VecModel<f32>>,
    window: Rc<VecModel<i32>>,
    span: (usize, usize),
    style: (f32, f32),
    anchor: Option<Anchor>,
    timer: Timer,
    me: Weak<RefCell<ChatList>>,
}

/// A main window with its chat list attached.
pub fn new_window() -> Result<MainWindow, slint::PlatformError> {
    let ui = MainWindow::new()?;
    attach(&ui);
    Ok(ui)
}

/// Gives `ui`'s transcript its list: the models and the callbacks.
pub fn attach(ui: &MainWindow) {
    let list = Rc::new(RefCell::new(ChatList::default()));
    if let Ok(mut inner) = list.try_borrow_mut() {
        inner.me = Rc::downgrade(&list);
        ui.set_chat_tops(ModelRc::from(inner.shown.clone()));
        ui.set_chat_slots(ModelRc::from(inner.window.clone()));
    }
    let (weak, shared) = (ui.as_weak(), list.clone());
    ui.on_chat_sync(move || with(&shared, &weak, ChatList::sync));
    let (weak, shared) = (ui.as_weak(), list.clone());
    ui.on_chat_anchor(move |row, head, turn| {
        with(&shared, &weak, |list, ui| list.pin(ui, row, head, turn));
    });
    let (weak, shared) = (ui.as_weak(), list.clone());
    ui.on_chat_go(move |item| with(&shared, &weak, |list, ui| list.go(ui, item)));
    with(&list, &ui.as_weak(), ChatList::sync);
}

/// Runs `f` on the list, unless it is busy (a call from inside a sync).
fn with(
    list: &Rc<RefCell<ChatList>>,
    ui: &slint::Weak<MainWindow>,
    f: impl FnOnce(&mut ChatList, &MainWindow),
) {
    let (Some(ui), Ok(mut list)) = (ui.upgrade(), list.try_borrow_mut()) else {
        return;
    };
    f(&mut list, &ui);
}

/// The tops of rows of `heights`, and the end.
fn prefix(heights: &[f32]) -> Vec<f32> {
    let mut tops = Vec::with_capacity(heights.len() + 1);
    let mut y = 0.0;
    tops.push(y);
    for height in heights {
        y += height;
        tops.push(y);
    }
    tops
}

/// A row's height before it is measured: from its kind and its text's
/// length at the list's width, never from other rows.
fn estimate(row: &MsgRow, width: f32) -> f32 {
    let column = (width.min(860.0) - 80.0).max(200.0);
    let per_line = (column / 7.5).max(20.0);
    let lines = |text: &str| -> f32 {
        let count = |line: &str| (line.chars().count() as f32 / per_line).ceil().max(1.0);
        text.lines().map(count).sum()
    };
    match row.kind.as_str() {
        "user" => 64.0 + 20.0 * lines(&row.text),
        "assistant" if row.text.is_empty() => {
            if row.thinking.is_empty() || row.folded {
                10.0
            } else {
                34.0
            }
        }
        "assistant" => 36.0 + 20.0 * lines(&row.text),
        "tool" if row.folded => 34.0,
        "tool" => 52.0,
        "note" => 12.0 + 16.0 * lines(&row.text),
        _ => 28.0,
    }
}

impl ChatList {
    /// Brings the heights, tops, window and scroll position up to date.
    pub fn sync(&mut self, ui: &MainWindow) {
        let hold = match self.anchor.take() {
            Some(anchor) => Hold::Toggle(anchor),
            None if ui.get_transcript_follow() => Hold::End,
            None => self.top_row(ui.get_chat_offset()),
        };
        let style = (ui.get_chat_width(), ui.global::<Theme>().get_font_m());
        if style != self.style {
            self.known.clear();
            self.style = style;
        }
        self.read(ui);
        self.place(ui, &hold);
    }

    /// A row's toggle is about to run: list row `row` (or what takes its
    /// place) keeps its top on screen through the next sync.
    pub fn pin(&mut self, ui: &MainWindow, row: i32, head: i32, turn: bool) {
        let Ok(i) = usize::try_from(row) else {
            return;
        };
        let (Some(&key), Some(&top)) = (self.keys.get(i), self.tops.get(i)) else {
            return;
        };
        let screen = top - ui.get_chat_offset();
        self.anchor = Some(Anchor {
            key,
            head,
            turn,
            screen,
        });
    }

    /// A navigator tick: row `item` (or the next one shown) at the top.
    pub fn go(&mut self, ui: &MainWindow, item: i32) {
        let i = self.rows.partition_point(|r| r.item < item);
        let top = self.tops.get(i).copied().unwrap_or(0.0);
        ui.set_chat_offset(top.clamp(0.0, self.max(ui.get_chat_list_height())));
        self.sync(ui);
    }

    /// The row at the view's top and its top on screen.
    fn top_row(&self, offset: f32) -> Hold {
        match self.index_at(offset) {
            Some(i) => Hold::Row {
                key: self.keys[i],
                screen: self.tops[i] - offset,
            },
            None => Hold::End,
        }
    }

    /// The row under `y` (from the list's top).
    fn index_at(&self, y: f32) -> Option<usize> {
        let n = self.keys.len();
        if n == 0 || self.tops.len() != n + 1 {
            return None;
        }
        Some(self.tops[1..].partition_point(|t| *t <= y).min(n - 1))
    }

    /// Reads the list's rows and their keys: the transcript's own index
    /// (`item`) when the rows come in its order, else the list's index.
    fn read(&mut self, ui: &MainWindow) {
        let model = ui.get_messages();
        self.rows = (0..model.row_count())
            .filter_map(|i| model.row_data(i))
            .collect();
        let by_item = self.rows.windows(2).all(|w| w[0].item < w[1].item);
        let key = |(i, row): (usize, &MsgRow)| {
            if by_item {
                i64::from(row.item)
            } else {
                i as i64
            }
        };
        self.keys = self.rows.iter().enumerate().map(key).collect();
        if self.known.len() > 2 * self.keys.len() + 64 {
            let keep: HashSet<i64> = self.keys.iter().copied().collect();
            self.known.retain(|k, _| keep.contains(k));
        }
    }

    /// Row `i`'s height: measured, else an estimate (kept until measured).
    fn height(&mut self, i: usize, pad: f32) -> f32 {
        let row = &self.rows[i];
        if row.kind == "spacer" {
            return pad;
        }
        if let Some(known) = self.known.get(&self.keys[i]).filter(|k| k.row == *row) {
            return known.height;
        }
        let height = estimate(row, self.style.0);
        let known = Known {
            row: row.clone(),
            height,
            exact: false,
        };
        self.known.insert(self.keys[i], known);
        height
    }

    /// Row `i`'s height is measured for the row it holds now.
    fn exact(&self, i: usize) -> bool {
        let row = &self.rows[i];
        let known = self.known.get(&self.keys[i]);
        row.kind == "spacer" || known.is_some_and(|k| k.exact && k.row == *row)
    }

    /// The tops from the heights, the scroll position that keeps `hold`
    /// and the window around it, its rows measured; then the view's turn.
    fn place(&mut self, ui: &MainWindow, hold: &Hold) {
        let (view, pad) = (ui.get_chat_list_height(), ui.get_chat_pad());
        let mut offset = 0.0;
        for _ in 0..4 {
            let heights: Vec<f32> = (0..self.rows.len()).map(|i| self.height(i, pad)).collect();
            self.tops = prefix(&heights);
            offset = self.offset_for(hold, view);
            let (a, b) = self.span_at(offset, view);
            if !self.measure(ui, a..b, None) {
                break;
            }
        }
        self.show(ui, offset, view, hold);
    }

    fn max(&self, view: f32) -> f32 {
        (self.tops.last().copied().unwrap_or(0.0) - view).max(0.0)
    }

    /// The scroll position that keeps `hold` in place.
    fn offset_for(&self, hold: &Hold, view: f32) -> f32 {
        let max = self.max(view);
        let want = match hold {
            Hold::End => max,
            Hold::Row { key, screen } => {
                let i = self.keys.partition_point(|k| k < key);
                self.tops.get(i).map_or(max, |top| top - screen)
            }
            Hold::Toggle(anchor) => self
                .toggled(anchor)
                .map_or(max, |i| self.tops[i] - anchor.screen),
        };
        want.clamp(0.0, max)
    }

    /// The row that keeps a toggled row's place: the row itself while it
    /// shows, else the line that took its place (the step's head, the
    /// folded turn's line), else the last row before it.
    fn toggled(&self, anchor: &Anchor) -> Option<usize> {
        if let Ok(i) = self.keys.binary_search(&anchor.key) {
            return Some(i);
        }
        let line = if anchor.turn {
            let line = |r: &MsgRow| r.turn_head == anchor.head && !r.turn_line.is_empty();
            self.rows.iter().position(line)
        } else {
            self.rows.iter().position(|r| r.item == anchor.head)
        };
        line.or_else(|| {
            self.keys
                .partition_point(|k| *k < anchor.key)
                .checked_sub(1)
        })
    }

    /// The rows to lay out for the view at `offset`: in it and around it.
    fn span_at(&self, offset: f32, view: f32) -> (usize, usize) {
        let n = self.rows.len();
        if n == 0 || self.tops.len() != n + 1 {
            return (0, 0);
        }
        let (low, high) = (offset - OVERSCAN * view, offset + view + OVERSCAN * view);
        let a = self.tops[1..].partition_point(|t| *t <= low);
        let b = self.tops[..n].partition_point(|t| *t < high);
        (a.min(b), b)
    }

    /// Measures the rows of `rows` not measured yet (until `until`, when
    /// given) through the hidden row; true when it measured any.
    fn measure(
        &mut self,
        ui: &MainWindow,
        rows: impl Iterator<Item = usize>,
        until: Option<Instant>,
    ) -> bool {
        let mut any = false;
        for i in rows {
            if until.is_some_and(|t| Instant::now() >= t) {
                break;
            }
            if self.exact(i) {
                continue;
            }
            ui.set_chat_probe_row(self.rows[i].clone());
            let height = ui.get_chat_probe_height();
            let known = Known {
                row: self.rows[i].clone(),
                height,
                exact: true,
            };
            self.known.insert(self.keys[i], known);
            any = true;
        }
        if any {
            // An empty row: a running tool's spinner never ticks unseen.
            ui.set_chat_probe_row(MsgRow::default());
        }
        any
    }

    /// Hands the tops, the window and the scroll position to the view.
    fn show(&mut self, ui: &MainWindow, offset: f32, view: f32, hold: &Hold) {
        if self.shown.iter().ne(self.tops.iter().copied()) {
            self.shown.set_vec(self.tops.clone());
        }
        let span = self.span_at(offset, view);
        self.set_window(span);
        if (ui.get_chat_offset() - offset).abs() > 0.01 {
            ui.set_chat_offset(offset);
        }
        if matches!(hold, Hold::Toggle(_)) {
            ui.set_transcript_follow(offset >= self.max(view) - 1.0);
        }
        if let Some(i) = self.index_at(offset + 1.0) {
            ui.global::<TurnsUi>().set_top_row(self.rows[i].item);
        }
        self.background(ui);
    }

    /// The window's rows: its ends move one row at a time, so the rows
    /// that stay keep their laid out views.
    fn set_window(&mut self, (a, b): (usize, usize)) {
        let (a0, b0) = self.span;
        let model = &self.window;
        if model.row_count() != b0 - a0 || b <= a0 || a >= b0 {
            model.set_vec((a..b).map(|i| i as i32).collect::<Vec<_>>());
        } else {
            for _ in b..b0 {
                model.remove(model.row_count() - 1);
            }
            (b0..b).for_each(|i| model.push(i as i32));
            for _ in a0..a {
                model.remove(0);
            }
            (a..a0).rev().for_each(|i| model.insert(0, i as i32));
        }
        self.span = (a, b);
    }

    /// Starts or stops the timer that measures the rows not measured yet.
    fn background(&mut self, ui: &MainWindow) {
        if (0..self.rows.len()).all(|i| self.exact(i)) {
            self.timer.stop();
            return;
        }
        if self.timer.running() {
            return;
        }
        let (me, weak) = (self.me.clone(), ui.as_weak());
        self.timer.start(TimerMode::Repeated, TICK, move || {
            if let Some(list) = me.upgrade() {
                with(&list, &weak, ChatList::tick);
            }
        });
    }

    /// One frame of background measuring, the rows nearest the view first,
    /// then a sync that keeps the view in place.
    fn tick(&mut self, ui: &MainWindow) {
        let until = Instant::now() + BUDGET;
        let ((a, b), n) = (self.span, self.rows.len());
        let near = (0..n)
            .flat_map(move |d| [b + d, a.wrapping_sub(d + 1)])
            .filter(move |i| *i < n);
        if self.measure(ui, near, Some(until)) {
            self.sync(ui);
        } else {
            self.background(ui);
        }
    }
}
