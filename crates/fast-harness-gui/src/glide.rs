//! The glide of a following list to its end when a new row lands there
//! (the owner 2026-11-15: a new step made the chat jump by the row's
//! height in one frame). The view moves from where it was to the end over
//! `GLIDE` with an ease-out, never past the end and never up. A row that
//! grows at the end (a streaming reply) keeps the view pinned to the end:
//! a glide toward it lagged a line behind the text.

use std::cell::Cell;
use std::time::{Duration, Instant};

use slint::{Timer, TimerMode};

/// How long a glide takes, and the time between its frames.
pub const GLIDE: Duration = Duration::from_millis(150);
const FRAME: Duration = Duration::from_millis(8);

thread_local! {
    /// A test's clock (None: the real one), so a glide test counts frames
    /// at set times instead of racing a loaded machine.
    static CLOCK: Cell<Option<Instant>> = const { Cell::new(None) };
}

/// The time a glide reads.
fn now() -> Instant {
    CLOCK.with(Cell::get).unwrap_or_else(Instant::now)
}

/// Sets this thread's glide clock (None: the real one).
#[cfg(test)]
pub fn set_clock(at: Option<Instant>) {
    CLOCK.with(|clock| clock.set(at));
}

/// Ease-out cubic: 0 at 0, 1 at 1, rising, never above 1.
pub fn ease(t: f32) -> f32 {
    let t = t.clamp(0.0, 1.0);
    1.0 - (1.0 - t).powi(3)
}

#[derive(Default)]
pub struct Glide {
    from: f32,
    start: Option<Instant>,
    timer: Timer,
}

impl Glide {
    /// The offset to show for the end at `target` with the view at
    /// `offset`: `fresh` when a new row ends the list since the last call
    /// (a glide starts there), else pinned to the end unless a glide runs.
    pub fn at(&mut self, offset: f32, target: f32, fresh: bool, now: Instant) -> f32 {
        if target <= offset {
            // The end came up (rows shrank): the view must follow it.
            self.start = None;
            return target;
        }
        if fresh {
            self.from = offset;
            self.start = Some(now);
        }
        let Some(start) = self.start else {
            return target;
        };
        let t = now.saturating_duration_since(start).as_secs_f32() / GLIDE.as_secs_f32();
        if t >= 1.0 {
            self.start = None;
            return target;
        }
        let y = self.from + (target - self.from) * ease(t);
        y.clamp(offset, target)
    }

    /// `at` for now, with a frame timer calling `tick` while a glide runs.
    pub fn toward(
        &mut self,
        offset: f32,
        target: f32,
        fresh: bool,
        tick: impl FnMut() + 'static,
    ) -> f32 {
        let y = self.at(offset, target, fresh, now());
        if self.start.is_none() {
            self.timer.stop();
        } else if !self.timer.running() {
            self.timer.start(TimerMode::Repeated, FRAME, tick);
        }
        y
    }

    /// The view left the end: no glide.
    pub fn stop(&mut self) {
        self.start = None;
        self.timer.stop();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_new_row_glides_down_without_overshoot() {
        let mut glide = Glide::default();
        let t0 = Instant::now();
        let mut y = glide.at(1000.0, 1000.0, false, t0);
        assert_eq!(y, 1000.0, "at the end: nothing to do");
        y = glide.at(y, 1200.0, true, t0);
        assert_eq!(y, 1000.0, "a glide starts where the view is");
        let mut seen = vec![y];
        for ms in (8..=200).step_by(8) {
            y = glide.at(y, 1200.0, false, t0 + Duration::from_millis(ms));
            seen.push(y);
        }
        assert!(seen.windows(2).all(|w| w[1] >= w[0]), "never up: {seen:?}");
        assert!(seen.iter().all(|y| *y <= 1200.0), "never past: {seen:?}");
        let between = seen.iter().filter(|y| **y > 1000.5 && **y < 1199.5);
        assert!(between.count() >= 5, "it moves over frames: {seen:?}");
        assert_eq!(*seen.last().unwrap(), 1200.0, "it ends at the end");
    }

    #[test]
    fn a_growing_last_row_stays_pinned() {
        let mut glide = Glide::default();
        let t0 = Instant::now();
        assert_eq!(glide.at(1000.0, 1014.0, false, t0), 1014.0);
        // During a glide, growth moves the target and the view goes on.
        let y = glide.at(1014.0, 1200.0, true, t0);
        let later = glide.at(y, 1214.0, false, t0 + Duration::from_millis(500));
        assert_eq!(later, 1214.0);
        // Rows shrank: the view follows the end up.
        assert_eq!(glide.at(1214.0, 1100.0, false, t0), 1100.0);
    }
}
