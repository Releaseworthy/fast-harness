//! Keeps the Slint transcript model in step with `view::Transcript` items:
//! appends new rows and replaces only the rows whose item or step line
//! changed (on a text delta that is the last row), never rebuilding the
//! whole list. Step lines and folds (D73) come from `step_rows`.

use std::collections::HashSet;

use fast_harness_core::view::{step_range, Approvals, Item};
use slint::{Model, VecModel};

use crate::rows::msg_row_with;
use crate::step_rows::{apply, Deco, Steps};
use crate::MsgRow;

/// The row operations the mirror needs; `VecModel` in the app, a recorder
/// in the tests.
pub trait Rows {
    fn count(&self) -> usize;
    fn get(&self, index: usize) -> Option<MsgRow>;
    fn set(&self, index: usize, row: MsgRow);
    fn push(&self, row: MsgRow);
    fn replace(&self, rows: Vec<MsgRow>);
    fn expanded(&self, index: usize) -> bool {
        self.get(index).is_some_and(|r| r.expanded)
    }
}

impl Rows for VecModel<MsgRow> {
    fn count(&self) -> usize {
        self.row_count()
    }
    fn get(&self, index: usize) -> Option<MsgRow> {
        self.row_data(index)
    }
    fn set(&self, index: usize, row: MsgRow) {
        self.set_row_data(index, row);
    }
    fn push(&self, row: MsgRow) {
        VecModel::push(self, row);
    }
    fn replace(&self, rows: Vec<MsgRow>) {
        self.set_vec(rows);
    }
}

#[derive(Default)]
pub struct Mirror {
    /// The items the model rows were made from.
    items: Vec<Item>,
    /// Each tool row's approval word (D64).
    words: Vec<Option<&'static str>>,
    /// Each row's step part (D73).
    decos: Vec<Deco>,
    /// The first rows of the steps opened by the user.
    open: HashSet<usize>,
    /// `chatDetail: "full"`.
    full: bool,
}

fn word(item: &Item, approvals: &Approvals) -> Option<&'static str> {
    match item {
        Item::Tool { id, .. } => approvals.word(id),
        _ => None,
    }
}

impl Mirror {
    pub fn new(full: bool) -> Mirror {
        Mirror {
            full,
            ..Mirror::default()
        }
    }

    /// Replaces every row (a session was opened or a new one started),
    /// with the approval words of the tool rows; every step folds again.
    pub fn reset_with(&mut self, items: &[Item], approvals: &Approvals, model: &dyn Rows) {
        self.open.clear();
        self.rebuild(items, approvals, model);
    }

    fn rebuild(&mut self, items: &[Item], approvals: &Approvals, model: &dyn Rows) {
        self.items = items.to_vec();
        self.words = items.iter().map(|i| word(i, approvals)).collect();
        self.decos = self.steps().from(0);
        let rows = (items.iter().zip(&self.decos)).map(|(item, deco)| {
            let mut row = msg_row_with(item, approvals);
            apply(&mut row, deco);
            row
        });
        model.replace(rows.collect());
    }

    fn steps(&self) -> Steps<'_> {
        Steps {
            items: &self.items,
            words: &self.words,
            full: self.full,
            open: &self.open,
        }
    }

    /// Applies the changes of `items` from index `from` on (items before it
    /// do not change during a turn). Returns the number of rows written.
    #[cfg(test)]
    pub fn sync(&mut self, items: &[Item], model: &dyn Rows, from: usize) -> usize {
        self.sync_with(items, &Approvals::default(), model, from)
    }

    /// [`Mirror::sync`] with the approval words of the tool rows (D64): a
    /// changed word rewrites only its own row. The steps from the one
    /// around `from` get their lines again; a row is written only when its
    /// item, word or line changed.
    pub fn sync_with(
        &mut self,
        items: &[Item],
        approvals: &Approvals,
        model: &dyn Rows,
        from: usize,
    ) -> usize {
        if items.len() < self.items.len() || model.count() != self.items.len() {
            self.rebuild(items, approvals, model);
            return items.len();
        }
        let start = step_range(items, from.min(self.items.len())).start;
        let words: Vec<_> = items[start..].iter().map(|i| word(i, approvals)).collect();
        let decos = Steps {
            items,
            words: &[&self.words[..start], &words[..]].concat(),
            full: self.full,
            open: &self.open,
        }
        .from(start);
        let mut written = 0;
        for (offset, (deco, w)) in decos.into_iter().zip(words).enumerate() {
            let index = start + offset;
            if self.write(index, &items[index], w, deco, approvals, model) {
                written += 1;
            }
        }
        written
    }

    /// Writes row `index` when its item, word or step part changed.
    fn write(
        &mut self,
        index: usize,
        item: &Item,
        w: Option<&'static str>,
        deco: Deco,
        approvals: &Approvals,
        model: &dyn Rows,
    ) -> bool {
        let same = self.items.get(index) == Some(item) && self.words.get(index) == Some(&w);
        if same && self.decos.get(index) == Some(&deco) {
            return false;
        }
        let mut row = match model.get(index).filter(|_| same) {
            Some(row) => row,
            None => {
                let mut row = msg_row_with(item, approvals);
                row.expanded = model.expanded(index);
                row
            }
        };
        apply(&mut row, &deco);
        if index < self.items.len() {
            model.set(index, row);
            (self.items[index], self.words[index], self.decos[index]) = (item.clone(), w, deco);
        } else {
            model.push(row);
            self.items.push(item.clone());
            self.words.push(w);
            self.decos.push(deco);
        }
        true
    }

    /// Opens or folds the step whose line is on row `index` (D73).
    pub fn toggle_step(&mut self, index: usize, model: &dyn Rows) {
        if index >= self.items.len() {
            return;
        }
        if !self.open.remove(&index) {
            self.open.insert(index);
        }
        let range = step_range(&self.items, index);
        let decos = self.steps().step(range.start, range.end);
        self.redraw(range.start, decos, model);
    }

    /// Switches between step lines and full rows (`chatDetail`) in place.
    pub fn set_full(&mut self, full: bool, model: &dyn Rows) {
        self.full = full;
        let decos = self.steps().from(0);
        self.redraw(0, decos, model);
    }

    /// Writes the rows from `start` whose step part changed.
    fn redraw(&mut self, start: usize, decos: Vec<Deco>, model: &dyn Rows) {
        for (offset, deco) in decos.into_iter().enumerate() {
            let index = start + offset;
            if self.decos.get(index) == Some(&deco) {
                continue;
            }
            if let Some(mut row) = model.get(index) {
                apply(&mut row, &deco);
                model.set(index, row);
            }
            if let Some(slot) = self.decos.get_mut(index) {
                *slot = deco;
            }
        }
    }

    #[cfg(test)]
    fn item_count(&self) -> usize {
        self.items.len()
    }
}

/// Opens or closes the thinking or tool output of row `index`.
pub fn toggle(model: &VecModel<MsgRow>, index: usize) {
    if let Some(mut row) = model.row_data(index) {
        row.expanded = !row.expanded;
        model.set_row_data(index, row);
    }
}
#[cfg(test)]
#[path = "mirror_bench.rs"]
mod bench;

#[cfg(test)]
mod tests {
    use super::*;
    use fast_harness_core::events::AgentEvent;
    use fast_harness_core::view::Transcript;
    use std::cell::RefCell;

    /// Rows that record every operation.
    #[derive(Default)]
    struct Log {
        rows: RefCell<Vec<MsgRow>>,
        ops: RefCell<Vec<String>>,
    }

    impl Rows for Log {
        fn count(&self) -> usize {
            self.rows.borrow().len()
        }
        fn get(&self, index: usize) -> Option<MsgRow> {
            self.rows.borrow().get(index).cloned()
        }
        fn set(&self, index: usize, row: MsgRow) {
            self.ops.borrow_mut().push(format!("set {index}"));
            if let Some(slot) = self.rows.borrow_mut().get_mut(index) {
                *slot = row;
            }
        }
        fn push(&self, row: MsgRow) {
            self.ops.borrow_mut().push("push".into());
            self.rows.borrow_mut().push(row);
        }
        fn replace(&self, rows: Vec<MsgRow>) {
            self.ops.borrow_mut().push("replace".into());
            *self.rows.borrow_mut() = rows;
        }
    }

    #[test]
    fn deltas_change_only_the_last_row() {
        let log = Log::default();
        let mut view = Transcript::new();
        let mut mirror = Mirror::default();
        view.push_user("hello");
        mirror.sync(&view.items, &log, 0);
        view.apply(&AgentEvent::TextDelta("Hi".into()));
        assert_eq!(mirror.sync(&view.items, &log, 1), 1);
        view.apply(&AgentEvent::TextDelta(" there".into()));
        assert_eq!(mirror.sync(&view.items, &log, 1), 1);
        assert_eq!(mirror.sync(&view.items, &log, 0), 0);
        assert_eq!(*log.ops.borrow(), ["push", "push", "set 1"]);
        assert_eq!(log.rows.borrow()[1].text, "Hi there");
    }

    #[test]
    fn expanded_rows_stay_open_and_shrinking_resets() {
        let model = VecModel::<MsgRow>::default();
        let mut view = Transcript::new();
        let mut mirror = Mirror::default();
        view.push_user("q");
        view.apply(&AgentEvent::ThinkingDelta("a".into()));
        mirror.sync(&view.items, &model, 0);
        toggle(&model, 1);
        view.apply(&AgentEvent::ThinkingDelta("b".into()));
        mirror.sync(&view.items, &model, 0);
        let row = model.row_data(1).unwrap_or_default();
        assert!(row.expanded);
        assert_eq!(row.thinking, "ab");
        assert_eq!(mirror.sync(&[], &model, 0), 0);
        assert_eq!((model.row_count(), mirror.item_count()), (0, 0));
        toggle(&model, 9);
    }

    #[test]
    fn a_tool_result_rewrites_its_own_card() {
        let log = Log::default();
        let mut view = Transcript::new();
        let mut mirror = Mirror::default();
        view.push_user("q");
        let call = |id: &str| AgentEvent::ToolCall {
            id: id.into(),
            name: "read".into(),
            arguments: "{}".into(),
        };
        view.apply(&call("a"));
        view.apply(&call("b"));
        mirror.sync(&view.items, &log, 0);
        view.apply(&AgentEvent::ToolResult {
            id: "a".into(),
            content: "done".into(),
            is_error: false,
            duration_ms: 3,
        });
        mirror.sync(&view.items, &log, 1);
        let rows = log.rows.borrow();
        let tools: Vec<bool> = rows.iter().skip(1).map(|r| r.running).collect();
        assert_eq!(tools, [false, true]);
        assert_eq!(log.ops.borrow().last().map(String::as_str), Some("set 1"));
    }

    #[test]
    fn progress_rewrites_only_its_running_row() {
        let log = Log::default();
        let mut view = Transcript::new();
        let mut mirror = Mirror::default();
        view.push_user("q");
        for id in ["a", "b"] {
            view.apply(&AgentEvent::ToolCall {
                id: id.into(),
                name: "shell".into(),
                arguments: "{}".into(),
            });
        }
        mirror.sync(&view.items, &log, 0);
        let ops = log.ops.borrow().len();
        for chunk in ["one\n", "two\n"] {
            let (id, chunk) = ("a".into(), chunk.into());
            view.apply(&AgentEvent::ToolProgress { id, chunk });
            assert_eq!(mirror.sync(&view.items, &log, 0), 1);
        }
        assert_eq!(log.ops.borrow()[ops..], ["set 1", "set 1"]);
        assert_eq!(log.rows.borrow()[1].live, "one\ntwo");
    }
}
