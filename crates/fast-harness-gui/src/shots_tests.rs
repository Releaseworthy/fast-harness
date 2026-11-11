//! Screenshots of the main pages in every theme, rendered with the software
//! renderer into PNG files for a visual check:
//! `$env:FAST_HARNESS_SHOTS_DIR='F:\shots'; cargo test -p fast-harness-gui shots -- --ignored`
//! (default directory: `fast-harness-gui-shots` in the temporary directory).

use std::path::PathBuf;
use std::rc::Rc;

use fast_harness_core::theme::THEMES;
use slint::platform::software_renderer::{MinimalSoftwareWindow, RepaintBufferType};
use slint::platform::{Platform, WindowAdapter};
use slint::{ComponentHandle, ModelRc, PhysicalSize, Rgb8Pixel, SharedString, VecModel};

use crate::rows::markdown;
use crate::{shots_png, shots_trace, theme, AccountRow, MainWindow, MsgRow, SideRow, TraceRow};

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
/// The composer's effort dropdown (sidebar 260 + card edge 1 + side 16 + padding 12
/// + model 200 + gap 8).
const EFFORT_X: f32 = 551.0;
/// The composer's model dropdown (200 wide, left of the effort one).
const MODEL_X: f32 = 443.0;
/// The page content starts under the title bar (40), the card edge (1) and the tabs (38).
const PAGE_Y: f32 = 79.0;
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
        shown: text.into(),
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
    let answer = "The theme lives in `crates/fast-harness-core/src/theme.rs`. **Every role** meets WCAG 2.2: \
                  body text at 7:1, captions at 4.5:1. See [the spec](https://www.w3.org/TR/WCAG22/).";
    vec![
        msg("user", "Add a theme picker to the settings page and keep the greyscale themes."),
        MsgRow {
            thinking: "The settings page already has model and effort; the theme goes above them.".into(),
            expanded: true,
            ..msg("assistant", answer)
        },
        tool("read", "crates/fast-harness-gui/ui/pages.slint", "80 lines", false, false),
        tool("bash", "cargo test -p fast-harness-gui", "error[E0425]: cannot find value `palette`", true, false),
        tool("grep", "Palette\\. in ui/*.slint", "", false, true),
        note("retry", "Overloaded (529): retrying in 4 s (attempt 2 of 5)"),
        note("switch", "Switched to account work@example.com (5h limit reached)"),
        note("error", "The request failed: connection reset"),
        note("info", "Session saved."),
        msg("footer", "turn 2 | step 3 | claude-opus-5-5 | work@example.com | TTFB 850 ms | total 3.2 s | in 1.2k out 300"),
    ]
}

fn side_rows() -> Vec<SideRow> {
    use crate::workspaces::workspace_row;
    use std::path::Path;
    let ws = "F:\\work\\fast-harness";
    let session = |id: &str, title: &str, time: &str, current: bool| SideRow {
        kind: "session".into(),
        id: id.into(),
        workspace: ws.into(),
        title: title.into(),
        detail: time.into(),
        current,
        expanded: false,
    };
    vec![
        workspace_row(Path::new("F:\\work\\auto-warband"), false, false),
        workspace_row(Path::new(ws), true, true),
        session("1", "Theme picker for the GUI", "2 min ago", true),
        session("2", "Fix the retry backoff", "1 h ago", false),
        session("3", "Trace page columns", "yesterday", false),
        workspace_row(Path::new("F:\\work\\releaseworthy-site"), false, false),
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

fn claude_rows() -> Vec<AccountRow> {
    vec![
        account("work@example.com", true, 0.42, 0.18, ""),
        account(
            "home@example.com",
            false,
            0.93,
            0.74,
            "On hold until 14:20: 5h limit reached",
        ),
        account("spare@example.com", false, 0.05, 0.02, ""),
    ]
}

/// Two fake Google accounts (active with quota, held for verification).
fn google_rows() -> Vec<crate::GoogleRow> {
    let now = 1_800_000_000_000;
    let list = crate::google_fixtures::statuses(now);
    let project = |i| (i == 0).then(|| "proj-example-1".to_string());
    list.iter()
        .enumerate()
        .map(|(i, s)| crate::google_rows::google_row(s, project(i), now))
        .collect()
}

/// Three Claude models, then three Antigravity ones (both picker groups in
/// the eight rows the list shows).
fn catalog() -> Vec<fast_harness_core::llm::anthropic::ModelInfo> {
    use fast_harness_core::llm::antigravity::models::{to_info, CatalogEntry};
    let mut list = fast_harness_core::llm::anthropic::builtin_models();
    list.truncate(3);
    for (id, name) in [
        ("gemini-3-flash", "Gemini 3 Flash"),
        ("gemini-3.1-pro", "Gemini 3.1 Pro"),
        ("claude-sonnet-5-5", "Claude Sonnet 5.5"),
    ] {
        list.push(to_info(&CatalogEntry {
            id: id.into(),
            display_name: name.into(),
            ..CatalogEntry::default()
        }));
    }
    list
}

fn fill(ui: &MainWindow) {
    ui.set_messages(model(messages()));
    ui.set_side_rows(model(side_rows()));
    ui.set_accounts(model(claude_rows()));
    ui.set_google_accounts(model(google_rows()));
    let models = crate::pickers::model_list(&catalog(), "claude-opus-5-5");
    ui.set_models(model(models.iter().map(|(_, l)| l.into()).collect()));
    ui.set_model_headers(model(
        models.iter().map(crate::pickers::is_header).collect(),
    ));
    ui.set_model_index(crate::pickers::model_index(&models, "claude-opus-5-5"));
    let efforts = crate::pickers::effort_list("claude-opus-5-5");
    ui.set_efforts(model(
        efforts
            .iter()
            .map(|s| SharedString::from(s.as_str()))
            .collect(),
    ));
    ui.set_effort_index(2);
    ui.set_trace_entries(model(trace_entries()));
    ui.set_trace_steps(model(shots_trace::steps()));
    ui.set_trace_session("2026-11-11T22-51-40-3f9c".into());
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
    std::env::var_os("FAST_HARNESS_SHOTS_DIR")
        .map(PathBuf::from)
        .unwrap_or_else(|| std::env::temp_dir().join("fast-harness-gui-shots"))
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
        for shot in [
            "chat", "accounts", "google", "settings", "about", "trace", "steps",
        ] {
            let page = match shot {
                "steps" => "trace",
                "google" => "accounts",
                _ => shot,
            };
            // The Google shot keeps one Claude card so its section is in view.
            let mut claude = claude_rows();
            claude.truncate(if shot == "google" { 1 } else { 3 });
            ui.set_accounts(model(claude));
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
        // The composer's model dropdown: the Claude and Google Antigravity groups.
        click(&window, MODEL_X, EFFORT_Y);
        let png = shots_png::encode(WIDTH, HEIGHT, &render(&window));
        std::fs::write(dir.join(format!("{}-models.png", core.id)), png).expect("write png");
        key(&window, slint::platform::Key::Escape.into());
        ui.set_page("settings".into());
        // The theme dropdown opened by a click, then closed with Escape.
        click(&window, 601.0, PAGE_Y + 104.0);
        // A second open by keyboard is a no-op when the click already opened it.
        key(&window, slint::platform::Key::Return.into());
        let png = shots_png::encode(WIDTH, HEIGHT, &render(&window));
        std::fs::write(dir.join(format!("{}-dropdown.png", core.id)), png).expect("write png");
        key(&window, slint::platform::Key::Escape.into());
        ui.set_page("chat".into());
        ui.set_sidebar_open(false);
        let png = shots_png::encode(WIDTH, HEIGHT, &render(&window));
        std::fs::write(dir.join(format!("{}-nosidebar.png", core.id)), png).expect("write png");
        ui.set_sidebar_open(true);
        outside_click_closes(&ui, &window);
    }
}

/// A neutral spot in the transcript, outside every control.
const OUTSIDE: (f32, f32) = (800.0, 300.0);

/// The effort dropdown's chevron, inside the control (clear of the focus
/// ring and of the popup's shadow), as RGB bytes.
fn chevron(rgb: &[u8]) -> Vec<u8> {
    let (x0, x1, y0, y1) = (578, 602, 682, 699);
    (y0..y1)
        .flat_map(|y| {
            let row = (y * WIDTH as usize + x0) * 3;
            rgb[row..row + (x1 - x0) * 3].to_vec()
        })
        .collect()
}

/// A dropdown closed by a click outside its list shows the closed chevron
/// again, and `opened()` fires once per open.
fn outside_click_closes(ui: &MainWindow, window: &MinimalSoftwareWindow) {
    let opens = Rc::new(std::cell::Cell::new(0));
    let counter = opens.clone();
    ui.on_pickers_opened(move || counter.set(counter.get() + 1));
    click(window, OUTSIDE.0, OUTSIDE.1);
    let closed = chevron(&render(window));
    click(window, EFFORT_X, EFFORT_Y);
    let open = chevron(&render(window));
    click(window, OUTSIDE.0, OUTSIDE.1);
    let after = chevron(&render(window));
    assert!(closed != open, "the chevron shows the open list");
    assert!(
        closed == after,
        "a click outside closes the list and the chevron"
    );
    click(window, EFFORT_X, EFFORT_Y);
    click(window, OUTSIDE.0, OUTSIDE.1);
    assert_eq!(opens.get(), 2, "opened() fires once per open");
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
