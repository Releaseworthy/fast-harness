//! Select mode on an assistant reply: Select swaps the markdown for
//! its plain text, selected, without moving the row; Esc brings it back.

use slint::platform::{Key, WindowEvent};
use slint::{ComponentHandle, ModelRc, VecModel};

use crate::fold_view_tests::shot;
use crate::real_child_tests::{pump, window};
use crate::{MainWindow, MsgRow, ReplySelect};

const REPLY: &str = "## Plan\n\nRun **cargo test** first, then read [the log](https://example.com) and `cargo fmt`.\n- one\n- two";

fn row(item: i32, kind: &str, text: &str) -> MsgRow {
    MsgRow {
        item,
        kind: kind.into(),
        text: text.into(),
        shown: text.into(),
        blocks: crate::rows::reply_blocks(text),
        ..MsgRow::default()
    }
}

/// Row `item`'s top in the list view.
fn top(ui: &MainWindow, window: &slint::platform::software_renderer::MinimalSoftwareWindow, item: i32) -> f32 {
    ui.set_chat_watch_item(-1);
    ui.set_chat_watch_y(-100_000.0);
    pump(window, 16);
    ui.set_chat_watch_item(item);
    pump(window, 48);
    ui.get_chat_watch_y()
}

#[test]
fn select_mode_shows_a_reply_in_place_and_esc_restores_it() {
    let (window, ui) = window();
    let rows: Vec<MsgRow> = (0..8)
        .map(|i| match i % 2 {
            0 => row(i, "user", &format!("Question {i}")),
            _ => row(i, "assistant", REPLY),
        })
        .collect();
    ui.set_messages(ModelRc::new(VecModel::from(rows)));
    pump(&window, 400);
    let item = 5;
    let before = top(&ui, &window, item);
    let view = ui.get_chat_list_height();
    assert!((0.0..view - 120.0).contains(&before), "reply {item} is not in view: y {before}");
    // The Select link's action (a double click cannot reach the reply:
    // StyledText takes the presses).
    ui.global::<ReplySelect>().invoke_enter(item);
    pump(&window, 300);
    let global = ui.global::<ReplySelect>();
    assert_eq!(global.get_item(), item, "Select starts select mode");
    assert_eq!(global.get_drawn(), crate::plain_text::plain(REPLY));
    assert!(!global.get_drawn().contains("**"), "{}", global.get_drawn());
    let during = top(&ui, &window, item);
    assert!((during - before).abs() <= 1.0, "select mode moved the reply from y {before} to {during}");
    shot(&window, "reply-select");
    window.dispatch_event(WindowEvent::KeyPressed { text: Key::Escape.into() });
    window.dispatch_event(WindowEvent::KeyReleased { text: Key::Escape.into() });
    pump(&window, 300);
    assert_eq!(global.get_item(), -1, "Esc ends select mode");
    let after = top(&ui, &window, item);
    assert!((after - before).abs() <= 1.0, "the reply came back at y {after}, not {before}");
}
