//! Every theme on full frames: greyscale 100 paints only greys, every glyph
//! meets WCAG 2.2 contrast at every level, F5 cycles the built-in and user
//! themes, F6 steps the greyscale level, both saved.

use std::fs;

use fast_harness_core::llm::anthropic::builtin_models;
use fast_harness_core::theme::{builtins, contrast_ratio, Rgb, Theme, ThemeSet};
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
        live: None,
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
            images: vec![],
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

fn frame(theme: &Theme, level: u8, setup: impl FnOnce(&mut Ui)) -> Buffer {
    let mut ui = Ui::new("claude-x".into(), Some("high".into()));
    ui.set_greyscale(level);
    ui.set_theme(theme.clone());
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
fn frames(theme: &Theme, level: u8) -> Vec<Buffer> {
    vec![
        frame(theme, level, |_| {}),
        frame(theme, level, |ui| {
            ui.running = true;
            ui.status = "theme Dark (saved)".into();
        }),
        frame(theme, level, |ui| {
            ui.screen = Screen::Models;
            ui.models = builtin_models();
            ui.selected = 1;
        }),
        frame(theme, level, |ui| {
            ui.screen = Screen::Trace;
            let steps = vec!["turn step model".into()];
            ui.trace
                .set(super::picker_tests::sample_entries(), steps, None);
        }),
        frame(theme, level, |ui| {
            ui.open_picker(super::picker::PickerKind::Model)
        }),
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
fn greyscale_100_paints_only_greys() {
    for theme in builtins() {
        for buffer in frames(theme, 100) {
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
    for theme in builtins() {
        for level in [0, 50, 100] {
            for buffer in frames(theme, level) {
                assert_contrast(&theme.id, level, &buffer);
            }
        }
    }
}

fn assert_contrast(id: &str, level: u8, buffer: &Buffer) {
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
            "{id} grey {level} ({x},{y}) `{symbol}`: {ratio:.2} < {need}"
        );
    }
}

#[test]
fn states_carry_marks_not_only_colour() {
    let dark = &builtins()[0];
    let idle = text(&frame(dark, 100, |_| {}));
    for mark in ["○ cargo build", "✓ cargo test", "✗ cargo fmt", "↻ retrying"] {
        assert!(idle.contains(mark), "{mark} missing");
    }
    assert!(idle.contains("⇄ switched") && idle.contains("✗ the turn failed"));
    assert!(idle.contains("F5 theme · F6 grey"));
    let running = text(&frame(dark, 100, |ui| ui.running = true));
    assert!(running.contains("⠋ cargo build"));
    let models = frame(dark, 0, |ui| {
        ui.screen = Screen::Models;
        ui.models = builtin_models();
        ui.selected = 0;
    });
    let selected = Styles::new(dark).palette().selection;
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
    let opts = |theme: &Theme| Opts {
        width: 60,
        show_thinking: false,
        running: false,
        frame: 0,
        styles: Styles::new(theme),
    };
    let mut cache = Cache::default();
    let (dark, light) = (&builtins()[0], &builtins()[1]);
    let fg = |cache: &mut Cache, theme| cache.lines(&items, opts(theme))[0].spans[0].style.fg;
    assert_eq!(fg(&mut cache, dark), Some(color(dark.palette.accent)));
    assert_eq!(fg(&mut cache, light), Some(color(light.palette.accent)));
}

struct Home(std::path::PathBuf);

impl Home {
    fn new(name: &str) -> Home {
        let home =
            std::env::temp_dir().join(format!("fast-harness-tui-{name}-{}", std::process::id()));
        let _ = fs::remove_dir_all(&home);
        fs::create_dir_all(home.join("themes")).unwrap();
        fs::write(home.join("config.json"), r#"{"model":"m"}"#).unwrap();
        Home(home)
    }

    fn saved(&self) -> serde_json::Value {
        serde_json::from_slice(&fs::read(self.0.join("config.json")).unwrap()).unwrap()
    }
}

impl Drop for Home {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

#[test]
fn f5_cycles_built_in_and_user_themes_and_saves_them() {
    let home = Home::new("theme");
    let mine =
        r#"{"version":1,"id":"mine","name":"Mine","kind":"dark","extends":"aurora","colors":{}}"#;
    fs::write(home.0.join("themes").join("mine.json"), mine).unwrap();
    let mut ui = Ui::new("m".into(), None);
    ui.themes = ThemeSet::load(&home.0);
    assert_eq!(ui.theme.id, "dark");
    let f5 = ui.key(KeyEvent::new(KeyCode::F(5), KeyModifiers::NONE));
    assert_eq!(f5, Action::CycleTheme);
    assert_eq!(theme::cycle(&mut ui, &home.0).id, "light");
    assert_eq!(ui.status, "theme Light (saved)");
    assert_eq!(theme::cycle(&mut ui, &home.0).id, "aurora");
    assert_eq!(ui.status, "theme Aurora (saved)");
    assert_eq!(ui.palette, builtins()[2].palette);
    let config = home.saved();
    assert_eq!(
        (config["theme"].as_str(), config["model"].as_str()),
        (Some("aurora"), Some("m"))
    );
    let ids: Vec<String> = (0..6).map(|_| theme::cycle(&mut ui, &home.0).id).collect();
    assert_eq!(
        ids,
        ["synthwave", "sunset", "emerald", "sakura", "citrus", "mine"]
    );
    assert_eq!(ui.status, "theme Mine (saved)");
    assert_eq!(theme::cycle(&mut ui, &home.0).id, "dark");
    // A home that cannot hold config.json: the theme still applies.
    let blocked = home.0.join("config.json");
    assert_eq!(theme::cycle(&mut ui, &blocked).id, "light");
    assert!(
        ui.status.starts_with("theme Light (not saved"),
        "{}",
        ui.status
    );
}

#[test]
fn f6_steps_the_greyscale_level_and_saves_it() {
    let home = Home::new("grey");
    let mut ui = Ui::new("m".into(), None);
    let f6 = ui.key(KeyEvent::new(KeyCode::F(6), KeyModifiers::NONE));
    assert_eq!(f6, Action::StepGreyscale(10));
    let back = ui.key(KeyEvent::new(KeyCode::F(6), KeyModifiers::SHIFT));
    assert_eq!(back, Action::StepGreyscale(-10));
    assert_eq!(theme::step_greyscale(&mut ui, &home.0, 10), 10);
    assert_eq!(ui.status, "greyscale 10 of 100 (saved)");
    assert_eq!(home.saved()["greyscale"], 10);
    assert_eq!(ui.palette, ui.theme.shown(10));
    assert_eq!(theme::step_greyscale(&mut ui, &home.0, -10), 0);
    assert_eq!(ui.status, "greyscale off, full colour (saved)");
    assert_eq!(theme::step_greyscale(&mut ui, &home.0, -10), 100);
    assert_eq!(ui.status, "greyscale 100 of 100, pure greys (saved)");
    assert!(ui.palette.accent.is_grey());
    assert_eq!(theme::step_greyscale(&mut ui, &home.0, 10), 0);
    // The level survives a theme change.
    ui.set_greyscale(40);
    theme::cycle(&mut ui, &home.0);
    assert_eq!((ui.theme.id.as_str(), ui.greyscale), ("light", 40));
    assert_eq!(ui.palette, builtins()[1].shown(40));
}
