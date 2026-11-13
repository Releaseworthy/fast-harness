//! The Jobs tab in the whole window (PLAN.md D68 "Views", D72): eight jobs keep
//! the transcript's rows drawn (a guard: a blank transcript with eight jobs
//! was seen once in the real window on 2026-11-13 and not reproduced since).

use std::rc::Rc;

use fast_harness_core::view::jobs::{JobRow, Mark};
use fast_harness_core::view::Item;
use slint::platform::software_renderer::{MinimalSoftwareWindow, RepaintBufferType};
use slint::platform::{Platform, WindowAdapter};
use slint::{ComponentHandle, ModelRc, PhysicalSize, Rgb8Pixel, VecModel};

use crate::MainWindow;

const W: u32 = 1000;
const H: u32 = 700;

struct TestPlatform(Rc<MinimalSoftwareWindow>);

impl Platform for TestPlatform {
    fn create_window_adapter(&self) -> Result<Rc<dyn WindowAdapter>, slint::PlatformError> {
        Ok(self.0.clone())
    }
}

fn frames(window: &MinimalSoftwareWindow) -> Vec<Rgb8Pixel> {
    let mut buffer = vec![Rgb8Pixel { r: 0, g: 0, b: 0 }; (W * H) as usize];
    for _ in 0..4 {
        slint::platform::update_timers_and_animations();
        window.request_redraw();
        window.draw_if_needed(|renderer| {
            renderer.render(&mut buffer, W as usize);
        });
    }
    buffer
}

fn jobs(count: usize) -> Vec<crate::JobChip> {
    let rows: Vec<JobRow> = (1..=count)
        .map(|i| JobRow {
            id: format!("job-{i}"),
            command: format!("ping -n 600 127.0.0.{i}"),
            word: "running".into(),
            mark: Mark::Running,
            took: None,
        })
        .collect();
    crate::jobs_view::chips(&rows)
}

fn rows() -> Vec<crate::MsgRow> {
    let user = Item::User {
        text: "JOBSNINE start them".into(),
        images: Vec::new(),
    };
    let text = Item::Assistant {
        turn: 1,
        step: 1,
        text: "started nine".into(),
        thinking: String::new(),
    };
    [user, text].iter().map(crate::rows::msg_row).collect()
}

/// The rows of a session with many jobs stay drawn: the transcript keeps its
/// height and width while the strip shows.
#[test]
fn many_jobs_keep_the_transcript_drawn() {
    let window = MinimalSoftwareWindow::new(RepaintBufferType::NewBuffer);
    slint::platform::set_platform(Box::new(TestPlatform(window.clone()))).expect("platform");
    window.set_size(PhysicalSize::new(W, H));
    let ui = MainWindow::new().expect("window");
    ui.show().expect("show");
    ui.set_page("chat".into());
    ui.set_messages(ModelRc::new(VecModel::from(rows())));
    let shot0 = frames(&window);
    let before = ui.get_transcript_content_height();
    ui.global::<crate::JobsUi>()
        .set_running(ModelRc::new(VecModel::from(jobs(8))));
    let shot1 = frames(&window);
    let after = ui.get_transcript_content_height();
    // Under the tab bar (the Jobs tab shows there, D76), down to row 300.
    let (bar, top) = ((W * 90) as usize, (W * 300) as usize);
    let differ = (bar..top).filter(|&i| shot0[i] != shot1[i]).count();
    let mut empty = rows();
    empty.clear();
    ui.global::<crate::JobsUi>()
        .set_running(ModelRc::new(VecModel::from(jobs(0))));
    ui.set_messages(ModelRc::new(VecModel::from(empty)));
    let shot2 = frames(&window);
    let blank = (0..top).filter(|&i| shot0[i] != shot2[i]).count();
    assert!(
        after > 0.0 && (after - before).abs() < 1.0,
        "{before} {after}"
    );
    assert_eq!(differ, 0, "the rows draw the same with eight jobs");
    assert!(blank > 0, "the rows were drawn at all");
}
