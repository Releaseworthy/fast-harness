//! Screenshots of the main pages in every theme, rendered with the software
//! renderer into PNG files for a visual check:
//! `$env:NH_SHOTS_DIR='F:\shots'; cargo test -p nh-gui shots -- --ignored`
//! (default directory: `nh-gui-shots` in the temporary directory).

use std::path::PathBuf;
use std::rc::Rc;

use nh_core::theme::THEMES;
use slint::platform::software_renderer::{MinimalSoftwareWindow, RepaintBufferType};
use slint::platform::{Platform, WindowAdapter};
use slint::{ComponentHandle, ModelRc, PhysicalSize, Rgb8Pixel, SharedString, VecModel};

use crate::rows::markdown;
use crate::{shots_png, shots_trace, theme, AccountRow, MainWindow, MsgRow, SessionRow, TraceRow};

/// The sample timeline with the failed tool output opened by the user.
fn trace_entries() -> Vec<TraceRow> {
    let mut rows = shots_trace::entries();
    if let Some(row) = rows.iter_mut().find(|r| r.kind == "thinking") {
        row.expanded = true;
    }
    rows
}

const WIDTH: u32 = 1100;
const HEIGHT: u32 = 760;
/// The composer's effort dropdown (sidebar 260 + side 16 + padding 12 + model 200 + gap 8).
const EFFORT_X: f32 = 550.0;
const EFFORT_Y: f32 = 690.0;

struct ShotPlatform(Rc<MinimalSoftwareWindow>);

impl Platform for ShotPlatform {
    fn create_window_adapter(&self) -> Result<Rc<dyn WindowAdapter>, slint::PlatformError> {
        Ok(self.0.clone())
    }
}

fn msg(kind: &str, text: &str) -> MsgRow {
    MsgRow {
        kind: kind.into(),
        text: text.into(),
        body: markdown(text),
        ..MsgRow::default()
    }
}

fn tool(title: &str, summary: &str, output: &str, error: bool, running: bool) -> MsgRow {
    MsgRow {
        title: title.into(),
        summary: summary.into(),
        output: output.into(),
        error,
        running,
        expanded: error,
        ..msg("tool", output)
    }
}

fn note(tone: &str, text: &str) -> MsgRow {
    MsgRow {
        tone: tone.into(),
        ..msg("note", text)
    }
}

fn messages() -> Vec<MsgRow> {
    let answer = "The theme lives in `crates/nh-core/src/theme.rs`. **Every role** meets WCAG 2.2: \
                  body text at 7:1, captions at 4.5:1. See [the spec](https://www.w3.org/TR/WCAG22/).";
    vec![
        msg("user", "Add a theme picker to the settings page and keep the greyscale themes."),
        MsgRow {
            thinking: "The settings page already has model and effort; the theme goes above them.".into(),
            expanded: true,
            ..msg("assistant", answer)
        },
        tool("read", "crates/nh-gui/ui/pages.slint", "80 lines", false, false),
        tool("bash", "cargo test -p nh-gui", "error[E0425]: cannot find value `palette`", true, false),
        tool("grep", "Palette\\. in ui/*.slint", "", false, true),
        note("retry", "Overloaded (529): retrying in 4 s (attempt 2 of 5)"),
        note("switch", "Switched to account work@example.com (5h limit reached)"),
        note("error", "The request failed: connection reset"),
        note("info", "Session saved."),
        msg("footer", "turn 2 | step 3 | claude-opus-5-5 | work@example.com | TTFB 850 ms | total 3.2 s | in 1.2k out 300"),
    ]
}

fn sessions() -> Vec<SessionRow> {
    let row = |id: &str, title: &str, time: &str, current: bool| SessionRow {
        id: id.into(),
        title: title.into(),
        time: time.into(),
        current,
    };
    vec![
        row("1", "Theme picker for the GUI", "2 min ago", true),
        row("2", "Fix the retry backoff", "1 h ago", false),
        row("3", "Trace page columns", "yesterday", false),
    ]
}

fn account(name: &str, active: bool, five: f32, seven: f32, hold: &str) -> AccountRow {
    AccountRow {
        id: name.into(),
        name: name.into(),
        kind: "oauth".into(),
        plan: "max".into(),
        active,
        five,
        five_text: format!("{:.0}%", five * 100.0).into(),
        seven,
        seven_text: format!("{:.0}%", seven * 100.0).into(),
        hold: hold.into(),
        token_ok: !hold.is_empty() || active,
    }
}

fn model<T: Clone + 'static>(items: Vec<T>) -> ModelRc<T> {
    ModelRc::new(VecModel::from(items))
}

fn fill(ui: &MainWindow) {
    ui.set_messages(model(messages()));
    ui.set_sessions(model(sessions()));
    ui.set_accounts(model(vec![
        account("work@example.com", true, 0.42, 0.18, ""),
        account(
            "home@example.com",
            false,
            0.93,
            0.74,
            "On hold until 14:20: 5h limit reached",
        ),
        account("spare@example.com", false, 0.05, 0.02, ""),
    ]));
    let models = crate::pickers::model_list("claude-opus-5-5");
    ui.set_models(model(models.iter().map(|(_, l)| l.into()).collect()));
    let efforts = crate::pickers::effort_list("claude-opus-5-5");
    ui.set_efforts(model(
        efforts.iter().map(|s| SharedString::from(*s)).collect(),
    ));
    ui.set_effort_index(2);
    ui.set_trace_entries(model(trace_entries()));
    ui.set_trace_steps(model(shots_trace::steps()));
    ui.set_trace_session("2026-11-11T22-51-40-3f9c".into());
    ui.set_chip_name("work@example.com".into());
    ui.set_chip_five(0.42);
    ui.set_chip_five_text("42%".into());
    ui.set_chip_seven(0.81);
    ui.set_chip_seven_text("81%".into());
    ui.set_status_model("claude-opus-5-5".into());
    ui.set_status_account("work@example.com".into());
    ui.set_status_tokens("session: in 12.4k out 2.1k cache 80.2k / 4.0k".into());
    ui.set_status_timing("last step: TTFB 850 ms, total 3.2 s".into());
    ui.set_accounts_message("Usage refreshed 1 min ago.".into());
    ui.set_version("0.1.0".into());
}

fn render(window: &MinimalSoftwareWindow) -> Vec<u8> {
    let mut buffer = vec![Rgb8Pixel { r: 0, g: 0, b: 0 }; (WIDTH * HEIGHT) as usize];
    // Twice: the list views measure their rows on the first pass.
    for _ in 0..2 {
        slint::platform::update_timers_and_animations();
        window.request_redraw();
        window.draw_if_needed(|renderer| {
            renderer.render(&mut buffer, WIDTH as usize);
        });
    }
    buffer.iter().flat_map(|p| [p.r, p.g, p.b]).collect()
}

fn out_dir() -> PathBuf {
    std::env::var_os("NH_SHOTS_DIR")
        .map(PathBuf::from)
        .unwrap_or_else(|| std::env::temp_dir().join("nh-gui-shots"))
}

#[test]
#[ignore = "writes PNG screenshots for a visual check"]
fn shots() {
    let window = MinimalSoftwareWindow::new(RepaintBufferType::NewBuffer);
    slint::platform::set_platform(Box::new(ShotPlatform(window.clone()))).expect("platform");
    window.set_size(PhysicalSize::new(WIDTH, HEIGHT));
    let ui = MainWindow::new().expect("window");
    ui.show().expect("show");
    fill(&ui);
    let dir = out_dir();
    std::fs::create_dir_all(&dir).expect("shots directory");
    for core in THEMES.iter() {
        theme::apply(&ui, core);
        theme::fill(&ui, core.id);
        for shot in ["chat", "accounts", "settings", "trace", "steps"] {
            let page = if shot == "steps" { "trace" } else { shot };
            ui.set_trace_mode(if shot == "steps" { "steps" } else { "timeline" }.into());
            ui.set_page(page.into());
            ui.set_error_text(
                if page == "chat" {
                    "Cannot list the sessions: access denied"
                } else {
                    ""
                }
                .into(),
            );
            let png = shots_png::encode(WIDTH, HEIGHT, &render(&window));
            let path = dir.join(format!("{}-{shot}.png", core.id));
            std::fs::write(&path, png).expect("write png");
        }
        ui.set_page("chat".into());
        ui.set_error_text("".into());
        ui.set_picker_note("Effort set to high: claude-opus-4-5 does not take max.".into());
        // The effort dropdown of the composer, opened upwards.
        click(&window, EFFORT_X, EFFORT_Y);
        key(&window, slint::platform::Key::Return.into());
        let png = shots_png::encode(WIDTH, HEIGHT, &render(&window));
        std::fs::write(dir.join(format!("{}-composer.png", core.id)), png).expect("write png");
        key(&window, slint::platform::Key::Escape.into());
        ui.set_picker_note("".into());
        ui.set_page("settings".into());
        // The theme dropdown opened by a click, then closed with Escape.
        click(&window, 600.0, 104.0);
        // A second open by keyboard is a no-op when the click already opened it.
        key(&window, slint::platform::Key::Return.into());
        let png = shots_png::encode(WIDTH, HEIGHT, &render(&window));
        std::fs::write(dir.join(format!("{}-dropdown.png", core.id)), png).expect("write png");
        key(&window, slint::platform::Key::Escape.into());
    }
}

fn click(window: &MinimalSoftwareWindow, x: f32, y: f32) {
    use slint::platform::{PointerEventButton, WindowEvent};
    let position = slint::LogicalPosition::new(x, y);
    let button = PointerEventButton::Left;
    window.dispatch_event(WindowEvent::PointerMoved { position });
    window.dispatch_event(WindowEvent::PointerPressed { position, button });
    window.dispatch_event(WindowEvent::PointerReleased { position, button });
}

fn key(window: &MinimalSoftwareWindow, text: SharedString) {
    use slint::platform::WindowEvent;
    window.dispatch_event(WindowEvent::KeyPressed { text: text.clone() });
    window.dispatch_event(WindowEvent::KeyReleased { text });
}
