//! The jobs pill and popover (PLAN.md D68 "Views", D72): the row model, the
//! panel's sections and Clear, and in a software window: no pill without
//! jobs, the popover opens and closes with Escape, a row shows its output.

use std::rc::Rc;
use std::time::Duration;

use fast_harness_core::view::jobs::{JobRow, Mark};
use slint::platform::software_renderer::{
    MinimalSoftwareWindow, PremultipliedRgbaColor, RepaintBufferType,
};
use slint::platform::{Platform, WindowAdapter, WindowEvent};
use slint::{ComponentHandle, ModelRc, PhysicalSize, SharedString, VecModel};

use super::{chips, took_text, Panel};

slint::slint! {
    import { JobsTab, JobsUi } from "ui/jobs.slint";
    import { JobChip } from "ui/types.slint";
    export { JobChip, JobsUi }
    export component JobsHarness inherits Window {
        width: 600px;
        height: 400px;
        // As in the tab bar (panel.slint): no Jobs tab without jobs.
        HorizontalLayout {
            y: 0;
            height: 40px;
            alignment: start;
            if JobsUi.count > 0: JobsTab { room: 340px; span: 560px; }
        }
    }
}

fn row(id: &str, word: &str, mark: Mark) -> JobRow {
    let (id, command, word) = (id.into(), format!("command of {id}"), word.into());
    let took = (mark != Mark::Running).then(|| Duration::from_secs(125));
    JobRow {
        id,
        command,
        word,
        mark,
        took,
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
    let chips = chips(&rows());
    let got: Vec<(&str, &str, &str, &str)> = (chips.iter())
        .map(|c| (c.id.as_str(), c.mark.as_str(), c.word.as_str(), c.when.as_str()))
        .collect();
    assert_eq!(
        got,
        [
            ("job-1", "running", "running", ""),
            ("job-2", "exited", "exited 0", "2m 05s"),
            ("job-3", "failed", "exited 3", "2m 05s"),
            ("job-4", "killed", "killed", "2m 05s"),
        ]
    );
    assert_eq!(chips[0].command.as_str(), "command of job-1");
}

#[test]
fn a_duration_reads_in_the_largest_two_units() {
    let text = |secs| took_text(Some(Duration::from_secs(secs)));
    assert_eq!((text(4), text(65), text(3720)), ("4s".into(), "1m 05s".into(), "1h 02m".into()));
    assert_eq!(took_text(None), "");
}

/// Clear takes the finished jobs off the list; a running job stays, and a
/// job that ends after the Clear is listed again.
#[test]
fn clear_hides_the_finished_jobs_only() {
    let mut panel = Panel {
        rows: rows(),
        ..Panel::default()
    };
    let ids = |rows: Vec<JobRow>| rows.into_iter().map(|r| r.id).collect::<Vec<_>>();
    let (running, finished) = panel.visible();
    assert_eq!((ids(running), ids(finished).len()), (vec!["job-1".to_string()], 3));
    panel.cleared.extend(["job-2", "job-3", "job-4"].map(String::from));
    let (running, finished) = panel.visible();
    assert_eq!((ids(running).len(), ids(finished).len()), (1, 0));
    panel.rows.push(row("job-5", "exited 0", Mark::Exited));
    assert_eq!(ids(panel.visible().1), ["job-5"]);
}

struct TestPlatform(Rc<MinimalSoftwareWindow>);

impl Platform for TestPlatform {
    fn create_window_adapter(&self) -> Result<Rc<dyn WindowAdapter>, slint::PlatformError> {
        Ok(self.0.clone())
    }
}

const W: usize = 600;
const H: usize = 400;

fn render(window: &MinimalSoftwareWindow) -> Vec<(u8, u8, u8, u8)> {
    let mut buffer = vec![PremultipliedRgbaColor::default(); W * H];
    for _ in 0..2 {
        slint::platform::update_timers_and_animations();
        window.request_redraw();
        window.draw_if_needed(|renderer| {        renderer.render(&mut buffer, W);    });
    }
    (buffer.iter())
        .map(|p| (p.red, p.green, p.blue, p.alpha))
        .collect()
}

fn model(rows: &[JobRow]) -> ModelRc<JobChip> {
    let chips = chips(rows).into_iter().map(|c| JobChip {
        id: c.id,
        command: c.command,
        word: c.word,
        mark: c.mark,
        when: c.when,
    });
    ModelRc::new(VecModel::from(chips.collect::<Vec<_>>()))
}

fn click(window: &MinimalSoftwareWindow, x: f32, y: f32) {
    use slint::platform::PointerEventButton;
    let position = slint::LogicalPosition::new(x, y);
    let button = PointerEventButton::Left;
    window.dispatch_event(WindowEvent::PointerMoved { position });
    window.dispatch_event(WindowEvent::PointerPressed { position, button });
    window.dispatch_event(WindowEvent::PointerReleased { position, button });
    window.dispatch_event(WindowEvent::PointerExited);
}

fn key(window: &MinimalSoftwareWindow, text: SharedString) {
    window.dispatch_event(WindowEvent::KeyPressed { text: text.clone() });
    window.dispatch_event(WindowEvent::KeyReleased { text });
}

#[test]
fn the_pill_shows_only_with_jobs_and_its_popover_closes_with_escape() {
    let window = MinimalSoftwareWindow::new(RepaintBufferType::NewBuffer);
    slint::platform::set_platform(Box::new(TestPlatform(window.clone()))).expect("platform");
    window.set_size(PhysicalSize::new(W as u32, H as u32));
    let ui = JobsHarness::new().expect("harness");
    ui.show().expect("show");
    let jobs = ui.global::<JobsUi>();
    let empty = render(&window);
    let (running, finished): (Vec<JobRow>, Vec<JobRow>) = rows().into_iter().partition(JobRow::running);
    jobs.set_running(model(&running));
    jobs.set_finished(model(&finished));
    let pill = render(&window);
    assert_ne!(empty, pill, "the tab draws with jobs");
    let closed = Rc::new(std::cell::Cell::new(0));
    let count = closed.clone();
    jobs.on_closed(move || count.set(count.get() + 1));
    // A click on the Jobs tab (top left) opens the popover under it.
    click(&window, 40.0, 20.0);
    assert!(jobs.get_open(), "the click opens the popover");
    let open = render(&window);
    assert_ne!(pill, open, "the popover draws");
    // A click on the last row (the popover starts at y 44: Running, one
    // row, Finished 3, then rows of 29px) asks for its output and keeps the
    // popover open.
    let asked = Rc::new(std::cell::RefCell::new(String::new()));
    let sink = asked.clone();
    jobs.on_expand(move |id| *sink.borrow_mut() = id.to_string());
    click(&window, 200.0, 196.0);
    assert_eq!((asked.borrow().as_str(), jobs.get_open()), ("job-4", true));
    jobs.set_expanded("job-1".into());
    jobs.set_output("line one\nline two\n".into());
    let expanded = render(&window);
    assert_ne!(open, expanded, "the expanded row shows its output");
    key(&window, slint::platform::Key::Escape.into());
    assert!(!jobs.get_open() && closed.get() == 1, "Escape closes it, closed() once");
    let after = render(&window);
    // Under the tab (the tab itself keeps its focus ring).
    let popover = (W * 50..W * 300).filter(|&i| after[i] != pill[i]).count();
    assert_eq!(popover, 0, "the popover area is clear again");
    jobs.set_running(model(&[]));
    jobs.set_finished(model(&[]));
    assert_eq!(empty, render(&window), "no tab without jobs");
}
