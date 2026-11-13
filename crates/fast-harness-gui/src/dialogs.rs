//! Settings and About open as modal dialogs over the window (PLAN.md D76),
//! not as pages: `open-page("settings")` from the title bar or a keys.json
//! action sets `DialogUi.open` and leaves the shown page as it is.

use slint::ComponentHandle;

use crate::{DialogUi, MainWindow};

/// Opens `page` as a dialog when it is one; false for a real page.
pub fn open(ui: &MainWindow, page: &str) -> bool {
    let dialog = matches!(page, "settings" | "about");
    if dialog {
        ui.global::<DialogUi>().set_open(page.into());
    }
    dialog
}

/// Closes the open dialog (a page opens from it, as a panel does).
pub fn close(ui: &MainWindow) {
    ui.global::<DialogUi>().set_open("".into());
}