//! What a virtual list (`virtual_list`) needs from the list it shows: its
//! rows, their estimates and folds, and the window's properties.

use slint::ModelRc;

use crate::MainWindow;

/// One list's rows and the window's properties that drive its view.
pub trait Host: 'static {
    type Row: Clone + PartialEq + 'static;
    /// Its syncs go to the follow log (`follow_log`) when that is on.
    const LOG: bool = false;
    /// The rows, and row `i`'s own index (rising in the list's order when
    /// the rows keep their place across changes; the list index if none).
    fn rows(ui: &MainWindow) -> ModelRc<Self::Row>;
    fn item(row: &Self::Row, i: usize) -> i32;
    /// A row that draws nothing (0 px), and the popup's spacer row.
    fn shows(_row: &Self::Row) -> bool {
        true
    }
    fn spacer(_row: &Self::Row) -> bool {
        false
    }
    /// A row's height before it is measured, at the list's width.
    fn estimate(row: &Self::Row, width: f32) -> f32;
    fn offset(ui: &MainWindow) -> f32;
    fn set_offset(ui: &MainWindow, offset: f32);
    /// The view's height, the spacer's height, and what measured heights
    /// depend on (the list's width and font size).
    fn view(ui: &MainWindow) -> f32;
    fn pad(_ui: &MainWindow) -> f32 {
        0.0
    }
    fn style(ui: &MainWindow) -> (f32, f32);
    fn follow(_ui: &MainWindow) -> bool {
        false
    }
    fn set_follow(_ui: &MainWindow, _follow: bool) {}
    /// The row at the view's top (its `item`) changed.
    fn top_row(_ui: &MainWindow, _item: i32) {}
    /// The row that took a toggled row's place (the step's or turn's line).
    fn line(_rows: &[Self::Row], _head: i32, _turn: bool) -> Option<usize> {
        None
    }
    /// A toggle folded efore into fter (None: the row left the list).
    fn collapsed(_before: &Self::Row, after: Option<&Self::Row>) -> bool {
        after.is_none()
    }
    /// Hands the tops, the drawn rows and the lane's rows to the view.
    fn set_models(ui: &MainWindow, tops: ModelRc<f32>, slots: ModelRc<i32>, probes: ModelRc<i32>);
}
