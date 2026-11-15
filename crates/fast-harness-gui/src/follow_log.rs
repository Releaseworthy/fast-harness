//! A debug log of the chat's scroll position, per sync and per frame: on
//! when `FAST_HARNESS_FOLLOW_LOG` names a file. Used to find a frame where
//! a following chat is not at its end (the owner, 2026-11-15: a new step
//! jumped the view up, then back down).

use std::cell::RefCell;
use std::fs::File;
use std::io::Write;
use std::time::Instant;

use slint::{ComponentHandle, Model, RenderingState};

use crate::MainWindow;

thread_local! {
    static LOG: RefCell<Option<(File, Instant)>> = RefCell::new(open());
}

fn open() -> Option<(File, Instant)> {
    let path = std::env::var_os("FAST_HARNESS_FOLLOW_LOG")?;
    Some((File::create(path).ok()?, Instant::now()))
}

/// True when the log is on.
pub fn on() -> bool {
    LOG.with(|log| log.borrow().is_some())
}

/// Writes the line `text` gives, when the log is on.
pub fn line(text: impl FnOnce() -> String) {
    LOG.with(|log| {
        if let Some((file, start)) = log.borrow_mut().as_mut() {
            let ms = start.elapsed().as_secs_f64() * 1000.0;
            let _ = writeln!(file, "{ms:10.1} {}", text());
        }
    });
}

/// The chat's numbers now: rows, content height, offset, view, end of the
/// last real row on screen, following.
pub fn state(ui: &MainWindow) -> String {
    let tops = ui.get_chat_tops();
    let total = tops.row_data(tops.row_count().saturating_sub(1)).unwrap_or(0.0);
    format!(
        "rows={} total={total:.1} offset={:.1} view={:.1} end={:.1} follow={}",
        ui.get_messages().row_count(),
        ui.get_chat_offset(),
        ui.get_chat_list_height(),
        ui.get_chat_end(),
        ui.get_transcript_follow(),
    )
}

/// Logs the chat's numbers before and after every frame. The notifier is
/// set from a timer once the window is shown: set at creation it never
/// fired (no renderer yet).
pub fn attach(ui: &MainWindow) {
    if !on() {
        return;
    }
    let weak = ui.as_weak();
    slint::Timer::single_shot(std::time::Duration::from_millis(1500), move || {
        if let Some(ui) = weak.upgrade() {
            notify(&ui);
        }
    });
}

fn notify(ui: &MainWindow) {
    let weak = ui.as_weak();
    let set = ui.window().set_rendering_notifier(move |state, _| {
        let Some(ui) = weak.upgrade() else {
            return;
        };
        let when = match state {
            RenderingState::BeforeRendering => "frame<",
            RenderingState::AfterRendering => "frame>",
            _ => return,
        };
        line(|| format!("{when} {}", state_of(&ui)));
    });
    match set {
        Ok(()) => line(|| "frame log on".to_string()),
        Err(err) => line(|| format!("no frame log: {err:?}")),
    }
}

fn state_of(ui: &MainWindow) -> String {
    state(ui)
}
