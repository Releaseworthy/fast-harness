//! The chat's follow mode near the end (the owner 2026-11-15): a scroll
//! down to within a line of the end follows again, and a new row at the
//! end of a following chat glides into view instead of jumping.

use std::time::Duration;

use slint::platform::WindowEvent;
use slint::{LogicalPosition, Rgb8Pixel};

use crate::scroll_tests::{following, frames, row};

/// A wheel step over the rows (`delta_y` > 0 scrolls up).
fn wheel(window: &slint::platform::software_renderer::MinimalSoftwareWindow, delta_y: f32) {
    let position = LogicalPosition::new(700.0, 300.0);
    window.dispatch_event(WindowEvent::PointerMoved { position });
    window.dispatch_event(WindowEvent::PointerScrolled {
        position,
        delta_x: 0.0,
        delta_y,
    });
    frames(window, 50);
}

#[test]
fn a_scroll_down_near_the_end_follows_again() {
    let (window, ui, _rows) = following();
    let end = ui.get_chat_offset();
    wheel(&window, 100.0);
    assert!(!ui.get_transcript_follow(), "a scroll up stops following");
    wheel(&window, -30.0);
    assert!(!ui.get_transcript_follow(), "70 px above the end: not yet");
    wheel(&window, -30.0);
    // 40 px above the end, moving down: within a line of it.
    assert!(ui.get_transcript_follow(), "near the end it follows again");
    frames(&window, 300);
    let at = ui.get_chat_offset();
    assert!((at - end).abs() < 0.5, "at the end: {at} {end}");
    wheel(&window, 20.0);
    assert!(
        !ui.get_transcript_follow(),
        "a scroll up near the end leaves"
    );
}

#[test]
fn a_new_row_glides_into_view() {
    let (window, ui, rows) = following();
    let start = ui.get_chat_offset();
    rows.push(row("assistant", &"A new step line.\n\n".repeat(12)));
    let mut seen = vec![start];
    let mut buffer = vec![Rgb8Pixel { r: 0, g: 0, b: 0 }; 1100 * 760];
    for _ in 0..40 {
        std::thread::sleep(Duration::from_millis(8));
        slint::platform::update_timers_and_animations();
        window.request_redraw();
        window.draw_if_needed(|renderer| {
            renderer.render(&mut buffer, 1100);
            seen.push(ui.get_chat_offset());
        });
    }
    let last = *seen.last().expect("frames");
    assert!(last > start + 100.0, "it shows the new row: {seen:?}");
    assert!(seen.windows(2).all(|w| w[1] >= w[0]), "never up: {seen:?}");
    assert!(seen.iter().all(|y| *y <= last), "never past: {seen:?}");
    let between = seen
        .iter()
        .filter(|y| **y > start + 0.5 && **y < last - 0.5);
    assert!(between.count() >= 3, "it moves over frames: {seen:?}");
    assert!(ui.get_transcript_follow(), "still following");
}
