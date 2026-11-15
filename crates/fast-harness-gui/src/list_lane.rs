//! The virtual list's lane (`virtual_list`): rows laid out off screen, a
//! batch per frame, until every row's height is known.

use std::time::Duration;

use slint::{ComponentHandle, Model, TimerMode};

use super::{with, Host, VirtualList};
use crate::MainWindow;

/// The lane's rows per batch, and the frames a batch may take to report.
const BATCH: usize = 12;
const PATIENCE: u32 = 10;
const TRIES: u8 = 4; // batches a late row is asked before it keeps its estimate
const TICK: Duration = Duration::from_millis(16);

impl<H: Host> VirtualList<H> {
    /// Starts or stops the lane that measures the rows not measured yet.
    pub(super) fn background(&mut self, ui: &MainWindow) {
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
