//! The header and the Chat | Trace tab line in both icon sets.

use fast_harness_core::view::Transcript;
use ratatui::backend::TestBackend;
use ratatui::buffer::Buffer;
use ratatui::crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
use ratatui::style::Modifier;
use ratatui::Terminal;

use super::draw::{self, View};
use super::icons::{Icons, NERD, PLAIN};
use super::transcript::Cache;
use super::ui::{Action, Screen, Ui};

fn buffer(ui: &mut Ui) -> Buffer {
    let transcript = Transcript::new();
    let mut cache = Cache::default();
    let mut view = View {
        transcript: &transcript,
        cache: &mut cache,
        session: "s-1",
    };
    let mut terminal = Terminal::new(TestBackend::new(140, 20)).unwrap();
    terminal
        .draw(|frame| draw::draw(frame, ui, &mut view, 0))
        .unwrap();
    terminal.backend().buffer().clone()
}

fn row(buf: &Buffer, y: u16) -> String {
    (0..buf.area.width).map(|x| buf[(x, y)].symbol()).collect()
}

/// The modifiers of the first cell of `word` on row `y`.
fn marked(buf: &Buffer, y: u16, word: &str) -> Modifier {
    let text = row(buf, y);
    let byte = text
        .find(word)
        .unwrap_or_else(|| panic!("{word} in {text:?}"));
    let x = text[..byte].chars().count() as u16;
    buf[(x, y)].modifier
}

fn ui(icons: &'static Icons) -> Ui {
    let mut ui = Ui::new("claude-x".into(), Some("high".into()));
    ui.icons = icons;
    ui
}

fn check_tabs(icons: &'static Icons) {
    let mut ui = ui(icons);
    let buf = buffer(&mut ui);
    let head = row(&buf, 0);
    let brand = format!(
        "{}Fast Harness │ claude-x high",
        super::icons::lead(icons.app)
    );
    assert!(head.starts_with(&brand), "{head:?}");
    let tabs = row(&buf, 1);
    let chat = format!(" {}Chat ", super::icons::lead(icons.chat));
    let trace = format!(" {}Trace ", super::icons::lead(icons.trace));
    assert!(tabs.contains(&format!("{chat}|{trace}")), "{tabs:?}");
    let on = Modifier::BOLD | Modifier::REVERSED;
    assert!(marked(&buf, 1, "Chat").contains(on));
    assert!(!marked(&buf, 1, "Trace").contains(Modifier::REVERSED));
    // F3 opens the trace tab, F3 again (or Esc) goes back to chat.
    let f3 = KeyEvent::new(KeyCode::F(3), KeyModifiers::NONE);
    assert_eq!(ui.key(f3), Action::Open(Screen::Trace));
    ui.screen = Screen::Trace;
    let buf = buffer(&mut ui);
    assert!(marked(&buf, 1, "Trace").contains(on));
    assert!(!marked(&buf, 1, "Chat").contains(Modifier::REVERSED));
    assert_eq!(ui.key(f3), Action::Open(Screen::Chat));
    let esc = ui.key(KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE));
    assert_eq!((esc, ui.screen), (Action::None, Screen::Chat));
}

#[test]
fn the_tab_line_marks_the_current_tab_in_nerd_mode() {
    check_tabs(&NERD);
    let mut ui = ui(&NERD);
    let buf = buffer(&mut ui);
    assert!(row(&buf, 0).starts_with("\u{F140B} Fast Harness"));
    assert!(row(&buf, 1).contains("\u{EAC7} Chat") && row(&buf, 1).contains("\u{EB31} Trace"));
}

#[test]
fn the_tab_line_marks_the_current_tab_in_plain_mode() {
    check_tabs(&PLAIN);
    let mut ui = ui(&PLAIN);
    let buf = buffer(&mut ui);
    assert!(row(&buf, 0).starts_with("Fast Harness │"));
    assert!(row(&buf, 1).starts_with(" Chat | Trace "));
}
