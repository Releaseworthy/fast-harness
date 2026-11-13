//! The one winit event hook of the main window (a window keeps only one):
//! dropped files attach, a resize re-reads the maximized state
//! (`window_state`, no polling timer), and the composer's caret rests.
//!
//! Caret rest: Slint 1.18.1 blinks the caret of a focused TextInput with a
//! repeating timer for as long as it believes the window is active
//! (`TextCursorBlinker`, half of the `GetCaretBlinkTime` cycle), and every
//! blink is a frame: the idle window with the composer focused used 140 to
//! 300 ms of CPU per 30 s. Slint has no public switch for it: the TextInput
//! has no blink property, and the flash cycle is a method of the winit
//! backend's `Platform`, which the `slint` crate does not re-export. So once
//! the caret timeout passes without input while the composer has focus, the
//! window tells Slint it is inactive: the TextInput hides its caret and drops
//! its blinker, keeping the focus item, the text, the cursor and the
//! selection (a `WindowActivation` focus-out). The next keyboard, mouse or
//! input-method event first tells Slint the window is active again, then
//! Slint handles it. The composer keeps its focus look (`Caret.resting`),
//! and an open input-method composition never rests (Slint clears the
//! preedit text on that focus-out). Windows stops its own caret after
//! `SPI_GETCARETTIMEOUT` (5000 ms by default) the same way.

use std::cell::Cell;
use std::path::PathBuf;
use std::time::Duration;

use slint::platform::WindowEvent as SlintEvent;
use slint::winit_030::winit::dpi::PhysicalSize;
use slint::winit_030::winit::event::{Ime, InnerSizeWriter, WindowEvent};
use slint::winit_030::{EventResult, WinitWindowAccessor};
use slint::{ComponentHandle, Timer, TimerMode};

use crate::{Caret, MainWindow};

/// The rest delay when the OS gives none (the Windows default caret timeout).
pub const REST_AFTER: Duration = Duration::from_millis(5000);

/// Whether Slint was told the window is inactive, and whether an input
/// method composition is open.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct Rest {
    resting: bool,
    composing: bool,
}

impl Rest {
    /// Keyboard, mouse or input-method input: `true` when Slint must hear
    /// the window is active again before the input.
    pub fn input(&mut self) -> bool {
        std::mem::take(&mut self.resting)
    }

    /// The delay passed: `true` when Slint must hear the window is inactive.
    /// Only while the composer has focus (so Slint sees the window active and
    /// a caret blinks), no composition is open and it is not resting yet.
    pub fn due(&mut self, composer_focused: bool) -> bool {
        let rest = composer_focused && !self.composing && !self.resting;
        self.resting |= rest;
        rest
    }

    /// The real window gained or lost activation: Slint hears that itself.
    pub fn activation(&mut self) {
        self.resting = false;
    }

    /// An input-method event: a non-empty preedit text is an open composition.
    pub fn ime(&mut self, ime: &Ime) {
        self.composing = matches!(ime, Ime::Preedit(text, _) if !text.is_empty());
    }
}

/// What the hook does with one winit event.
#[derive(Debug, PartialEq)]
enum Kind {
    Drop(PathBuf),
    Resize,
    Scale,
    Activation(bool),
    Ime,
    Input,
    Other,
}

fn kind(event: &WindowEvent) -> Kind {
    match event {
        WindowEvent::DroppedFile(path) => Kind::Drop(path.clone()),
        WindowEvent::Resized(_) => Kind::Resize,
        WindowEvent::ScaleFactorChanged { .. } => Kind::Scale,
        WindowEvent::Focused(active) => Kind::Activation(*active),
        WindowEvent::Ime(_) => Kind::Ime,
        WindowEvent::KeyboardInput { .. }
        | WindowEvent::MouseInput { .. }
        | WindowEvent::MouseWheel { .. }
        | WindowEvent::CursorMoved { .. } => Kind::Input,
        _ => Kind::Other,
    }
}

thread_local! {
    static STATE: Cell<Rest> = Cell::new(Rest::default());
    static DELAY: Cell<Duration> = const { Cell::new(REST_AFTER) };
    static TIMER: Timer = Timer::default();
}

fn with_rest<R>(f: impl FnOnce(&mut Rest) -> R) -> R {
    STATE.with(|cell| {
        let mut rest = cell.get();
        let out = f(&mut rest);
        cell.set(rest);
        out
    })
}

/// Restarts the idle countdown. It fires once, so an idle window keeps no
/// timer after it.
fn arm(ui: &MainWindow) {
    TIMER.with(|timer| {
        if timer.running() {
            timer.restart();
            return;
        }
        let weak = ui.as_weak();
        timer.start(TimerMode::SingleShot, DELAY.with(Cell::get), move || {
            if let Some(ui) = weak.upgrade() {
                rest(&ui);
            }
        });
    });
}

fn rest(ui: &MainWindow) {
    let caret = ui.global::<Caret>();
    if with_rest(|rest| rest.due(caret.get_composer())) {
        caret.set_resting(true);
        ui.window()
            .dispatch_event(SlintEvent::WindowActiveChanged(false));
    }
}

fn wake(ui: &MainWindow, window: &slint::Window) {
    if with_rest(Rest::input) {
        ui.global::<Caret>().set_resting(false);
        window.dispatch_event(SlintEvent::WindowActiveChanged(true));
    }
    arm(ui);
}

fn activation(ui: &MainWindow, active: bool) {
    with_rest(Rest::activation);
    ui.global::<Caret>().set_resting(false);
    match active {
        true => arm(ui),
        false => TIMER.with(Timer::stop),
    }
}

/// The window moved to a monitor with another Windows scale: the UI scale
/// is fixed (`display`, D75), so the physical size stays as it is. winit
/// would otherwise apply its suggested size (the logical size at the new
/// scale), shrinking or growing the window while the content stays 1:1.
fn keep_size(window: &slint::Window, mut writer: InnerSizeWriter) {
    let size = window.size();
    let _ = writer.request_inner_size(PhysicalSize::new(size.width, size.height));
}

fn on_event(ui: &MainWindow, window: &slint::Window, event: &WindowEvent) -> EventResult {
    match kind(event) {
        Kind::Drop(path) => {
            crate::images_view::dropped(path);
            return EventResult::PreventDefault;
        }
        Kind::Resize => crate::window_state::sync(ui),
        Kind::Scale => {
            if let WindowEvent::ScaleFactorChanged { inner_size_writer, .. } = event {
                keep_size(window, inner_size_writer.clone());
            }
        }
        Kind::Activation(active) => activation(ui, active),
        Kind::Ime => {
            if let WindowEvent::Ime(ime) = event {
                with_rest(|rest| rest.ime(ime));
            }
            wake(ui, window);
        }
        Kind::Input => wake(ui, window),
        Kind::Other => {}
    }
    EventResult::Propagate
}

/// Installs the hook and starts the first countdown: the rest does not wait
/// for a winit `Focused(true)`, which a window started in the background
/// may never get.
pub fn connect(ui: &MainWindow) {
    let delay = fast_harness_core::platform::caret_timeout().unwrap_or(REST_AFTER);
    DELAY.with(|cell| cell.set(delay));
    let weak = ui.as_weak();
    ui.window().on_winit_window_event(move |window, event| {
        match weak.upgrade() {
            Some(ui) => on_event(&ui, window, event),
            None => EventResult::Propagate,
        }
    });
    arm(ui);
}

#[cfg(test)]
#[path = "window_events_tests.rs"]
mod tests;
