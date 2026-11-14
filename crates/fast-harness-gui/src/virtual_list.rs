//! A virtual list with measured row heights (2026-11-14), shared by the
//! chat (`chat_list`) and the Trace tab's timeline (`trace_list`). Slint
//! 1.18.1's ListView made its content height the average height of the
//! rows it had laid out times the row count (i-slint-core
//! `update_visible_instances`), so its thumb changed size as the view
//! moved, a drag of the thumb jumped and a fold moved the view.
//!
//! Each row's height is kept here once a laid out copy of the row reports
//! it (`measured`): the drawn rows, and a lane of rows laid out off screen
//! a batch per frame, nearest the view first, until every row is known. A
//! row not measured yet counts with its host's estimate, never from other
//! rows. Slint builds a row's `if` parts only when its renderer visits the
//! row (i-slint-core 1.18.1 `item_rendering.rs`), and never under an empty
//! clip, so a `visible: false` copy measured only its paddings: the lane is
//! visible, off screen and unclipped. The rows' tops are the prefix sums of
//! the heights (the last one the content height), and the view lays out
//! only the rows in and around it.
//!
//! The view keeps its place through every change: following, it holds the
//! end; after a toggle (`pin`), the clicked row, or the line that took its
//! place, stays at its height on screen; else the row at the view's top
//! stays put while rows above it change height.

use std::cell::RefCell;
use std::collections::{HashMap, HashSet};
use std::rc::{Rc, Weak};
use std::time::Duration;

use slint::{ComponentHandle, Model, ModelRc, Timer, TimerMode, VecModel};

pub use crate::list_host::Host;
use crate::MainWindow;

/// Rows laid out above and below the view, in view heights.
const OVERSCAN: f32 = 0.5;
/// The lane's rows per batch, and the frames a batch may take to report.
const BATCH: usize = 12;
const PATIENCE: u32 = 10;
const TRIES: u8 = 4; // batches a late row is asked before it keeps its estimate
const TICK: Duration = Duration::from_millis(16);
/// A folded row whose top was above the view comes to this far below its top.
pub const MARGIN: f32 = 8.0;

/// A row's height and the row it was found for.
struct Known<R> {
    row: R,
    height: f32,
    exact: bool,
}

/// The clicked row before a toggle: its key, the toggled step's or turn's
/// head, its top on screen and the row before the toggle.
struct Anchor<R> {
    key: i64,
    head: i32,
    turn: bool,
    screen: f32,
    before: Option<R>,
}

/// What a sync keeps in place.
enum Hold<R> {
    End,
    Row { key: i64, screen: f32 },
    // No rows placed yet (a view just built): its scroll position as it is.
    Offset(f32),
    Toggle(Anchor<R>),
}

pub struct VirtualList<H: Host> {
    known: HashMap<i64, Known<H::Row>>,
    rows: Vec<H::Row>,
    keys: Vec<i64>,
    by_item: bool,
    tops: Vec<f32>,
    shown: Rc<VecModel<f32>>,
    window: Rc<VecModel<i32>>,
    probes: Rc<VecModel<i32>>,
    waited: u32,
    tries: HashMap<i64, u8>,
    span: (usize, usize),
    style: (f32, f32),
    anchor: Option<Anchor<H::Row>>,
    timer: Timer,
    me: Weak<RefCell<VirtualList<H>>>,
}

/// A list attached to a window: what its callbacks call.
pub struct Handle<H: Host> {
    list: Rc<RefCell<VirtualList<H>>>,
    ui: slint::Weak<MainWindow>,
}

impl<H: Host> Clone for Handle<H> {
    fn clone(&self) -> Self {
        Handle {
            list: self.list.clone(),
            ui: self.ui.clone(),
        }
    }
}

/// A new list for `ui`, its models handed to the view.
pub fn attach<H: Host>(ui: &MainWindow) -> Handle<H> {
    let list = Rc::new(RefCell::new(VirtualList::<H> {
        known: HashMap::new(),
        rows: Vec::new(),
        keys: Vec::new(),
        by_item: false,
        tops: Vec::new(),
        shown: Rc::default(),
        window: Rc::default(),
        probes: Rc::default(),
        waited: 0,
        tries: HashMap::new(),
        span: (0, 0),
        style: (0.0, 0.0),
        anchor: None,
        timer: Timer::default(),
        me: Weak::new(),
    }));
    if let Ok(mut inner) = list.try_borrow_mut() {
        inner.me = Rc::downgrade(&list);
        let tops = ModelRc::from(inner.shown.clone());
        H::set_models(
            ui,
            tops,
            ModelRc::from(inner.window.clone()),
            ModelRc::from(inner.probes.clone()),
        );
    }
    Handle {
        list,
        ui: ui.as_weak(),
    }
}

impl<H: Host> Handle<H> {
    /// Runs `f` on the list, unless it is busy (a call from inside a sync).
    fn run(&self, f: impl FnOnce(&mut VirtualList<H>, &MainWindow)) {
        with(&self.list, &self.ui, f);
    }
    pub fn sync(&self) {
        self.run(VirtualList::sync);
    }
    pub fn pin(&self, row: i32, head: i32, turn: bool) {
        self.run(|list, ui| list.pin(ui, row, head, turn));
    }
    pub fn measured(&self, row: i32, item: i32, height: f32) {
        self.run(|list, ui| list.measured(ui, row, item, height));
    }
    pub fn go(&self, item: i32) {
        self.run(|list, ui| list.go(ui, item));
    }
}

fn with<H: Host>(
    list: &Rc<RefCell<VirtualList<H>>>,
    ui: &slint::Weak<MainWindow>,
    f: impl FnOnce(&mut VirtualList<H>, &MainWindow),
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

impl<H: Host> VirtualList<H> {
    /// Brings the heights, tops, window and scroll position up to date.
    pub fn sync(&mut self, ui: &MainWindow) {
        let hold = match self.anchor.take() {
            Some(anchor) => Hold::Toggle(anchor),
            None if H::follow(ui) => Hold::End,
            None => self.top_row(H::offset(ui)),
        };
        let style = H::style(ui);
        if style != self.style {
            self.known.clear();
            self.tries.clear();
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
        let screen = top - H::offset(ui);
        let before = self.rows.get(i).cloned();
        self.anchor = Some(Anchor {
            key,
            head,
            turn,
            screen,
            before,
        });
    }

    /// A laid out copy of list row `row` (row `item`) reports its height:
    /// kept, and the list placed again when it is news.
    pub fn measured(&mut self, ui: &MainWindow, row: i32, item: i32, height: f32) {
        let Ok(i) = usize::try_from(row) else {
            return;
        };
        let Some(data) = H::rows(ui).row_data(i).filter(|r| H::item(r, i) == item) else {
            return;
        };
        let key = if self.by_item {
            i64::from(item)
        } else {
            i as i64
        };
        let known = self.known.get(&key);
        let same =
            known.is_some_and(|k| k.exact && k.row == data && (k.height - height).abs() < 0.01);
        if same || H::spacer(&data) {
            return;
        }
        let known = Known {
            row: data,
            height,
            exact: true,
        };
        self.known.insert(key, known);
        self.sync(ui);
    }

    /// A navigator tick: row `item` (or the next one shown) at the top.
    pub fn go(&mut self, ui: &MainWindow, item: i32) {
        let i = (self.rows.iter().enumerate()).position(|(i, r)| H::item(r, i) >= item);
        let top = (i.and_then(|i| self.tops.get(i)).copied()).unwrap_or(0.0);
        H::set_offset(ui, top.clamp(0.0, self.max(H::view(ui))));
        self.sync(ui);
    }

    /// The row at the view's top and its top on screen.
    fn top_row(&self, offset: f32) -> Hold<H::Row> {
        match self.index_at(offset) {
            Some(i) => Hold::Row {
                key: self.keys[i],
                screen: self.tops[i] - offset,
            },
            // Before this fix the Trace tab opened at its end, then kept a
            // middle row at the top as heights came in (2026-11-14).
            None => Hold::Offset(offset),
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

    /// Reads the list's rows and their keys: the rows' own index (`item`)
    /// when the rows come in its order, else the list's index.
    fn read(&mut self, ui: &MainWindow) {
        let model = H::rows(ui);
        self.rows = (0..model.row_count())
            .filter_map(|i| model.row_data(i))
            .collect();
        let items: Vec<i32> = (self.rows.iter().enumerate())
            .map(|(i, r)| H::item(r, i))
            .collect();
        let by_item = items.windows(2).all(|w| w[0] < w[1]);
        self.keys = (items.iter().enumerate())
            .map(|(i, item)| if by_item { i64::from(*item) } else { i as i64 })
            .collect();
        self.by_item = by_item;
        if self.known.len() > 2 * self.keys.len() + 64 {
            let keep: HashSet<i64> = self.keys.iter().copied().collect();
            self.known.retain(|k, _| keep.contains(k));
        }
    }

    /// Row `i`'s height: measured; for a row that changed since, its last
    /// height until it reports again; else an estimate.
    fn height(&mut self, i: usize, pad: f32) -> f32 {
        let (row, key) = (&self.rows[i], self.keys[i]);
        if H::spacer(row) {
            return pad;
        }
        if !H::shows(row) {
            return 0.0;
        }
        let height = match self.known.get(&key) {
            Some(known) if known.row == *row => return known.height,
            Some(known) => known.height,
            None => H::estimate(row, self.style.0),
        };
        let known = Known {
            row: row.clone(),
            height,
            exact: false,
        };
        self.known.insert(key, known);
        height
    }

    /// Row `i`'s height is measured for the row it holds now.
    fn exact(&self, i: usize) -> bool {
        let row = &self.rows[i];
        let known = self.known.get(&self.keys[i]);
        H::spacer(row) || !H::shows(row) || known.is_some_and(|k| k.exact && k.row == *row)
    }

    /// The tops from the heights, then the scroll position that keeps
    /// `hold` and the window around it, handed to the view.
    fn place(&mut self, ui: &MainWindow, hold: &Hold<H::Row>) {
        let (view, pad) = (H::view(ui), H::pad(ui));
        let heights: Vec<f32> = (0..self.rows.len()).map(|i| self.height(i, pad)).collect();
        self.tops = prefix(&heights);
        let offset = self.offset_for(hold, view);
        self.show(ui, offset, view, hold);
    }

    fn max(&self, view: f32) -> f32 {
        (self.tops.last().copied().unwrap_or(0.0) - view).max(0.0)
    }

    /// The scroll position that keeps `hold` in place.
    fn offset_for(&self, hold: &Hold<H::Row>, view: f32) -> f32 {
        let max = self.max(view);
        let want = match hold {
            Hold::End => max,
            Hold::Offset(offset) => *offset,
            Hold::Row { key, screen } => {
                let i = self.keys.partition_point(|k| k < key);
                self.tops.get(i).map_or(max, |top| top - screen)
            }
            Hold::Toggle(anchor) => {
                let screen = self.landing(anchor);
                self.toggled(anchor).map_or(max, |i| self.tops[i] - screen)
            }
        };
        want.clamp(0.0, max)
    }

    /// Where a toggled row's top lands on screen: where it was, unless a
    /// fold left it above the view (the owner, 2026-11-14: a fold clicked
    /// low in a long open block must show the folded row), then at the
    /// view's top.
    fn landing(&self, anchor: &Anchor<H::Row>) -> f32 {
        let after = (self.keys.binary_search(&anchor.key).ok()).map(|i| &self.rows[i]);
        let folded = (anchor.before.as_ref()).is_some_and(|b| H::collapsed(b, after));
        if folded && anchor.screen < 0.0 {
            MARGIN
        } else {
            anchor.screen
        }
    }

    /// The row that keeps a toggled row's place: the row itself while it
    /// shows, else the line that took its place, else the last row before.
    fn toggled(&self, anchor: &Anchor<H::Row>) -> Option<usize> {
        if let Ok(i) = self.keys.binary_search(&anchor.key) {
            return Some(i);
        }
        H::line(&self.rows, anchor.head, anchor.turn).or_else(|| {
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

    /// Hands the tops, the window and the scroll position to the view.
    fn show(&mut self, ui: &MainWindow, offset: f32, view: f32, hold: &Hold<H::Row>) {
        if self.shown.iter().ne(self.tops.iter().copied()) {
            self.shown.set_vec(self.tops.clone());
        }
        let span = self.span_at(offset, view);
        self.set_window(span);
        if (H::offset(ui) - offset).abs() > 0.01 {
            H::set_offset(ui, offset);
        }
        if matches!(hold, Hold::Toggle(_)) {
            H::set_follow(ui, offset >= self.max(view) - 1.0);
        }
        if let Some(i) = self.index_at(offset + 1.0) {
            H::top_row(ui, H::item(&self.rows[i], i));
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

    /// Starts or stops the lane that measures the rows not measured yet.
    fn background(&mut self, ui: &MainWindow) {
        if (0..self.rows.len()).all(|i| self.exact(i)) {
            self.timer.stop();
            if self.probes.row_count() > 0 {
                self.probes.set_vec(Vec::new());
            }
            return;
        }
        if self.timer.running() {
            return;
        }
        let (me, weak) = (self.me.clone(), ui.as_weak());
        self.timer.start(TimerMode::Repeated, TICK, move || {
            if let Some(list) = me.upgrade() {
                with(&list, &weak, VirtualList::tick);
            }
        });
    }

    /// One frame of the lane: while its batch has not reported, wait (a
    /// row that never reports keeps its estimate after `PATIENCE` frames);
    /// then the next batch, the window's rows and the nearest first.
    fn tick(&mut self, ui: &MainWindow) {
        let n = self.rows.len();
        let late: Vec<usize> = (self.probes.iter())
            .filter_map(|s| usize::try_from(s).ok())
            .filter(|i| *i < n && !self.exact(*i))
            .collect();
        if !late.is_empty() && self.waited < PATIENCE {
            self.waited += 1;
            return;
        }
        // A late row is asked again: a frozen estimate moved the thumb when
        // the row was drawn (log 6143, 2026-11-14).
        for i in late {
            let key = self.keys[i];
            let tries = self.tries.entry(key).or_insert(0);
            *tries += 1;
            if *tries >= TRIES {
                if let Some(known) = self.known.get_mut(&key) {
                    known.exact = true;
                }
            }
        }
        self.waited = 0;
        let (a, b) = self.span;
        let around = (0..n).flat_map(move |d| [b + d, a.wrapping_sub(d + 1)]);
        let order = (a..b).chain(around).filter(move |i| *i < n);
        let batch: Vec<i32> = order
            .filter(|i| !self.exact(*i))
            .take(BATCH)
            .map(|i| i as i32)
            .collect();
        if batch.is_empty() {
            self.background(ui);
            return;
        }
        self.probes.set_vec(batch);
    }
}
