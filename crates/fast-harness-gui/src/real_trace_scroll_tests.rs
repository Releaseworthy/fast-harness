//! The Trace tab's scroll bar on the owner's real session (2026-11-14: the
//! thumb changed size while dragging, as the chat's did before 3e84c6f;
//! Slint's ListView estimated the content height from the rows it had laid
//! out). The trace view is now `src/trace_list.rs` on `virtual_list`.

use std::path::{Path, PathBuf};
use std::rc::Rc;
use std::time::{Duration, Instant};

use fast_harness_core::session;
use slint::platform::software_renderer::MinimalSoftwareWindow;
use slint::platform::{PointerEventButton, WindowEvent};
use slint::{ComponentHandle, LogicalPosition, Model};

use crate::fold_view_tests::shot;
use crate::real_child_tests::{pump, window};
use crate::{MainWindow, TraceProbe};

const OWNER: &str = r"C:\Users\Yamasaki\.fast-harness";
/// The owner's largest child log (1.1 MB).
const LOG: &str = "20261113-195342-6143";

struct Trace {
    window: Rc<MinimalSoftwareWindow>,
    ui: MainWindow,
}

/// The owner's log on the Trace tab's timeline, settled.
fn open(tag: &str) -> Trace {
    let (window, ui) = window();
    let cwd = PathBuf::from(r"F:\work\fast-harness-general");
    let path = session::dir(Path::new(OWNER), &cwd).join(format!("{LOG}.jsonl"));
    ui.set_page("trace".into());
    ui.set_trace_mode("timeline".into());
    crate::trace_rows::show(&ui, crate::trace_rows::load(&path));
    let trace = Trace { window, ui };
    trace.settle();
    let rows = trace.ui.get_trace_entries().row_count();
    let (content, view) = (trace.content(), trace.view());
    eprintln!("{tag}: {rows} entries, content {content:.0} px, view {view:.0} px");
    assert!(content > 20.0 * view, "the log is too short for this test");
    trace
}

impl Trace {
    fn probe(&self) -> TraceProbe<'_> {
        self.ui.global::<TraceProbe>()
    }
    fn content(&self) -> f32 {
        self.probe().get_content_height()
    }
    fn view(&self) -> f32 {
        self.probe().get_visible_height()
    }
    fn y(&self) -> f32 {
        self.probe().get_view_y()
    }
    fn ratio(&self) -> f32 {
        self.view() / self.content()
    }
    /// Pumps until the content height holds for 10 frames (at most 10 s).
    fn settle(&self) {
        let start = Instant::now();
        let (mut last, mut same) = (f32::NAN, 0);
        while same < 10 && start.elapsed() < Duration::from_secs(10) {
            pump(&self.window, 16);
            let now = self.content();
            same = if (now - last).abs() < 0.5 {
                same + 1
            } else {
                0
            };
            last = now;
        }
    }
    fn move_to(&self, x: f32, y: f32) {
        let position = LogicalPosition::new(x, y);
        self.window
            .dispatch_event(WindowEvent::PointerMoved { position });
    }
    fn press(&self, x: f32, y: f32) {
        self.move_to(x, y);
        let (position, button) = (LogicalPosition::new(x, y), PointerEventButton::Left);
        self.window
            .dispatch_event(WindowEvent::PointerPressed { position, button });
    }
    fn release(&self, x: f32, y: f32) {
        let (position, button) = (LogicalPosition::new(x, y), PointerEventButton::Left);
        self.window
            .dispatch_event(WindowEvent::PointerReleased { position, button });
    }
}

/// The swing of the view/content ratios (the thumb's share of its track),
/// in percent of the largest.
fn swing(ratios: &[f32]) -> (f32, f32, f32) {
    let fold = |(lo, hi): (f32, f32), r: &f32| (lo.min(*r), hi.max(*r));
    let (lo, hi) = ratios.iter().fold((f32::MAX, f32::MIN), fold);
    (lo, hi, (hi - lo) / hi * 100.0)
}

/// A drag of the thumb from the top to the end in small steps: the thumb
/// keeps its size (view/content within 3 %), the view moves down only,
/// never more than one screen per step, and stops at the end.
#[test]
#[ignore = "reads the owner's sessions under C:\\Users\\Yamasaki"]
fn real_trace_a_thumb_drag_keeps_the_thumb_size() {
    let trace = open("trace-drag");
    let x = trace.probe().get_thumb_x();
    let top = trace.probe().get_list_top();
    let (view, content) = (trace.view(), trace.content());
    let steps = ((content / view).ceil() as usize * 2).max(20);
    let (from, to) = (trace.probe().get_thumb_y(), top + view);
    trace.press(x, from);
    let (mut ys, mut ratios) = (vec![trace.y()], vec![trace.ratio()]);
    for k in 1..=steps {
        trace.move_to(x, from + (to - from) * k as f32 / steps as f32);
        pump(&trace.window, 32);
        ys.push(trace.y());
        ratios.push(trace.ratio());
    }
    trace.release(x, to);
    pump(&trace.window, 100);
    shot(&trace.window, "trace-drag-end");
    let (lo, hi, swing) = swing(&ratios);
    let moved: Vec<f32> = ys.windows(2).map(|w| w[0] - w[1]).collect();
    let jump = moved.iter().copied().fold(0.0, f32::max);
    eprintln!("drag: {steps} steps, view/content {lo:.4} to {hi:.4} ({swing:.1} %), largest step {jump:.0} px");
    assert!(
        swing <= 3.0,
        "the thumb changes size during a drag: view/content from {lo:.4} to {hi:.4} ({swing:.1} %)"
    );
    if let Some(k) = moved.iter().position(|d| *d < -0.5) {
        panic!(
            "a drag down moved the view back up at step {}: {:.0} -> {:.0}",
            k + 1,
            ys[k],
            ys[k + 1]
        );
    }
    assert!(
        jump <= view + 0.5,
        "a drag step moved the view {jump:.0} px, more than one screen ({view:.0} px)"
    );
    let end = view - trace.content();
    assert!(
        (trace.y() - end).abs() <= 1.0,
        "the drag to the track's end stops at {:.0}, not the end {end:.0}",
        trace.y()
    );
}

/// The wheel from the top to the end and back: the thumb keeps its size.
#[test]
#[ignore = "reads the owner's sessions under C:\\Users\\Yamasaki"]
fn real_trace_the_wheel_keeps_the_thumb_size() {
    let trace = open("trace-wheel");
    let (x, top) = (300.0, trace.probe().get_list_top() + 100.0);
    let mut ratios = vec![trace.ratio()];
    for down in [true, false] {
        for _ in 0..5000 {
            let before = trace.y();
            let position = LogicalPosition::new(x, top);
            let delta_y = if down { -600.0 } else { 600.0 };
            let event = WindowEvent::PointerScrolled {
                position,
                delta_x: 0.0,
                delta_y,
            };
            trace.window.dispatch_event(event);
            pump(&trace.window, 32);
            ratios.push(trace.ratio());
            if (trace.y() - before).abs() < 0.5 {
                break;
            }
        }
    }
    let (lo, hi, swing) = swing(&ratios);
    eprintln!(
        "wheel: {} moves, view/content {lo:.4} to {hi:.4} ({swing:.1} %)",
        ratios.len()
    );
    assert!(swing <= 3.0, "the thumb changes size under the wheel: view/content from {lo:.4} to {hi:.4} ({swing:.1} %)");
}
