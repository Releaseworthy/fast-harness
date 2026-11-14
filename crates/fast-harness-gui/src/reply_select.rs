//! Select mode for an assistant reply (2026-11-14): the reply draws as
//! StyledText (markdown), which Slint 1.18.1 cannot select, so its Select
//! link swaps it for a SelectableText of its
//! plain text (`plain_text`), every line selected. Esc, the focus moving
//! elsewhere (a click in another text, the composer), Done or a fold of
//! any row bring the reply back (`ui/transcript.slint`). One reply at a
//! time: the `ReplySelect` global holds its item. The chat stops following,
//! so the row keeps its top while its height changes (`virtual_list`).

use slint::{ComponentHandle, Model};

use crate::{MainWindow, ReplySelect};

/// Connects the Select action.
pub fn attach(ui: &MainWindow) {
    let weak = ui.as_weak();
    ui.global::<ReplySelect>().on_enter(move |item| {
        if let Some(ui) = weak.upgrade() {
            enter(&ui, item);
        }
    });
}

/// Puts the reply of transcript row `item` in select mode.
pub fn enter(ui: &MainWindow, item: i32) {
    let rows = ui.get_messages();
    let Some(row) = rows.iter().find(|r| r.item == item && r.kind == "assistant") else {
        return;
    };
    ui.set_transcript_follow(false);
    let global = ui.global::<ReplySelect>();
    global.set_text(crate::plain_text::plain(&row.text).into());
    global.set_item(item);
}

#[cfg(test)]
#[path = "reply_select_tests.rs"]
mod tests;
