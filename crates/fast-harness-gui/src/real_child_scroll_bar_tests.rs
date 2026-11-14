//! The scroll bar on the owner's real session (2026-11-14, reports 2 and 3:
//! the thumb changed size as the view moved, a drag of the thumb jumped).
//! The session and its helpers: `real_child_scroll_tests`.

use crate::fold_view_tests::shot;
use crate::real_child_scroll_tests::{open, Chat, CHILDREN};
use crate::real_child_tests::{click_child, pump};

/// Pages up to the top, down to the end and up again (the view opens at
/// the end): an error when the view/content ratio (the thumb's share of
/// its track) moves more than 3 %.
fn sweep(chat: &Chat, what: &str) -> Result<(), String> {
    let mut ratios = vec![chat.view() / chat.content()];
    let mut frames = Vec::new();
    for down in [false, true, false] {
        for _ in 0..3000 {
            let before = chat.y();
            frames.push(chat.page(down));
            ratios.push(chat.view() / chat.content());
            if (chat.y() - before).abs() < 0.5 {
                break;
            }
        }
        shot(
            &chat.window,
            &format!("thumb-{what}-{}", if down { "end" } else { "top" }),
        );
    }
    frames.sort();
    let fold = |(lo, hi): (f32, f32), r: &f32| (lo.min(*r), hi.max(*r));
    let (lo, hi) = ratios.iter().fold((f32::MAX, f32::MIN), fold);
    let swing = (hi - lo) / hi * 100.0;
    let (pages, median, slowest) = (
        frames.len(),
        frames[frames.len() / 2],
        frames[frames.len() - 1],
    );
    eprintln!("{what}: {pages} pages, view/content {lo:.4} to {hi:.4} ({swing:.1} %), frame median {median:?}, slowest {slowest:?}");
    if swing <= 3.0 {
        return Ok(());
    }
    Err(format!("{what}: the thumb changes size while scrolling: view/content from {lo:.4} to {hi:.4} ({swing:.1} %) over {pages} pages"))
}

/// Report 2: the thumb kept changing size, on the parent and on the
/// largest child log (6143, 1.1 MB).
#[test]
#[ignore = "reads the owner's sessions under C:\\Users\\Yamasaki"]
fn real_child_scroll_the_thumb_keeps_its_size() {
    let chat = open("thumb");
    let parent = sweep(&chat, "parent");
    click_child(&chat.window, CHILDREN[2]);
    chat.settle();
    let (rows, content) = (chat.rows().len(), chat.content());
    eprintln!(
        "child {}: {rows} rows, content {content:.0} px",
        CHILDREN[2]
    );
    let child = sweep(&chat, "child-6143");
    let fails: Vec<String> = [parent, child]
        .into_iter()
        .filter_map(Result::err)
        .collect();
    assert!(fails.is_empty(), "{}", fails.join("\n"));
}

/// Report 3: a drag of the thumb from the top to the end in small steps
/// moves the view down only, never more than one screen per step.
#[test]
#[ignore = "reads the owner's sessions under C:\\Users\\Yamasaki"]
fn real_child_scroll_a_thumb_drag_moves_the_view_steadily() {
    let chat = open("drag");
    let x = chat.ui.get_transcript_thumb_x();
    chat.press(x, chat.ui.get_transcript_thumb_y());
    chat.move_to(x, chat.top() - 400.0);
    chat.release(x, chat.top() - 400.0);
    pump(&chat.window, 300);
    assert!(
        chat.y().abs() < 1.0,
        "a drag up to the top ends at content-y {}",
        chat.y()
    );
    let (view, content) = (chat.view(), chat.content());
    let steps = ((content / view).ceil() as usize * 2).max(20);
    let (from, to) = (chat.ui.get_transcript_thumb_y(), chat.top() + view);
    chat.press(x, from);
    let mut ys = vec![chat.y()];
    for k in 1..=steps {
        chat.move_to(x, from + (to - from) * k as f32 / steps as f32);
        pump(&chat.window, 32);
        ys.push(chat.y());
    }
    chat.release(x, to);
    pump(&chat.window, 100);
    shot(&chat.window, "drag-end");
    let moved: Vec<f32> = ys.windows(2).map(|w| w[0] - w[1]).collect();
    let largest = |a: (usize, f32), (k, d): (usize, &f32)| if *d > a.1 { (k, *d) } else { a };
    let (worst, jump) = moved.iter().enumerate().fold((0, 0.0), largest);
    let now = chat.content();
    eprintln!("drag: {steps} steps, content {content:.0} px (now {now:.0}), view {view:.0} px, largest step {jump:.0} px at {}", worst + 1);
    if let Some(k) = moved.iter().position(|d| *d < -0.5) {
        panic!(
            "a drag down moved the view back up at step {}: content-y {:.0} -> {:.0}",
            k + 1,
            ys[k],
            ys[k + 1]
        );
    }
    assert!(
        jump <= view + 0.5,
        "a drag step moved the view {jump:.0} px, more than one screen ({view:.0} px), at step {}",
        worst + 1
    );
    let end = view - now;
    assert!(
        (chat.y() - end).abs() <= 1.0,
        "the drag to the track's end stops at content-y {:.0}, not the end {end:.0}",
        chat.y()
    );
}
