//! The Settings > System prompt section in the screenshot test: the
//! workspace scope with an override, an unsaved edit and the preview shown.

use std::path::Path;

use slint::platform::software_renderer::MinimalSoftwareWindow;
use slint::platform::WindowEvent;
use slint::{ComponentHandle, LogicalPosition};

use crate::{shots_png, MainWindow, PromptUi};

const TEMPLATE: &str = "You are a coding agent in Fast Harness, working in {cwd} on {os}. \
Today is {date} (UTC). The model is {model}.\nKeep answers short and cite file paths.\n";

fn fill(ui: &MainWindow) {
    let state = ui.global::<PromptUi>();
    state.set_workspace("fast-harness".into());
    state.set_scope(1);
    state.set_override_on(true);
    state.set_saved(TEMPLATE.into());
    state.set_text(format!("{TEMPLATE}Prefer small commits.").into());
    state.set_source(
        "Used in this workspace: workspace prompt C:\\Users\\me\\.fast-harness\\prompts\\F--work-fast-harness.md. Save writes C:\\Users\\me\\.fast-harness\\prompts\\F--work-fast-harness.md.".into(),
    );
    state.set_status(
        "Override on: edit this workspace's prompt, starting from the global one.".into(),
    );
    state.set_placeholders(crate::prompt_settings::PLACEHOLDERS.into());
    state.set_show_preview(true);
    let preview = "You are a coding agent in Fast Harness, working in F:\\work\\fast-harness on Windows. \
Today is 2026-11-12 (UTC). The model is claude-opus-5-5.\nKeep answers short and cite file paths.\n\
Prefer small commits.\n\n# Project instructions (AGENTS.md)\n\n## F:\\work\\fast-harness\\AGENTS.md\n\n\
Rules for anyone changing this repo. Read PLAN.md first.";
    state.set_preview(preview.into());
}

fn scroll(window: &MinimalSoftwareWindow, delta_y: f32) {
    let position = LogicalPosition::new(700.0, 400.0);
    let event = WindowEvent::PointerScrolled {
        position,
        delta_x: 0.0,
        delta_y,
    };
    window.dispatch_event(event);
}

/// `<theme>-prompt.png`: the section scrolled into view; scrolls back after.
pub fn shot(
    ui: &MainWindow,
    window: &MinimalSoftwareWindow,
    dir: &Path,
    theme: &str,
    size: (u32, u32),
    render: impl Fn() -> Vec<u8>,
) {
    fill(ui);
    ui.set_page("settings".into());
    render();
    scroll(window, -2000.0);
    let png = shots_png::encode(size.0, size.1, &render());
    std::fs::write(dir.join(format!("{theme}-prompt.png")), png).expect("write png");
    scroll(window, 2000.0);
    ui.set_page("chat".into());
}
