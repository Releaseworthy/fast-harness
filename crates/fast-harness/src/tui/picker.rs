//! The model and effort pickers of the chat box: a small list drawn just
//! above the input. Ctrl+P opens the models, Ctrl+E the efforts of the
//! current model; Enter picks, Esc closes, other keys are ignored.

use fast_harness_core::app::pickers::{effort_list, fetched, kept_row, model_list, Fetched};
use fast_harness_core::app::words::Voice;
use fast_harness_core::llm::anthropic::ModelInfo;
use fast_harness_core::llm::catalog::label;
use fast_harness_core::llm::Catalog;
use ratatui::crossterm::event::{KeyCode, KeyEvent};
use ratatui::layout::Rect;
use ratatui::style::Modifier;
use ratatui::text::{Line, Span};
use ratatui::widgets::{Clear, Paragraph};
use ratatui::Frame;

use super::theme::Styles;
use super::ui::{Action, Ui};
use super::wrap::{clip, text_width};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PickerKind {
    Model,
    Effort,
}

/// One open picker: `(value, label)` items and the highlighted row.
#[derive(Clone, Debug, PartialEq)]
pub struct Picker {
    pub kind: PickerKind,
    pub items: Vec<(String, String)>,
    pub selected: usize,
    /// The value in use, marked with `✓`.
    pub current: Option<String>,
    /// Visible rows at the last draw (page size).
    pub rows: usize,
    /// Shown after the title: the live list is loading, or why it failed.
    pub note: Option<String>,
}

/// The display name of `model` (`catalog::label`), else its id.
pub fn model_name(facts: &Catalog, model: &str) -> String {
    (facts.display_name(model))
        .map(|name| label(model, &name))
        .unwrap_or_else(|| model.to_string())
}

/// The effort after `current` among the efforts of `model`; `None` when
/// the model takes no effort.
pub fn next_effort(facts: &Catalog, model: &str, current: Option<&str>) -> Option<String> {
    let list = facts.efforts(model);
    let index = list.iter().position(|e| Some(e.as_str()) == current);
    let next = index.map_or(0, |i| (i + 1) % list.len().max(1));
    list.get(next).cloned()
}

pub use fast_harness_core::app::pickers::is_header;

/// The model rows of `catalog` in the terminal's words (`app::pickers`).
fn model_rows(facts: &Catalog, catalog: &[ModelInfo], model: &str) -> Vec<(String, String)> {
    model_list(facts, catalog, model, Voice::Terminal)
}

/// The efforts `catalog` lists for `model`, else the known ones.
fn effort_rows(facts: &Catalog, catalog: &[ModelInfo], model: &str) -> Vec<(String, String)> {
    let list = effort_list(facts, catalog, model);
    list.into_iter().map(|e| (e.clone(), e)).collect()
}

/// The nearest pickable row from `index`, looking `forward` first, then back.
fn settle(items: &[(String, String)], index: usize, forward: bool) -> usize {
    let index = index.min(items.len().saturating_sub(1));
    let ahead = |i: &usize| !is_header(&items[*i]);
    let after = (index..items.len()).find(ahead);
    let before = (0..=index).rev().find(ahead);
    let found = if forward {
        after.or(before)
    } else {
        before.or(after)
    };
    found.unwrap_or(index)
}

impl Picker {
    pub fn new(
        kind: PickerKind,
        facts: &Catalog,
        catalog: &[ModelInfo],
        model: &str,
        effort: Option<&str>,
    ) -> Picker {
        let (items, current) = match kind {
            PickerKind::Model => (model_rows(facts, catalog, model), Some(model.to_string())),
            PickerKind::Effort => (effort_rows(facts, catalog, model), effort.map(String::from)),
        };
        let found = items
            .iter()
            .position(|(value, _)| Some(value) == current.as_ref());
        let selected = settle(&items, found.unwrap_or(0), true);
        Picker {
            kind,
            items,
            selected,
            current,
            rows: 8,
            note: None,
        }
    }

    pub fn title(&self) -> String {
        let title = match self.kind {
            PickerKind::Model => "Model for the next turn",
            PickerKind::Effort => "Thinking effort",
        };
        match &self.note {
            Some(note) => format!("{title} · {note}"),
            None => title.to_string(),
        }
    }
}

/// Picker note while the live list loads.
pub const REFRESHING: &str = "refreshing...";

impl Ui {
    pub(super) fn open_picker(&mut self, kind: PickerKind) {
        let mut picker = Picker::new(
            kind,
            &self.facts,
            &self.catalog,
            &self.model,
            self.effort.as_deref(),
        );
        picker.note = self.fetching.then(|| REFRESHING.to_string());
        self.picker = Some(picker);
    }

    /// A live fetch of the model list started.
    pub fn fetch_started(&mut self) {
        self.fetching = true;
        if let Some(picker) = self.picker.as_mut() {
            picker.note = Some(REFRESHING.into());
        }
    }

    /// The fetch ended: a list replaces the catalog and refreshes the open
    /// picker in place (same highlighted value); an error is shown in its
    /// title and the list stays.
    pub fn models_fetched(&mut self, result: Fetched) {
        self.fetching = false;
        match fetched(result, Voice::Terminal) {
            Ok(list) => self.catalog = list,
            Err(note) => {
                if let Some(picker) = self.picker.as_mut() {
                    picker.note = Some(note);
                }
                return;
            }
        }
        let Some(old) = self.picker.take() else {
            return;
        };
        let mut picker = Picker::new(
            old.kind,
            &self.facts,
            &self.catalog,
            &self.model,
            self.effort.as_deref(),
        );
        if let Some(index) = kept_row(&old.items, old.selected, &picker.items) {
            picker.selected = index;
        }
        picker.rows = old.rows;
        self.picker = Some(picker);
    }
    /// Keys while a picker is open: it takes every key.
    pub(super) fn picker_key(&mut self, key: &KeyEvent) -> Action {
        let Some(picker) = self.picker.as_mut() else {
            return Action::None;
        };
        let last = picker.items.len().saturating_sub(1);
        let page = picker.rows.saturating_sub(1).max(1);
        let at = picker.selected;
        let moved = match key.code {
            KeyCode::Up => Some((at.saturating_sub(1), false)),
            KeyCode::Down => Some(((at + 1).min(last), true)),
            KeyCode::PageUp => Some((at.saturating_sub(page), false)),
            KeyCode::PageDown => Some(((at + page).min(last), true)),
            KeyCode::Home => Some((0, true)),
            KeyCode::End => Some((last, false)),
            KeyCode::Enter => return self.pick(),
            KeyCode::Esc => None,
            _ if super::ui::is_ctrl(key, 'c') => None,
            _ => return Action::None,
        };
        match moved {
            Some((index, forward)) => picker.selected = settle(&picker.items, index, forward),
            None => self.picker = None,
        }
        Action::None
    }

    fn pick(&mut self) -> Action {
        let Some(picker) = self.picker.take() else {
            return Action::None;
        };
        let item = picker.items.get(picker.selected).filter(|i| !is_header(i));
        match (picker.kind, item) {
            (PickerKind::Model, Some((id, _))) => Action::SetModel(id.clone()),
            (PickerKind::Effort, Some((effort, _))) => Action::SetEffort(effort.clone()),
            _ => Action::None,
        }
    }

    /// Uses `model` from the next turn on and fits the effort to it;
    /// sets the status line to what applies.
    pub fn set_model(&mut self, model: String) {
        let before = self.effort.clone();
        self.effort = self.facts.fit_effort(&model, before.as_deref());
        let effort = self.effort.as_deref().unwrap_or("default");
        let note = match &before {
            Some(old) if self.effort != before => format!(" ({old} does not apply)"),
            _ => String::new(),
        };
        let when = if self.running {
            "after this turn"
        } else {
            "next turn"
        };
        self.status = format!(
            "{when}: {} · effort {effort}{note}",
            model_name(&self.facts, &model)
        );
        self.model = model;
    }
}

/// The picker rows: `› ` on the highlighted row, `✓` on the current value.
fn rows(picker: &Picker, width: usize, height: usize, st: &Styles) -> Vec<Line<'static>> {
    let skip = (picker.selected + 1).saturating_sub(height.max(1));
    let label_width = width.saturating_sub(4);
    picker
        .items
        .iter()
        .enumerate()
        .skip(skip)
        .take(height)
        .map(|(index, (value, label))| {
            if value.is_empty() {
                let title = clip(label, width.saturating_sub(1));
                return Line::from(Span::styled(format!(" {title}"), st.dim()));
            }
            let label = clip(label, label_width);
            let fill = " ".repeat(label_width.saturating_sub(text_width(&label)));
            let check = if Some(value) == picker.current.as_ref() {
                st.icons.ok
            } else {
                " "
            };
            if index == picker.selected {
                let text = format!("{} {label}{fill} {check}", st.icons.pointer);
                Line::from(Span::styled(text, st.selected()))
            } else {
                let mark = Span::styled(format!(" {check}"), st.success());
                Line::from(vec![Span::raw(format!("  {label}{fill}")), mark])
            }
        })
        .collect()
}

/// Draws the open picker just above `input`, inside `body`.
pub fn draw(frame: &mut Frame, ui: &mut Ui, input: Rect, body: Rect) {
    let st = Styles::of(ui);
    let Some(picker) = ui.picker.as_mut() else {
        return;
    };
    let widest = picker.items.iter().map(|(_, l)| text_width(l)).max();
    let wanted = widest.unwrap_or(0).max(text_width(&picker.title()) + 2) + 6;
    let width = (wanted as u16).min(body.width);
    let height = (picker.items.len() as u16 + 2).min(body.height);
    if width < 8 || height < 3 {
        return;
    }
    let y = input.y.saturating_sub(height).max(body.y);
    let area = Rect::new(input.x, y, width, height);
    picker.rows = usize::from(height - 2);
    let inner = usize::from(width - 2);
    let lines = rows(picker, inner, picker.rows, &st);
    let block = st
        .focus_block()
        .title(format!(" {} ", picker.title()))
        .title_style(st.text().add_modifier(Modifier::BOLD));
    frame.render_widget(Clear, area);
    let widget = Paragraph::new(lines).style(st.base()).block(block);
    frame.render_widget(widget, area);
}
