//! The jobs strip (PLAN.md D68 "Views"): its row model, and in a software
//! window, drawn only while the session has jobs.

use std::rc::Rc;

use fast_harness_core::view::jobs::{JobRow, Mark};
use slint::platform::software_renderer::{
    MinimalSoftwareWindow, PremultipliedRgbaColor, RepaintBufferType,
};
use slint::platform::{Platform, WindowAdapter};
use slint::{ComponentHandle, ModelRc, PhysicalSize, VecModel};

slint::slint! {
    import { JobsRow } from "ui/jobs.slint";
    import { JobChip } from "ui/types.slint";
    export { JobChip }
    export component JobsHarness inherits Window {
        width: 600px;
        height: 80px;
        in property <[JobChip]> jobs;
        out property <string> killed;
        out property <length> used-width: strip.used-width;
        out property <length> strip-height: strip.height;
        VerticalLayout {
            strip := JobsRow {
                column: 560px;
                jobs: root.jobs;
            }
            // As in main.slint: no row while the session has no jobs.
            if root.jobs.length > 0: JobsRow {
                column: 560px;
                jobs: root.jobs;
                kill(id) => { root.killed = id; }
            }
            Rectangle {}
        }
    }
}

fn row(id: &str, word: &str, mark: Mark) -> JobRow {
    let (id, command, word) = (id.into(), format!("command of {id}"), word.into());
    JobRow {
        id,
        command,
        word,
        mark,
    }
}

fn rows() -> Vec<JobRow> {
    vec![
        row("job-1", "running", Mark::Running),
        row("job-2", "exited 0", Mark::Exited),
        row("job-3", "exited 3", Mark::Failed),
        row("job-4", "killed", Mark::Killed),
    ]
}

#[test]
fn the_row_model_names_each_state_and_keeps_its_word() {
    let chips = super::chips(&rows());
    let got: Vec<(&str, &str, &str)> = (chips.iter())
        .map(|c| (c.id.as_str(), c.mark.as_str(), c.word.as_str()))
        .collect();
    assert_eq!(
        got,
        [
            ("job-1", "running", "running"),
            ("job-2", "exited", "exited 0"),
            ("job-3", "failed", "exited 3"),
            ("job-4", "killed", "killed"),
        ]
    );
    assert_eq!(chips[0].command.as_str(), "command of job-1");
}

struct TestPlatform(Rc<MinimalSoftwareWindow>);

impl Platform for TestPlatform {
    fn create_window_adapter(&self) -> Result<Rc<dyn WindowAdapter>, slint::PlatformError> {
        Ok(self.0.clone())
    }
}

const W: usize = 600;
const H: usize = 80;

fn render(window: &MinimalSoftwareWindow) -> Vec<(u8, u8, u8, u8)> {
    let mut buffer = vec![PremultipliedRgbaColor::default(); W * H];
    window.request_redraw();
    window.draw_if_needed(|renderer| {
        renderer.render(&mut buffer, W);
    });
    (buffer.iter())
        .map(|p| (p.red, p.green, p.blue, p.alpha))
        .collect()
}

#[test]
fn the_strip_draws_only_while_the_session_has_jobs() {
    let window = MinimalSoftwareWindow::new(RepaintBufferType::NewBuffer);
    slint::platform::set_platform(Box::new(TestPlatform(window.clone()))).expect("platform");
    window.set_size(PhysicalSize::new(W as u32, H as u32));
    let ui = JobsHarness::new().expect("harness");
    ui.show().expect("show");
    let empty = render(&window);
    // The harness has its own JobChip type: the same fields.
    let model = |rows: &[JobRow]| {
        let chips = super::chips(rows).into_iter().map(|c| JobChip {
            id: c.id,
            command: c.command,
            word: c.word,
            mark: c.mark,
        });
        ModelRc::new(VecModel::from(chips.collect::<Vec<_>>()))
    };
    ui.set_jobs(model(&rows()));
    let shown = render(&window);
    assert_ne!(empty, shown, "the strip draws with jobs");
    ui.set_jobs(model(&[]));
    assert_eq!(empty, render(&window), "no strip without jobs");
    // Eight jobs (the cap) all fit the column, on more lines (found in the
    // real window, 2026-11-13: job-5 was cut and jobs 6 to 8 unreachable).
    let one_line = {
        ui.set_jobs(model(&rows()[..1]));
        ui.get_strip_height()
    };
    let eight: Vec<JobRow> = (1..=8).map(|i| row(&format!("job-{i}"), "running", Mark::Running)).collect();
    ui.set_jobs(model(&eight));
    let used = ui.get_used_width();
    assert!(used > 0.0 && used <= 560.0, "the chips take {used} px of 560");
    assert!(ui.get_strip_height() > one_line, "more than one line");
}
