//! Rendering into ratatui's TestBackend and the key state machine, without
//! a real terminal, a runtime or a model.

use nh_core::llm::Timing;
use nh_core::types::{StopReason, Usage};
use nh_core::view::{Item, NoteKind, State, Transcript};
use ratatui::backend::TestBackend;
use ratatui::buffer::Buffer;
use ratatui::crossterm::event::{
    KeyCode, KeyEvent, KeyEventKind, KeyModifiers, MouseEvent, MouseEventKind,
};
use ratatui::Terminal;

use super::draw::{self, View};
use super::theme::color;
use super::transcript::Cache;
use super::ui::{Action, Dialog, Screen, Ui};

fn tool(output: Option<&str>, is_error: bool) -> Item {
    Item::Tool {
        id: "t1".into(),
        name: "read".into(),
        arguments: "{}".into(),
        summary: "read README.md".into(),
        output: output.map(String::from),
        is_error,
        duration_ms: output.map(|_| 5),
    }
}

fn sample() -> Transcript {
    let mut t = Transcript::new();
    t.items = vec![
        Item::User {
            text: "hello 世界".into(),
        },
        Item::Assistant {
            turn: 1,
            step: 1,
            text: "# Answer\n\n**bold** text".into(),
            thinking: "pondering deeply\nsecond line".into(),
        },
        tool(Some("line one\nline two\nline three\nline four"), false),
        tool(Some("boom"), true),
        Item::Note {
            kind: NoteKind::Error,
            text: "it failed".into(),
        },
        Item::StepFooter {
            turn: 1,
            step: 1,
            model: "claude-x".into(),
            account: "me@x.y".into(),
            usage: Usage {
                input: 1200,
                output: 30,
                cache_read: 7,
                cache_write: 9,
            },
            timing: Timing {
                ttfb_ms: 1500,
                total_ms: 4200,
                ..Timing::default()
            },
            stop: StopReason::End,
        },
    ];
    t.revision = 1;
    t.totals.input = 1200;
    t
}

fn render(ui: &mut Ui, transcript: &Transcript, width: u16, height: u16) -> Buffer {
    let mut terminal = Terminal::new(TestBackend::new(width, height)).unwrap();
    let mut cache = Cache::default();
    let mut view = View {
        transcript,
        cache: &mut cache,
        session: "s-1",
    };
    terminal
        .draw(|frame| draw::draw(frame, ui, &mut view, 0))
        .unwrap();
    terminal.backend().buffer().clone()
}

fn rows(buffer: &Buffer) -> Vec<String> {
    let area = buffer.area;
    (0..area.height)
        .map(|y| {
            (0..area.width)
                .map(|x| buffer[(x, y)].symbol().to_string())
                .collect::<String>()
        })
        .collect()
}

fn find(rows: &[String], needle: &str) -> usize {
    rows.iter()
        .position(|r| r.contains(needle))
        .unwrap_or_else(|| panic!("`{needle}` not on screen:\n{}", rows.join("\n")))
}

#[test]
fn renders_every_item_kind() {
    let mut ui = Ui::new("claude-x".into(), Some("high".into()));
    let buffer = render(&mut ui, &sample(), 70, 30);
    let screen = rows(&buffer);
    assert!(screen[0].contains("claude-x high") && screen[0].contains("s-1"));
    assert!(screen[0].contains("idle") && screen[0].contains("in 1.2k"));
    find(&screen, "› hello 世");
    find(&screen, "✻ Thinking: pondering deeply");
    assert!(!screen.iter().any(|r| r.contains("second line")));
    find(&screen, "Answer");
    find(&screen, "✓ read README.md");
    find(&screen, "✗ read README.md");
    find(&screen, "⎿ line one");
    find(&screen, "line three");
    find(&screen, "… +1 lines");
    let error = color(ui.theme.palette.error);
    let boom = find(&screen, "boom");
    let x = screen[boom].find("boom").unwrap_or(0);
    let col = screen[boom][..x].chars().count() as u16;
    assert_eq!(buffer[(col, boom as u16)].fg, error);
    let note = find(&screen, "✗ it failed");
    assert_eq!(buffer[(2, note as u16)].fg, error);
    find(
        &screen,
        "turn 1 step 1 · claude-x · me@x.y · ttfb 1.5s · 4.2s",
    );
    find(&screen, "Enter send");
}

#[test]
fn thinking_expands_and_spinner_runs() {
    let mut ui = Ui::new("m".into(), None);
    ui.show_thinking = true;
    ui.running = true;
    let mut t = sample();
    t.items.push(tool(None, false));
    t.state = State::RunningTools { running: 1 };
    let screen = rows(&render(&mut ui, &t, 70, 40));
    find(&screen, "second line");
    find(&screen, "running 1 tool(s)");
    find(&screen, "⠋ read README.md");
}

#[test]
fn follows_the_bottom_unless_scrolled_up() {
    let mut ui = Ui::new("m".into(), None);
    let mut t = Transcript::new();
    t.items = (0..50)
        .map(|i| Item::Note {
            kind: NoteKind::Info,
            text: format!("note {i}"),
        })
        .collect();
    let screen = rows(&render(&mut ui, &t, 40, 12));
    find(&screen, "note 49");
    ui.mouse(MouseEvent {
        kind: MouseEventKind::ScrollUp,
        column: 0,
        row: 0,
        modifiers: KeyModifiers::NONE,
    });
    t.items.push(Item::Note {
        kind: NoteKind::Info,
        text: "note 50".into(),
    });
    let screen = rows(&render(&mut ui, &t, 40, 12));
    assert!(!screen.iter().any(|r| r.contains("note 50")));
    find(&screen, "more (End)");
    press(&mut ui, KeyCode::End, KeyModifiers::NONE);
    let screen = rows(&render(&mut ui, &t, 40, 12));
    find(&screen, "note 50");
}

fn press(ui: &mut Ui, code: KeyCode, modifiers: KeyModifiers) -> Action {
    ui.key(KeyEvent::new(code, modifiers))
}

fn typed(ui: &mut Ui, text: &str) {
    for c in text.chars() {
        press(ui, KeyCode::Char(c), KeyModifiers::NONE);
    }
}

#[test]
fn keys_send_newline_cancel_and_quit() {
    let mut ui = Ui::new("m".into(), None);
    typed(&mut ui, "hi");
    press(&mut ui, KeyCode::Enter, KeyModifiers::ALT);
    press(&mut ui, KeyCode::Char('j'), KeyModifiers::CONTROL);
    typed(&mut ui, "there");
    let release = KeyEvent::new_with_kind(
        KeyCode::Char('x'),
        KeyModifiers::NONE,
        KeyEventKind::Release,
    );
    assert_eq!(ui.key(release), Action::None);
    let sent = press(&mut ui, KeyCode::Enter, KeyModifiers::NONE);
    assert_eq!(sent, Action::Send("hi\n\nthere".into()));
    assert!(ui.input.is_empty());
    press(&mut ui, KeyCode::Up, KeyModifiers::NONE);
    assert_eq!(ui.input.text(), "hi\n\nthere");
    ui.input.clear();
    ui.running = true;
    assert_eq!(
        press(&mut ui, KeyCode::Esc, KeyModifiers::NONE),
        Action::Cancel
    );
    let ctrl_c = |ui: &mut Ui| press(ui, KeyCode::Char('c'), KeyModifiers::CONTROL);
    assert_eq!(ctrl_c(&mut ui), Action::Cancel);
    typed(&mut ui, "x");
    assert_eq!(
        press(&mut ui, KeyCode::Enter, KeyModifiers::NONE),
        Action::None
    );
    ui.running = false;
    assert_eq!(ctrl_c(&mut ui), Action::None);
    assert!(ui.input.is_empty());
    assert_eq!(ctrl_c(&mut ui), Action::Quit);
    assert_eq!(ctrl_c(&mut ui), Action::Quit);
    typed(&mut ui, "a");
    assert_eq!(ctrl_c(&mut ui), Action::None);
    let ctrl_d = press(&mut ui, KeyCode::Char('d'), KeyModifiers::CONTROL);
    assert_eq!(ctrl_d, Action::Quit);
}

#[test]
fn screens_open_and_dispatch() {
    let mut ui = Ui::new("m".into(), Some("high".into()));
    let open = press(&mut ui, KeyCode::F(2), KeyModifiers::NONE);
    assert_eq!(open, Action::Open(Screen::Accounts));
    ui.screen = Screen::Accounts;
    assert_eq!(
        press(&mut ui, KeyCode::Char('l'), KeyModifiers::NONE),
        Action::Login
    );
    assert_eq!(
        press(&mut ui, KeyCode::Char('r'), KeyModifiers::NONE),
        Action::PollUsage
    );
    assert_eq!(
        press(&mut ui, KeyCode::Char('i'), KeyModifiers::NONE),
        Action::Import
    );
    press(&mut ui, KeyCode::Char('k'), KeyModifiers::NONE);
    typed(&mut ui, "sk-abc");
    let screen = rows(&render(&mut ui, &Transcript::new(), 80, 20));
    find(&screen, "key: ••••••");
    assert!(!screen.iter().any(|r| r.contains("sk-abc")));
    let added = press(&mut ui, KeyCode::Enter, KeyModifiers::NONE);
    assert_eq!(added, Action::AddKey("sk-abc".into()));
    ui.dialog = Dialog::Login("https://claude.ai/x".into());
    assert_eq!(
        press(&mut ui, KeyCode::Esc, KeyModifiers::NONE),
        Action::CancelLogin
    );
    ui.dialog = Dialog::None;
    assert_eq!(
        press(&mut ui, KeyCode::Esc, KeyModifiers::NONE),
        Action::None
    );
    assert_eq!(ui.screen, Screen::Chat);
    ui.screen = Screen::Models;
    let effort = press(&mut ui, KeyCode::Char('e'), KeyModifiers::NONE);
    assert_eq!(effort, Action::SetEffort("xhigh".into()));
    ui.running = true;
    ui.screen = Screen::Chat;
    let sessions = press(&mut ui, KeyCode::Char('o'), KeyModifiers::CONTROL);
    assert_eq!(sessions, Action::None);
    let trace = press(&mut ui, KeyCode::Char('r'), KeyModifiers::CONTROL);
    assert_eq!(trace, Action::Open(Screen::Trace));
}

#[test]
fn pasted_enter_becomes_a_newline() {
    use super::app::Msg;
    use ratatui::crossterm::event::Event;
    let key = |code, kind| {
        Msg::Term(Event::Key(KeyEvent::new_with_kind(
            code,
            KeyModifiers::NONE,
            kind,
        )))
    };
    let mut batch = vec![
        key(KeyCode::Char('a'), KeyEventKind::Press),
        key(KeyCode::Enter, KeyEventKind::Press),
        key(KeyCode::Enter, KeyEventKind::Release),
        key(KeyCode::Char('b'), KeyEventKind::Press),
        key(KeyCode::Enter, KeyEventKind::Press),
        key(KeyCode::Enter, KeyEventKind::Release),
    ];
    super::paste_newlines(&mut batch);
    let mods: Vec<KeyModifiers> = batch
        .iter()
        .filter_map(|m| match m {
            Msg::Term(Event::Key(k))
                if k.code == KeyCode::Enter && k.kind == KeyEventKind::Press =>
            {
                Some(k.modifiers)
            }
            _ => None,
        })
        .collect();
    assert_eq!(mods, [KeyModifiers::ALT, KeyModifiers::NONE]);
}
