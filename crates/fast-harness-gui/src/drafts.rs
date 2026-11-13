//! Each session's unsent composer text (PLAN.md D65): the one composer of the
//! window belongs to the session shown, or to the session being opened while
//! its log loads (D60), so a session's draft shows as soon as it is asked
//! for and the text of the session left behind never shows as the next one's.
//! The window cannot read its cursor, so a draft comes back with the cursor
//! at its end.

use fast_harness_core::app::drafts::Draft;

use crate::app::App;
use crate::MainWindow;

/// The composer moves to session `to` (`None`: the new session): its text is
/// saved for the session it belonged to and `to`'s draft shows. Nothing
/// changes when `to` already has it.
pub fn switch(app: &mut App, ui: &MainWindow, to: Option<&str>) {
    let current = Draft::at_end(ui.get_draft().as_str());
    if let Some(next) = app.drafts.switch(current, to) {
        let cursor = i32::try_from(next.cursor).unwrap_or(i32::MAX);
        ui.invoke_show_draft(next.text.into(), cursor);
    }
}

/// An open failed or could not start: the composer goes back to the session
/// the window still shows.
pub fn back_to_shown(app: &mut App, ui: &MainWindow) {
    let id = app.runs.id.clone();
    switch(app, ui, id.as_deref());
}
