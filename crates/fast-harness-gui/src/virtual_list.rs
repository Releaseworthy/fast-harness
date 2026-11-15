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
use slint::{ComponentHandle, Model, ModelRc, Timer, VecModel};

use crate::glide::Glide;
use crate::list_end::{self, Tail};
pub use crate::list_host::Host;
use crate::MainWindow;

#[path = "list_lane.rs"]
mod lane;

/// A folded row whose top was above the view comes to this far below its top.
pub const MARGIN: f32 = 8.0;

/// A row's height and the row it was found for.
struct Known<R> {
    row: R,
    height: f32,
    exact: bool,
    guessed: bool, // the height is an estimate (never laid out)
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
    glide: Glide,
    end_key: Option<i64>, // the last measured row a following view showed
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
        glide: Glide::default(),
        end_key: None,
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
        if H::LOG && crate::follow_log::on() {
            let guessed = (0..self.rows.len()).filter(|i| self.tail(*i) == Tail::Guessed);
            let guessed: Vec<usize> = guessed.collect();
            let state = crate::follow_log::state(ui);
            crate::follow_log::line(|| format!("sync guessed={guessed:?} {state}"));
        }
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
        if H::LOG && crate::follow_log::on() {
            let was = known.map(|k| (k.height, k.exact, k.guessed));
            crate::follow_log::line(|| format!("measured row={i} h={height:.1} was={was:?}"));
        }
        let known = Known {
            row: data,
            height,
            exact: true,
            guessed: false,
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
        let (height, guessed) = match self.known.get(&key) {
            Some(known) if known.row == *row => return known.height,
            Some(known) => (known.height, known.guessed),
            None => (H::estimate(row, self.style.0), true),
        };
        let known = Known {
            row: row.clone(),
            height,
            exact: false,
            guessed,
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
        self.tops = list_end::prefix(&heights);
        let mut offset = self.offset_for(hold, view);
        if matches!(hold, Hold::End) {
            offset = self.glide(ui, offset);
        } else {
            self.glide.stop();
            self.end_key = None;
        }
        self.show(ui, offset, view, hold);
    }

    /// Following, the view at the end `target`: a new last row glides into
    /// view (`glide`), a growing one keeps the view at the end.
    fn glide(&mut self, ui: &MainWindow, target: f32) -> f32 {
        let tails: Vec<Tail> = (0..self.rows.len()).map(|i| self.tail(i)).collect();
        let last = list_end::last_measured(&tails).map(|i| self.keys[i]);
        let fresh = self.end_key.is_some() && last != self.end_key;
        self.end_key = last;
        let (me, weak) = (self.me.clone(), ui.as_weak());
        self.glide.toward(H::offset(ui), target, fresh, move || {
            if let Some(list) = me.upgrade() {
                with(&list, &weak, VirtualList::sync);
            }
        })
    }

    fn max(&self, view: f32) -> f32 {
        (self.tops.last().copied().unwrap_or(0.0) - view).max(0.0)
    }

    /// The scroll position that keeps `hold` in place.
    fn offset_for(&self, hold: &Hold<H::Row>, view: f32) -> f32 {
        let max = self.max(view);
        let want = match hold {
            Hold::End => {
                let tails: Vec<Tail> = (0..self.rows.len()).map(|i| self.tail(i)).collect();
                list_end::settled_end(&self.tops, &tails) - view
            }
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

    /// Row `i` as the end of a following view sees it.
    fn tail(&self, i: usize) -> Tail {
        let row = &self.rows[i];
        if H::spacer(row) || !H::shows(row) {
            return Tail::Skip;
        }
        match self.known.get(&self.keys[i]) {
            Some(known) if !known.guessed => Tail::Measured,
            _ => Tail::Guessed,
        }
    }

    /// Hands the tops, the window and the scroll position to the view.
    fn show(&mut self, ui: &MainWindow, offset: f32, view: f32, hold: &Hold<H::Row>) {
        if self.shown.iter().ne(self.tops.iter().copied()) {
            self.shown.set_vec(self.tops.clone());
        }
        let span = list_end::span(&self.tops, self.rows.len(), offset, view);
        list_end::set_window(&self.window, self.span, span);
        self.span = span;
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
}
