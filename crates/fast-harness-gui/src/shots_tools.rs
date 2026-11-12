//! Sample tool cards for the screenshots, built through the real mapping
//! (`rows::msg_row`) from realistic arguments and the outputs the core
//! tools print: "tools-a" (read, write, edit), "tools-b" (grep, glob, a
//! running grep), "tools-c" (shell, a failing shell, subagent, find).

use std::path::Path;

use fast_harness_core::view::Item;
use slint::platform::software_renderer::MinimalSoftwareWindow;
use slint::{ModelRc, PhysicalSize, Rgb8Pixel, VecModel};

use crate::rows::msg_row;
use crate::{shots_png, MainWindow, MsgRow};

pub fn tool(name: &str, args: serde_json::Value, output: Option<&str>, error: bool) -> MsgRow {
    msg_row(&Item::Tool {
        id: "t".into(),
        name: name.into(),
        arguments: args.to_string(),
        summary: String::new(),
        output: output.map(str::to_string),
        is_error: error,
        duration_ms: output.map(|_| 41),
    })
}

const THEME_RS: &str = "crates/fast-harness-core/src/theme.rs";

pub fn read() -> MsgRow {
    let out = "40: /// The colour roles every view draws with.\n41: pub struct Theme {\n42:     pub id: String,\n43:     pub name: String,\n44:     pub dark: bool,\n45:     pub roles: Roles,\n[showing lines 40-45 of 184; read on with offset 46]";
    let args = serde_json::json!({ "path": THEME_RS, "offset": 40, "limit": 6 });
    tool("read", args, Some(out), false)
}

fn write() -> MsgRow {
    let content = "# Themes\n\nEvery theme is a JSON file with one colour per role.\nCopy a built-in theme, change the colours, and pick it in Settings.\nThe contrast check runs when the theme loads.";
    let args = serde_json::json!({ "path": "docs/themes.md", "content": content });
    tool(
        "write",
        args,
        Some("Wrote 205 bytes to docs/themes.md (new file)"),
        false,
    )
}

fn edit() -> MsgRow {
    let args = serde_json::json!({
        "path": THEME_RS,
        "old_string": "    pub dark: bool,\n    pub roles: Roles,\n}",
        "new_string": "    pub dark: bool,\n    /// Greyscale level, 0 to 100.\n    pub grey: u8,\n    pub roles: Roles,\n}",
    });
    let out = format!("Replaced 1 occurrence in {THEME_RS} at line 44");
    tool("edit", args, Some(&out), false)
}

pub fn edit_error() -> MsgRow {
    let args = serde_json::json!({ "path": THEME_RS, "old_string": "pub grey: u16,", "new_string": "pub grey: u8," });
    tool(
        "edit",
        args,
        Some("old_string not found in crates/fast-harness-core/src/theme.rs"),
        true,
    )
}

fn grep() -> MsgRow {
    let out = "crates/fast-harness-gui/src/theme.rs:12: pub fn apply(ui: &MainWindow, theme: &Theme, grey: u8) {\ncrates/fast-harness-gui/src/theme.rs:48:     ui.global::<Palette>().set_bg(color(theme.bg));\ncrates/fast-harness-gui/ui/parts.slint:4: export global Theme {\ncrates/fast-harness/src/tui/theme.rs:9: pub fn styles(theme: &Theme) -> Styles {";
    let args = serde_json::json!({ "pattern": "Theme\\b", "path": "crates", "glob": "*.{rs,slint}", "ignore_case": false });
    tool("grep", args, Some(out), false)
}

fn glob() -> MsgRow {
    let files = [
        "aurora",
        "citrus",
        "dark",
        "emerald",
        "light",
        "sakura",
        "sunset",
        "synthwave",
    ];
    let mut out: Vec<String> = files
        .iter()
        .map(|f| format!("crates/fast-harness-core/src/theme/builtin/{f}.json"))
        .collect();
    out.extend((1..=6).map(|i| format!("docs/themes/sample-{i}.json")));
    tool(
        "glob",
        serde_json::json!({ "pattern": "**/*.json", "path": "." }),
        Some(&out.join("\n")),
        false,
    )
}

pub fn running_grep() -> MsgRow {
    let args = serde_json::json!({ "pattern": "fn render", "path": "crates/fast-harness-gui" });
    tool("grep", args, None, false)
}

fn shell() -> MsgRow {
    let out = "   Compiling fast-harness-gui v0.1.0\n    Finished `test` profile in 14.2s\n     Running unittests src/main.rs\ntest result: ok. 68 passed; 0 failed; 1 ignored\n[exit code: 0]";
    let args =
        serde_json::json!({ "command": "cargo test -p fast-harness-gui", "timeout_ms": 600000 });
    tool("shell", args, Some(out), false)
}

pub fn shell_failed() -> MsgRow {
    let out = "error[E0425]: cannot find value `palette` in this scope\n --> crates/fast-harness-gui/src/theme.rs:12:5\n   |\n12 |     palette.apply(ui);\n   |     ^^^^^^^ not found in this scope\n[exit code: 101]";
    let args = serde_json::json!({ "command": "cargo build -p fast-harness-gui\ncargo clippy -p fast-harness-gui" });
    tool("shell", args, Some(out), true)
}

fn subagent() -> MsgRow {
    let out = "[subagent session 3f9c · model gemini-3-flash · 12 steps · 9 tool calls · 48.2k tokens]\nThe trace page reads trace::report only. The detail module already maps the user message, the thinking and every tool block; the page needs a row per entry.";
    let args = serde_json::json!({
        "description": "Map the trace detail entries",
        "prompt": "Read crates/fast-harness-core/src/trace/detail.rs and the Trace page.\nList what each entry kind carries.\nSay which fields the page does not show yet.\nKeep the answer under 200 words.",
    });
    tool("subagent", args, Some(out), false)
}

pub fn find() -> MsgRow {
    let args = serde_json::json!({ "query": "edit files" });
    tool(
        "find_tools",
        args,
        Some("Loaded: read, write, edit\nThe tools are ready for the next step."),
        false,
    )
}

/// The window height of the tool shots: tall enough for every card.
const TALL: u32 = 1400;

fn render(window: &MinimalSoftwareWindow, width: u32) -> Vec<u8> {
    let mut buffer = vec![Rgb8Pixel { r: 0, g: 0, b: 0 }; (width * TALL) as usize];
    // Three passes: the list view measures its rows on the first ones.
    for _ in 0..3 {
        slint::platform::update_timers_and_animations();
        window.request_redraw();
        window.draw_if_needed(|renderer| {
            renderer.render(&mut buffer, width as usize);
        });
    }
    buffer.iter().flat_map(|p| [p.r, p.g, p.b]).collect()
}

/// The three tool shots of one theme, in a window `TALL` pixels high; the
/// window gets `size` back after.
pub fn shot(
    ui: &MainWindow,
    window: &MinimalSoftwareWindow,
    dir: &Path,
    theme: &str,
    size: (u32, u32),
) {
    let sets = [
        ("tools-a", vec![read(), write(), edit()]),
        ("tools-b", vec![grep(), glob(), running_grep()]),
        ("tools-c", vec![shell(), shell_failed(), subagent(), find()]),
    ];
    ui.set_page("chat".into());
    window.set_size(PhysicalSize::new(size.0, TALL));
    for (name, rows) in sets {
        ui.set_messages(ModelRc::new(VecModel::from(rows)));
        let png = shots_png::encode(size.0, TALL, &render(window, size.0));
        std::fs::write(dir.join(format!("{theme}-{name}.png")), png).expect("write png");
    }
    window.set_size(PhysicalSize::new(size.0, size.1));
}
