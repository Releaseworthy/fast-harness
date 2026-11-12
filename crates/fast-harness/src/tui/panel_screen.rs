//! The panel screen (PLAN.md D46, docs/panels.md): `/panel <id>` shows a
//! panel's view tree as text in place of the transcript. Tab and Shift+Tab
//! move between buttons, inputs, selects, checkboxes and list items; Enter
//! acts (an input sends its typed text as `submit`), Left and Right change
//! a select; Esc returns to the chat.

use std::sync::Arc;

use fast_harness_core::panels::{self, Cell, Host, Panel, Row};
use fast_harness_core::view::NoteKind;
use ratatui::crossterm::event::{KeyCode, KeyEvent, KeyEventKind, KeyModifiers};
use ratatui::layout::Rect;
use ratatui::style::Modifier;
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, Paragraph};
use ratatui::Frame;
use serde_json::Value;

use super::app::{Msg, Tui};
use super::theme::Styles;

/// The shown panel and the focus.
#[derive(Debug, Default)]
pub struct PanelScreen {
    pub id: String,
    pub title: String,
    pub rows: Vec<Row>,
    pub status: String,
    pub ended: bool,
    /// Index into the focusable cells.
    pub focus: usize,
    /// The text typed into the focused input.
    pub typed: String,
}

/// What a key asks the controller to do.
#[derive(Debug, PartialEq)]
pub enum Step {
    None,
    Close,
    /// Send this event kind, widget id and value to the process.
    Send(&'static str, String, Value),
}

const FOCUSABLE: [&str; 5] = ["button", "input", "select", "checkbox", "item"];

impl PanelScreen {
    /// Takes the panel's current rows and status, keeping the focus.
    pub fn sync(&mut self, panel: &Panel) {
        self.rows = panel.rows.clone();
        self.status = panel.status.clone();
        self.ended = panel.ended;
        let count = self.targets().len();
        if self.focus >= count {
            self.focus = count.saturating_sub(1);
        }
    }

    /// `(row, cell)` of every focusable cell, in reading order.
    fn targets(&self) -> Vec<(usize, usize)> {
        let rows = self.rows.iter().enumerate();
        rows.flat_map(|(r, row)| {
            (row.cells.iter().enumerate())
                .filter(|(_, c)| FOCUSABLE.contains(&c.kind.as_str()))
                .map(move |(c, _)| (r, c))
        })
        .collect()
    }

    fn focused(&self) -> Option<&Cell> {
        let (r, c) = *self.targets().get(self.focus)?;
        Some(&self.rows[r].cells[c])
    }

    fn move_focus(&mut self, forward: bool) {
        let count = self.targets().len().max(1);
        self.focus = match forward {
            true => (self.focus + 1) % count,
            false => (self.focus + count - 1) % count,
        };
        self.typed = self.focused().map(|c| c.value.clone()).unwrap_or_default();
    }

    pub fn key(&mut self, key: KeyEvent) -> Step {
        match key.code {
            KeyCode::Esc => return Step::Close,
            KeyCode::Tab if key.modifiers.contains(KeyModifiers::SHIFT) => self.move_focus(false),
            KeyCode::Tab => self.move_focus(true),
            KeyCode::BackTab => self.move_focus(false),
            KeyCode::Enter => return self.act(),
            KeyCode::Left => return self.step_select(false),
            KeyCode::Right => return self.step_select(true),
            KeyCode::Backspace if self.is_input() => {
                self.typed.pop();
            }
            KeyCode::Char(c) if self.is_input() => self.typed.push(c),
            _ => {}
        }
        Step::None
    }

    fn is_input(&self) -> bool {
        self.focused().is_some_and(|c| c.kind == "input")
    }

    fn act(&mut self) -> Step {
        let Some(cell) = self.focused() else {
            return Step::None;
        };
        let id = cell.id.clone();
        match cell.kind.as_str() {
            "button" => Step::Send("click", id, Value::Null),
            "item" => Step::Send("click", id, Value::String(cell.text.clone())),
            "checkbox" => Step::Send("change", id, Value::Bool(!cell.checked)),
            "input" => Step::Send("submit", id, Value::String(self.typed.clone())),
            "select" => self.step_select(true),
            _ => Step::None,
        }
    }

    fn step_select(&self, forward: bool) -> Step {
        let Some(cell) = self.focused().filter(|c| c.kind == "select") else {
            return Step::None;
        };
        let count = cell.options.len();
        if count == 0 {
            return Step::None;
        }
        let at = cell.options.iter().position(|o| *o == cell.value);
        let next = match (at, forward) {
            (None, _) => 0,
            (Some(i), true) => (i + 1) % count,
            (Some(i), false) => (i + count - 1) % count,
        };
        Step::Send(
            "change",
            cell.id.clone(),
            Value::String(cell.options[next].clone()),
        )
    }

    /// The screen as text lines; the focused cell has a `›` mark.
    pub fn lines(&self, st: &Styles) -> Vec<Line<'static>> {
        let focus = self.targets().get(self.focus).copied();
        let mut out = Vec::new();
        for (r, row) in self.rows.iter().enumerate() {
            let mut spans = vec![Span::raw("  ".repeat(row.depth))];
            for (c, cell) in row.cells.iter().enumerate() {
                let focused = focus == Some((r, c));
                let mark = if focused { "› " } else { "  " };
                let style = match (focused, cell.kind.as_str()) {
                    (true, _) => st.selected(),
                    (_, "note") => st.warning(),
                    _ if cell.header => st.text().add_modifier(Modifier::BOLD),
                    _ => st.text(),
                };
                spans.push(Span::styled(
                    format!("{mark}{}", self.cell_text(cell, focused)),
                    style,
                ));
            }
            out.push(Line::from(spans));
        }
        if !self.status.is_empty() {
            let (mark, style) = if self.ended {
                ("! ", st.warning())
            } else {
                ("i ", st.dim())
            };
            out.push(Line::styled(format!("{mark}{}", self.status), style));
        }
        out
    }

    fn cell_text(&self, cell: &Cell, focused: bool) -> String {
        let label = |text: &str| match text.is_empty() {
            true => String::new(),
            false => format!("{text}: "),
        };
        match cell.kind.as_str() {
            "button" => format!("[ {} ]", cell.text),
            "input" => {
                let value = if focused { &self.typed } else { &cell.value };
                let shown = if value.is_empty() { &cell.hint } else { value };
                format!("{}[{shown}_]", label(&cell.text))
            }
            "select" => format!("{}< {} >", label(&cell.text), cell.value),
            "checkbox" => format!("[{}] {}", if cell.checked { "x" } else { " " }, cell.text),
            "progress" => {
                let filled = (cell.progress * 20.0).round() as usize;
                let bar = format!("{}{}", "#".repeat(filled), "-".repeat(20 - filled.min(20)));
                format!("{}[{bar}] {:.0}%", label(&cell.text), cell.progress * 100.0)
            }
            "item" => format!("• {}", cell.text),
            "note" => format!("! {}", cell.text),
            _ => cell.text.replace('\n', " "),
        }
    }

    pub fn draw(&self, frame: &mut Frame, st: &Styles, area: Rect) {
        let title = format!(" {} · Tab moves, Enter acts, Esc closes ", self.title);
        let block = Block::default()
            .borders(Borders::TOP)
            .title(Span::styled(title, st.accent()));
        let lines = self.lines(st);
        let height = usize::from(area.height.saturating_sub(1)).max(1);
        let focus_row = self.targets().get(self.focus).map_or(0, |t| t.0);
        let skip = (focus_row + 1).saturating_sub(height);
        let shown: Vec<Line> = lines.into_iter().skip(skip).collect();
        frame.render_widget(Paragraph::new(shown).block(block).style(st.base()), area);
    }
}

impl Tui {
    /// `/panel [id]` (listed by `/help`): opens the panel, or lists the
    /// panels; true when `text` was this command.
    pub(super) fn panel_command(&mut self, text: &str) -> bool {
        let Some(id) = panels::slash_id(text) else {
            return false;
        };
        if self.panels.get(id).is_none() {
            let listing = panels::listing(&self.panels);
            let text = match id.is_empty() {
                true => listing,
                false => format!("no panel {id}\n{listing}"),
            };
            self.note(NoteKind::Info, text);
            return true;
        }
        let tx = self.tx.clone();
        let wake: panels::Wake = Arc::new(move || {
            let _ = tx.send(Msg::Panel);
        });
        let host = Host::new(&self.cwd, &self.session, self.ui.theme.dark);
        if let Some(panel) = self.panels.get_mut(id) {
            panel.open(host, wake);
            let mut screen = PanelScreen {
                id: id.into(),
                title: panel.file.title.clone(),
                ..PanelScreen::default()
            };
            screen.sync(panel);
            self.ui.panel = Some(screen);
        }
        true
    }

    /// A panel's thread queued messages: apply them to the shown screen.
    pub(super) fn panel_poll(&mut self) {
        if !self.panels.poll() {
            return;
        }
        if let Some(screen) = self.ui.panel.as_mut() {
            if let Some(panel) = self.panels.get(&screen.id) {
                screen.sync(panel);
            }
        }
    }

    /// A key while the panel screen is open.
    pub(super) fn panel_key(&mut self, key: KeyEvent) {
        let Some(screen) = self.ui.panel.as_mut() else {
            return;
        };
        if key.kind == KeyEventKind::Release {
            return;
        }
        match screen.key(key) {
            Step::None => {}
            Step::Close => self.ui.panel = None,
            Step::Send(kind, id, value) => {
                if let Some(panel) = self.panels.get(&screen.id) {
                    panel.send(kind, &id, value);
                }
            }
        }
    }
}
