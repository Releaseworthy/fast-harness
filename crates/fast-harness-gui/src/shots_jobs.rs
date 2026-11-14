//! The jobs pill and popover (D72) in the whole window, for a visual check:
//! `FAST_HARNESS_SHOTS_DIR=<dir> cargo test -p fast-harness-gui jobs_shots
//! -- --ignored` writes `jobs-none.png` (no jobs, no pill), `jobs-pill.png`
//! (two jobs, closed) and `jobs-open.png` (open, the running job's output).

use std::path::PathBuf;
use std::rc::Rc;
use std::time::Duration;

use fast_harness_core::theme::builtins;
use fast_harness_core::view::jobs::{JobRow, Mark};
use slint::platform::software_renderer::{MinimalSoftwareWindow, RepaintBufferType};
use slint::platform::{Platform, WindowAdapter};
use slint::{ComponentHandle, ModelRc, PhysicalSize, Rgb8Pixel, VecModel};

use crate::{jobs_view, shots_png, theme, JobsUi};

const W: u32 = 1100;
const H: u32 = 760;

struct ShotPlatform(Rc<MinimalSoftwareWindow>);

impl Platform for ShotPlatform {
    fn create_window_adapter(&self) -> Result<Rc<dyn WindowAdapter>, slint::PlatformError> {
        Ok(self.0.clone())
    }
}

fn render(window: &MinimalSoftwareWindow) -> Vec<u8> {
    let mut buffer = vec![Rgb8Pixel { r: 0, g: 0, b: 0 }; (W * H) as usize];
    for _ in 0..3 {
        slint::platform::update_timers_and_animations();
        window.request_redraw();
        window.draw_if_needed(|renderer| {
            renderer.render(&mut buffer, W as usize);
        });
    }
    buffer.iter().flat_map(|p| [p.r, p.g, p.b]).collect()
}

fn job(id: &str, command: &str, mark: Mark, took: Option<u64>) -> JobRow {
    let word = if mark == Mark::Running {
        "running"
    } else {
        "exited 0"
    };
    JobRow {
        id: id.into(),
        command: command.into(),
        word: word.into(),
        mark,
        took: took.map(Duration::from_secs),
    }
}

fn model(rows: &[JobRow]) -> ModelRc<crate::JobChip> {
    ModelRc::new(VecModel::from(jobs_view::chips(rows)))
}

fn output() -> String {
    let lines = (1..=24).map(|i| format!("   Compiling crate-{i:02} v0.{i}.0\n"));
    lines.collect::<String>() + "    Building [=======>          ] 141/388: fast-harness-core\n"
}

fn out_dir() -> PathBuf {
    std::env::var_os("FAST_HARNESS_SHOTS_DIR")
        .map(PathBuf::from)
        .unwrap_or_else(|| std::env::temp_dir().join("fast-harness-gui-shots"))
}

#[test]
#[ignore = "writes PNG screenshots for a visual check"]
fn jobs_shots() {
    let window = MinimalSoftwareWindow::new(RepaintBufferType::NewBuffer);
    slint::platform::set_platform(Box::new(ShotPlatform(window.clone()))).expect("platform");
    window.set_size(PhysicalSize::new(W, H));
    let ui = crate::chat_list::new_window().expect("window");
    ui.show().expect("show");
    theme::apply(&ui, &builtins()[0], 0);
    ui.set_page("chat".into());
    let dir = out_dir();
    std::fs::create_dir_all(&dir).expect("shots directory");
    let save = |name: &str| {
        let png = shots_png::encode(W, H, &render(&window));
        std::fs::write(dir.join(format!("jobs-{name}.png")), png).expect("write png");
    };
    save("none");
    let jobs = ui.global::<JobsUi>();
    let build = job("job-1", "cargo build --release", Mark::Running, None);
    let test = job("job-2", "cargo test --workspace", Mark::Exited, Some(125));
    jobs.set_running(model(&[build]));
    jobs.set_finished(model(&[test]));
    save("pill");
    jobs.set_open(true);
    jobs.set_expanded("job-1".into());
    jobs.set_output(output().into());
    jobs.set_dropped(true);
    save("open");
}
