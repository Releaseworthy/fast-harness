//! TUI keys of steering, queue, pause and resume (PLAN.md D39).

use ratatui::backend::TestBackend;
use ratatui::crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use ratatui::Terminal;

use fast_harness_core::view::Transcript;

use super::draw::{self, View};
use super::transcript::Cache;
use super::ui::{Action, Ui};

fn key(code: KeyCode, modifiers: KeyModifiers) -> KeyEvent {
    KeyEvent::new(code, modifiers)
}

fn typed(ui: &mut Ui, text: &str) {
    for c in text.chars() {
        ui.key(key(KeyCode::Char(c), KeyModifiers::NONE));
    }
}

fn running() -> Ui {
    let mut ui = Ui::new("claude-x".into(), None);
    ui.running = true;
    ui
}

#[test]
fn enter_while_busy_steers() {
    let mut ui = running();
    typed(&mut ui, "look at b.rs");
    let action = ui.key(key(KeyCode::Enter, KeyModifiers::NONE));
    assert_eq!(action, Action::Steer("look at b.rs".into()));
    assert!(ui.input.is_empty());
}

#[test]
fn alt_enter_queues_while_busy_and_ctrl_backspace_unqueues() {
    let mut ui = running();
    typed(&mut ui, "first");
    assert_eq!(ui.key(key(KeyCode::Enter, KeyModifiers::ALT)), Action::None);
    typed(&mut ui, "second");
    ui.key(key(KeyCode::Enter, KeyModifiers::ALT));
    assert_eq!(ui.queued, ["first", "second"]);
    ui.key(key(KeyCode::Backspace, KeyModifiers::CONTROL));
    assert_eq!(ui.queued, ["first"]);
    assert!(ui.status.contains("second"));
}

#[test]
fn alt_enter_is_a_newline_when_idle() {
    let mut ui = Ui::new("claude-x".into(), None);
    typed(&mut ui, "a");
    ui.key(key(KeyCode::Enter, KeyModifiers::ALT));
    assert!(ui.queued.is_empty());
    assert_eq!(ui.input.text(), "a\n");
}

#[test]
fn ctrl_g_pauses_a_running_turn() {
    let mut ui = running();
    let action = ui.key(key(KeyCode::Char('g'), KeyModifiers::CONTROL));
    assert_eq!(action, Action::Pause);
}

#[test]
fn slash_resume_resumes_when_idle() {
    let mut ui = Ui::new("claude-x".into(), None);
    typed(&mut ui, "/resume");
    assert_eq!(
        ui.key(key(KeyCode::Enter, KeyModifiers::NONE)),
        Action::ResumeTurn
    );
}

#[test]
fn queued_texts_show_above_the_input() {
    let mut ui = running();
    ui.steering.push("use the fast path".into());
    ui.queued.push("then write docs".into());
    let mut terminal = Terminal::new(TestBackend::new(80, 16)).unwrap();
    let (transcript, mut cache) = (Transcript::new(), Cache::default());
    let mut view = View {
        transcript: &transcript,
        cache: &mut cache,
        session: "s-1",
    };
    terminal
        .draw(|frame| draw::draw(frame, &mut ui, &mut view, 0))
        .unwrap();
    let buffer = terminal.backend().buffer().clone();
    let rows: Vec<String> = (0..16)
        .map(|y| {
            (0..80)
                .map(|x| buffer[(x, y)].symbol().to_string())
                .collect()
        })
        .collect();
    let at = |needle: &str| rows.iter().position(|r| r.contains(needle)).unwrap();
    let steer = at("steer (next step): use the fast path");
    let queued = at("queued 1: then write docs");
    assert_eq!(queued, steer + 1);
    assert!(rows[queued + 1].starts_with('╭') || rows[queued + 1].starts_with('┌'));
    assert!(rows[15].contains("Ctrl+G pause"));
}
