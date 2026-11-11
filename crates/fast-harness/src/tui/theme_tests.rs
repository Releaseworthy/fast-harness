//! Every theme on full frames: the greyscale themes paint only greys, every
//! glyph meets WCAG 2.2 contrast, F5 cycles the theme and saves it.

use std::fs;

use fast_harness_core::llm::anthropic::builtin_models;
use fast_harness_core::theme::{contrast_ratio, Rgb, Theme, THEMES};
use fast_harness_core::view::{Item, NoteKind, State, Transcript};
use ratatui::backend::TestBackend;
use ratatui::buffer::Buffer;
use ratatui::crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use ratatui::style::{Color, Modifier};
use ratatui::Terminal;

use super::draw::{self, View};
use super::theme::{self, color, Styles};
use super::transcript::{Cache, Opts};
use super::ui::{Action, Screen, Ui};

/// Wide enough for the whole hint bar.
const WIDTH: u16 = 160;

fn tool(summary: &str, output: Option<&str>, is_error: bool) -> Item {
    Item::Tool {
        id: summary.into(),
        name: "bash".into(),
        arguments: "{}".into(),
        summary: summary.into(),
        output: output.map(String::from),
        is_error,
        duration_ms: output.map(|_| 5),
    }
}

fn note(kind: NoteKind, text: &str) -> Item {
    Item::Note {
        kind,
        text: text.into(),
    }
}

fn transcript() -> Transcript {
    let mut t = Transcript::new();
    t.items = vec![
        Item::User {
            text: "show me the build".into(),
        },
        Item::Assistant {
            turn: 1,
            step: 1,
            text: "# Build\n\n## Steps\n\nRun `cargo test` and read [the docs](https://x.y).\n\n```\nlet x = 1;\n```\n\n- first\n1. one".into(),
            thinking: "weighing the options".into(),
        },
        tool("cargo build", None, false),
        tool("cargo test", Some("ok: 12 passed"), false),
        tool("cargo fmt", Some("error: bad file"), true),
        note(NoteKind::Retry, "retrying in 2s"),
        note(NoteKind::AccountSwitch, "switched to b@x.y"),
        note(NoteKind::Error, "the turn failed"),
        note(NoteKind::Info, "session resumed"),
    ];
    t.state = State::RunningTools { running: 1 };
    t.revision = 1;
    t
}

fn frame(theme: &'static Theme, setup: impl FnOnce(&mut Ui)) -> Buffer {
    let mut ui = Ui::new("claude-x".into(), Some("high".into()));
    ui.theme = theme;
    setup(&mut ui);
    let transcript = transcript();
    let mut terminal = Terminal::new(TestBackend::new(WIDTH, 40)).unwrap();
    let mut cache = Cache::default();
    let mut view = View {
        transcript: &transcript,
        cache: &mut cache,
        session: "s-1",
    };
    terminal
        .draw(|frame| draw::draw(frame, &mut ui, &mut view, 0))
        .unwrap();
    terminal.backend().buffer().clone()
}

/// The frames every check runs on: chat idle, chat running with a status
/// message, the model list with a selected row, the trace timeline and
/// the open model picker.
fn frames(theme: &'static Theme) -> Vec<Buffer> {
    vec![
        frame(theme, |_| {}),
        frame(theme, |ui| {
            ui.running = true;
            ui.status = "theme Dark (saved)".into();
        }),
        frame(theme, |ui| {
            ui.screen = Screen::Models;
            ui.models = builtin_models();
            ui.selected = 1;
        }),
        frame(theme, |ui| {
            ui.screen = Screen::Trace;
            let steps = vec!["turn step model".into()];
            ui.trace
                .set(super::picker_tests::sample_entries(), steps, None);
        }),
        frame(theme, |ui| ui.open_picker(super::picker::PickerKind::Model)),
    ]
}

fn rgb(c: Color) -> Rgb {
    match c {
        Color::Rgb(r, g, b) => Rgb(r, g, b),
        other => panic!("not a theme colour: {other:?}"),
    }
}

fn text(buffer: &Buffer) -> String {
    buffer.content().iter().map(|c| c.symbol()).collect()
}

#[test]
fn greyscale_themes_paint_only_greys() {
    let greys: Vec<&'static Theme> = THEMES
        .iter()
        .filter(|t| t.id.starts_with("greyscale"))
        .collect();
    assert_eq!(greys.len(), 2);
    for theme in greys {
        for buffer in frames(theme) {
            for (i, cell) in buffer.content().iter().enumerate() {
                let (fg, bg) = (rgb(cell.fg), rgb(cell.bg));
                assert!(
                    fg.is_grey() && bg.is_grey(),
                    "{} cell {i} `{}`: {fg:?} on {bg:?}",
                    theme.id,
                    cell.symbol()
                );
            }
        }
    }
}

/// Text needs 4.5:1 (SC 1.4.3); box-drawing borders are graphics and need
/// 3:1 (SC 1.4.11).
#[test]
fn every_glyph_meets_wcag_contrast_in_every_theme() {
    for theme in &THEMES {
        for buffer in frames(theme) {
            for (i, cell) in buffer.content().iter().enumerate() {
                let symbol = cell.symbol();
                let Some(first) = symbol.chars().next().filter(|c| !c.is_whitespace()) else {
                    continue;
                };
                let border = ('\u{2500}'..='\u{257F}').contains(&first);
                let need = if border { 3.0 } else { 4.5 };
                let ratio = contrast_ratio(rgb(cell.fg), rgb(cell.bg));
                let (x, y) = (i % usize::from(WIDTH), i / usize::from(WIDTH));
                assert!(
                    ratio >= need,
                    "{} ({x},{y}) `{symbol}`: {ratio:.2} < {need}",
                    theme.id
                );
            }
        }
    }
}

#[test]
fn states_carry_marks_not_only_colour() {
    let idle = text(&frame(&THEMES[2], |_| {}));
    for mark in ["○ cargo build", "✓ cargo test", "✗ cargo fmt", "↻ retrying"] {
        assert!(idle.contains(mark), "{mark} missing");
    }
    assert!(idle.contains("⇄ switched") && idle.contains("✗ the turn failed"));
    assert!(idle.contains("F5 theme"));
    let running = text(&frame(&THEMES[2], |ui| ui.running = true));
    assert!(running.contains("⠋ cargo build"));
    let models = frame(&THEMES[0], |ui| {
        ui.screen = Screen::Models;
        ui.models = builtin_models();
        ui.selected = 0;
    });
    let selected = Styles::new(&THEMES[0]).palette().selection;
    let row = (0..models.area.height)
        .find(|&y| models[(1, y)].symbol() == "›")
        .expect("a › marker on the selected row");
    let cell = &models[(3, row)];
    assert_eq!(cell.bg, color(selected));
    assert!(!cell.modifier.contains(Modifier::REVERSED));
}

#[test]
fn a_theme_change_re_renders_cached_lines() {
    let items = transcript().items;
    let opts = |theme: &'static Theme| Opts {
        width: 60,
        show_thinking: false,
        running: false,
        frame: 0,
        styles: Styles::new(theme),
    };
    let mut cache = Cache::default();
    let dark = cache.lines(&items, opts(&THEMES[0]))[0].spans[0].style.fg;
    let light = cache.lines(&items, opts(&THEMES[1]))[0].spans[0].style.fg;
    assert_eq!(dark, Some(color(THEMES[0].palette.accent)));
    assert_eq!(light, Some(color(THEMES[1].palette.accent)));
}

#[test]
fn f5_cycles_the_theme_and_saves_it() {
    let home = std::env::temp_dir().join(format!("fast-harness-tui-theme-{}", std::process::id()));
    let _ = fs::remove_dir_all(&home);
    fs::create_dir_all(&home).unwrap();
    fs::write(home.join("config.json"), r#"{"model":"m"}"#).unwrap();
    let mut ui = Ui::new("m".into(), None);
    assert_eq!(ui.theme.id, "dark");
    let f5 = ui.key(KeyEvent::new(KeyCode::F(5), KeyModifiers::NONE));
    assert_eq!(f5, Action::CycleTheme);
    let saved = |home: &std::path::Path| -> serde_json::Value {
        serde_json::from_slice(&fs::read(home.join("config.json")).unwrap()).unwrap()
    };
    assert_eq!(theme::cycle(&mut ui, &home).id, "light");
    assert_eq!(ui.status, "theme Light (saved)");
    assert_eq!(theme::cycle(&mut ui, &home).id, "greyscale-dark");
    assert_eq!(ui.status, "theme Greyscale dark (saved)");
    let config = saved(&home);
    assert_eq!(config["theme"], "greyscale-dark");
    assert_eq!(config["model"], "m");
    theme::cycle(&mut ui, &home);
    assert_eq!(theme::cycle(&mut ui, &home).id, "dark");
    // A home that cannot hold config.json: the theme still applies.
    let blocked = home.join("config.json");
    assert_eq!(theme::cycle(&mut ui, &blocked).id, "light");
    assert!(
        ui.status.starts_with("theme Light (not saved"),
        "{}",
        ui.status
    );
    fs::remove_dir_all(&home).unwrap();
}
