//! The tab bar's Subagents and Jobs tabs and the Settings and About dialogs
//! (PLAN.md D76) in the whole window, for a visual check:
//! `FAST_HARNESS_SHOTS_DIR=<dir> cargo test -p fast-harness-gui tabbar_shots
//! -- --ignored` writes `tabbar-tabs.png` (both tabs), `tabbar-subagents.png`
//! and `tabbar-jobs.png` (each popover open), `tabbar-settings.png`,
//! `tabbar-about.png` and `tabbar-child.png` (a subagent's session, read-only).

use std::path::PathBuf;
use std::rc::Rc;
use std::time::Duration;

use fast_harness_core::theme::builtins;
use fast_harness_core::view::jobs::{JobRow, Mark};
use fast_harness_core::view::Item;
use slint::platform::software_renderer::{MinimalSoftwareWindow, RepaintBufferType};
use slint::platform::{Platform, WindowAdapter};
use slint::{ComponentHandle, ModelRc, PhysicalSize, Rgb8Pixel, VecModel};

use crate::tabbar_tests::subagent;
use crate::{dialogs, shots_png, theme, JobsUi, MainWindow, SubagentsUi};

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

fn job(id: &str, command: &str, mark: Mark) -> JobRow {
    let running = mark == Mark::Running;
    JobRow {
        id: id.into(),
        command: command.into(),
        word: if running { "running" } else { "exited 0" }.into(),
        mark,
        took: (!running).then(|| Duration::from_secs(125)),
    }
}

/// One running background job, as the jobs view gives it.
pub fn jobs() -> Vec<crate::JobChip> {
    crate::jobs_view::chips(&[job("job-1", "cargo build --release", Mark::Running)])
}

fn rows() -> Vec<crate::MsgRow> {
    let user = Item::User {
        text: "Review the GUI tab bar and report what looks wrong.".into(),
        images: Vec::new(),
    };
    let text = Item::Assistant {
        turn: 1,
        step: 1,
        text: "I started three reviewers; two still run.".into(),
        thinking: String::new(),
    };
    [user, text].iter().map(crate::rows::msg_row).collect()
}

fn out_dir() -> PathBuf {
    std::env::var_os("FAST_HARNESS_SHOTS_DIR")
        .map(PathBuf::from)
        .unwrap_or_else(|| std::env::temp_dir().join("fast-harness-gui-shots"))
}

fn fill(ui: &MainWindow) {
    ui.set_page("chat".into());
    ui.set_version("0.1.0".into());
    ui.set_messages(ModelRc::new(VecModel::from(rows())));
    let subs = ui.global::<SubagentsUi>();
    let kids = vec![
        subagent("gui", true),
        subagent("core", true),
        subagent("docs", false),
    ];
    subs.set_rows(ModelRc::new(VecModel::from(kids)));
    subs.set_running(2);
    let jobs = ui.global::<JobsUi>();
    jobs.set_running(ModelRc::new(VecModel::from(self::jobs())));
    let done = job("job-2", "cargo test --workspace", Mark::Exited);
    jobs.set_finished(ModelRc::new(VecModel::from(crate::jobs_view::chips(&[
        done,
    ]))));
}

#[test]
#[ignore = "writes PNG screenshots for a visual check"]
fn tabbar_shots() {
    let window = MinimalSoftwareWindow::new(RepaintBufferType::NewBuffer);
    slint::platform::set_platform(Box::new(ShotPlatform(window.clone()))).expect("platform");
    window.set_size(PhysicalSize::new(W, H));
    let ui = crate::chat_list::new_window().expect("window");
    ui.show().expect("show");
    theme::apply(&ui, &builtins()[0], 0);
    fill(&ui);
    let dir = out_dir();
    std::fs::create_dir_all(&dir).expect("shots directory");
    let save = |name: &str| {
        let png = shots_png::encode(W, H, &render(&window));
        std::fs::write(dir.join(format!("tabbar-{name}.png")), png).expect("write png");
    };
    save("tabs");
    let (subs, jobs) = (ui.global::<SubagentsUi>(), ui.global::<JobsUi>());
    subs.set_open(true);
    save("subagents");
    subs.set_open(false);
    jobs.set_open(true);
    save("jobs");
    jobs.set_open(false);
    for name in ["settings", "about"] {
        dialogs::open(&ui, name);
        save(name);
        dialogs::close(&ui);
    }
    subs.set_back_label("Review the GUI tab bar".into());
    subs.set_in_child(true);
    save("child");
}
