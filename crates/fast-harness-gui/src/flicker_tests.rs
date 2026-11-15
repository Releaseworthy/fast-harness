//! The flicker of a following chat while a step runs (the owner,
//! 2026-11-15: the screen flickered and the scroll bar came and went). The
//! status bar's token and timing words had no elide, so their width was the
//! main card's minimum: as the numbers grew the card widened by a few px
//! (840 -> 846 in the real window), the chat's width changed, and the list
//! dropped every measured height: a following chat jumped ~1900 px off its
//! end and measured every row again.

use slint::{Model, PhysicalSize};

use crate::scroll_tests::{following, frames};
use crate::MainWindow;

fn total(ui: &MainWindow) -> f32 {
    let tops = ui.get_chat_tops();
    tops.row_data(tops.row_count().saturating_sub(1))
        .unwrap_or(0.0)
}

#[test]
fn a_long_status_line_never_changes_the_chat_width() {
    let (window, ui, _rows) = following();
    let (width, height, offset) = (ui.get_chat_width(), total(&ui), ui.get_chat_offset());
    ui.set_status_state("Running 1 tool(s)".into());
    let tokens =
        "context 12% of 200K · cache 97% · session: in 1,234,567 out 345.6k cache 98.7M / 1.2M ";
    ui.set_status_tokens(tokens.repeat(4).into());
    ui.set_status_timing("last step: TTFB 1,234 ms, total 56,789 ms".into());
    frames(&window, 100);
    assert_eq!(ui.get_chat_width(), width, "the status bar sets no width");
    assert_eq!(total(&ui), height, "the rows keep their heights");
    assert_eq!(ui.get_chat_offset(), offset, "the view stays at the end");
    assert!(ui.get_transcript_follow(), "still following");
}

#[test]
fn a_width_change_while_following_keeps_the_heights_and_the_end() {
    let (window, ui, _rows) = following();
    let (height, offset) = (total(&ui), ui.get_chat_offset());
    window.set_size(PhysicalSize::new(1094, 760));
    // The sync the width change runs, before any row is laid out again.
    ui.invoke_chat_sync();
    let (now, at) = (total(&ui), ui.get_chat_offset());
    assert!(
        (now - height).abs() < 1.0,
        "no jump to estimates: {height} -> {now}"
    );
    assert!(
        (at - offset).abs() < 1.0,
        "the view holds: {offset} -> {at}"
    );
    frames(&window, 300);
    let end = (total(&ui) - ui.get_chat_list_height()).max(0.0);
    assert!(ui.get_transcript_follow(), "still following");
    assert!(
        (ui.get_chat_offset() - end).abs() < 1.0,
        "at the end once measured: {} {end}",
        ui.get_chat_offset()
    );
}
