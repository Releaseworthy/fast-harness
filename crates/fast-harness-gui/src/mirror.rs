//! Keeps the Slint transcript model in step with `view::Transcript` items:
//! appends new rows and replaces only the rows whose item changed (on a
//! text delta that is the last row), never rebuilding the whole list.

use fast_harness_core::view::Item;
use slint::{Model, VecModel};

use crate::rows::msg_row;
use crate::MsgRow;

/// The row operations the mirror needs; `VecModel` in the app, a recorder
/// in the tests.
pub trait Rows {
    fn count(&self) -> usize;
    fn expanded(&self, index: usize) -> bool;
    fn set(&self, index: usize, row: MsgRow);
    fn push(&self, row: MsgRow);
    fn replace(&self, rows: Vec<MsgRow>);
}

impl Rows for VecModel<MsgRow> {
    fn count(&self) -> usize {
        self.row_count()
    }
    fn expanded(&self, index: usize) -> bool {
        self.row_data(index).is_some_and(|r| r.expanded)
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
}

impl Mirror {
    /// Replaces every row (a session was opened or a new one started).
    pub fn reset(&mut self, items: &[Item], model: &dyn Rows) {
        model.replace(items.iter().map(msg_row).collect());
        self.items = items.to_vec();
    }

    /// Applies the changes of `items` from index `from` on (items before it
    /// do not change during a turn). Returns the number of rows written.
    pub fn sync(&mut self, items: &[Item], model: &dyn Rows, from: usize) -> usize {
        if items.len() < self.items.len() || model.count() != self.items.len() {
            self.reset(items, model);
            return items.len();
        }
        let mut written = 0;
        for (index, item) in items.iter().enumerate().skip(from) {
            match self.items.get(index) {
                Some(old) if old == item => continue,
                Some(_) => {
                    let mut row = msg_row(item);
                    row.expanded = model.expanded(index);
                    model.set(index, row);
                    self.items[index] = item.clone();
                }
                None => {
                    model.push(msg_row(item));
                    self.items.push(item.clone());
                }
            }
            written += 1;
        }
        written
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
        fn expanded(&self, index: usize) -> bool {
            self.rows.borrow().get(index).is_some_and(|r| r.expanded)
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
}
