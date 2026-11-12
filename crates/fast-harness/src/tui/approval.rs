//! The approval prompt (PLAN.md D64): the oldest request of the shown
//! session above the input box, answered with the `approval` keys of
//! `keys.json` (Alt+Y, Alt+A, Alt+N by default). Enter and Space keep
//! typing in the input box: no key answers by default.

use fast_harness_core::agent::Answer;
use fast_harness_core::events::ApprovalRequest;
use fast_harness_core::tools::Effect;
use fast_harness_core::view::approvals::prompt_text;
use ratatui::crossterm::event::KeyEvent;
use ratatui::layout::Rect;
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, Paragraph};
use ratatui::Frame;

use super::icons::lead;
use super::theme::Styles;
use super::ui::{Action, Screen, Ui};
use super::wrap::{sanitize, wrap_line};

/// The answer of a keymap action of the `approval` scope.
pub fn answer_of(name: &str) -> Option<Answer> {
    match name {
        "approve_once" => Some(Answer::Once),
        "approve_session" => Some(Answer::Session),
        "decline" => Some(Answer::No(None)),
        _ => None,
    }
}

/// The prompt shows: a request waits and the chat is on screen.
pub fn shown(ui: &Ui) -> bool {
    !ui.approvals.is_empty() && ui.screen == Screen::Chat && ui.panel.is_none()
}

/// The answer a key gives while the prompt shows; `None` for other keys.
pub fn key(ui: &Ui, key: &KeyEvent) -> Option<Action> {
    if !shown(ui) {
        return None;
    }
    let answer = answer_of(ui.action_of("approval", key)?)?;
    let id = ui.approvals.first()?.id.clone();
    Some(Action::Answer(id, answer))
}

/// What the call will do, in words.
fn what(request: &ApprovalRequest) -> String {
    let verb = match request.effect {
        Effect::Write => "change files",
        _ => "run a command",
    };
    format!("Approval needed: {} wants to {verb}", request.tool)
}

/// The summary wrapped in full; past `max` rows the start and the end
/// stay (the end of a command is often what matters) around a count.
fn summary_rows(text: &str, width: usize, max: usize, st: &Styles) -> Vec<Line<'static>> {
    let rows: Vec<Line<'static>> = (sanitize(text).split('\n'))
        .flat_map(|row| wrap_line(&Line::styled(row.to_string(), st.text()), width, 2))
        .collect();
    let max = max.max(3);
    if rows.len() <= max {
        return rows;
    }
    let head = (max - 1) / 2;
    let tail = max - 1 - head;
    let hidden = rows.len() - head - tail;
    let note = Line::styled(
        format!("  … {hidden} more rows (the end follows) …"),
        st.dim(),
    );
    let mut out: Vec<Line<'static>> = rows[..head].to_vec();
    out.push(note);
    out.extend_from_slice(&rows[rows.len() - tail..]);
    out
}

/// `Alt+Y allow once · ...` with the keys bound now.
pub fn keys_text(ui: &Ui) -> String {
    let items = [
        ("approve_once", "allow once"),
        ("approve_session", "allow for this session"),
        ("decline", "decline"),
    ];
    let mut parts: Vec<String> = (items.iter())
        .map(|(name, word)| match ui.keys.label(name) {
            Some(key) => format!("{key} {word}"),
            None => format!("{word} (unbound)"),
        })
        .collect();
    if let Some(stop) = ui.keys.label("stop") {
        parts.push(format!("{stop} cancels the turn"));
    }
    parts.join(" · ")
}

/// The prompt's lines inside its frame, at most `max` of them; empty when
/// no request waits.
pub fn lines(ui: &Ui, st: &Styles, width: usize, max: usize) -> Vec<Line<'static>> {
    let Some(request) = ui.approvals.first().filter(|_| shown(ui)) else {
        return Vec::new();
    };
    let inner = width.saturating_sub(2).max(1);
    let mark = Span::styled(lead(st.icons.info), st.warning());
    let mut out = vec![Line::from(vec![
        mark,
        Span::styled(what(request), st.warning()),
    ])];
    if let Some(agent) = &request.agent {
        out.push(Line::styled(
            format!("asked by subagent {agent}"),
            st.text(),
        ));
    }
    let more = ui.approvals.len() - 1;
    let fixed = out.len() + 1 + usize::from(more > 0);
    let text = prompt_text(request);
    out.extend(summary_rows(&text, inner, max.saturating_sub(fixed), st));
    out.extend(wrap_line(
        &Line::styled(keys_text(ui), st.accent()),
        inner,
        2,
    ));
    if more > 0 {
        out.push(Line::styled(
            format!("{more} more waiting after this one"),
            st.dim(),
        ));
    }
    out
}

/// The rows the prompt of `lines` takes with its frame (0 without one).
pub fn height(lines: &[Line<'static>]) -> u16 {
    match lines.len() {
        0 => 0,
        n => u16::try_from(n + 2).unwrap_or(u16::MAX),
    }
}

/// Draws the prompt in its frame: warning borders, the title in words.
pub fn draw(frame: &mut Frame, st: &Styles, lines: Vec<Line<'static>>, area: Rect) {
    if lines.is_empty() {
        return;
    }
    let block = Block::default()
        .borders(Borders::ALL)
        .border_style(st.warning())
        .title(Span::styled(" Approval ", st.warning()));
    frame.render_widget(Paragraph::new(lines).block(block), area);
}
