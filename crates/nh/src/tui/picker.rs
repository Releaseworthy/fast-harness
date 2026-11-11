//! The model and effort pickers of the chat box: a small list drawn just
//! above the input. Ctrl+P opens the models, Ctrl+E the efforts of the
//! current model; Enter picks, Esc closes, other keys are ignored.

use nh_core::llm::anthropic::{builtin_models, efforts, fit_effort};
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
}

/// The display name of `model`, else its id.
pub fn model_name(model: &str) -> String {
    builtin_models()
        .into_iter()
        .find(|m| m.id == model)
        .map_or_else(|| model.to_string(), |m| m.display_name)
}

/// The effort after `current` among the efforts of `model`.
pub fn next_effort(model: &str, current: Option<&str>) -> &'static str {
    let list = efforts(model);
    let index = list.iter().position(|e| Some(*e) == current);
    list[index.map_or(0, |i| (i + 1) % list.len())]
}

impl Picker {
    pub fn new(kind: PickerKind, model: &str, effort: Option<&str>) -> Picker {
        let (items, current): (Vec<(String, String)>, Option<String>) = match kind {
            PickerKind::Model => {
                let mut items: Vec<(String, String)> = builtin_models()
                    .into_iter()
                    .map(|m| (m.id.clone(), format!("{:<18} {}", m.display_name, m.id)))
                    .collect();
                if !items.iter().any(|(id, _)| id == model) {
                    items.insert(0, (model.to_string(), model.to_string()));
                }
                (items, Some(model.to_string()))
            }
            PickerKind::Effort => {
                let items = efforts(model)
                    .into_iter()
                    .map(|e| (e.to_string(), e.to_string()))
                    .collect();
                (items, effort.map(String::from))
            }
        };
        let selected = items
            .iter()
            .position(|(value, _)| Some(value) == current.as_ref())
            .unwrap_or(0);
        Picker {
            kind,
            items,
            selected,
            current,
            rows: 8,
        }
    }

    pub fn title(&self) -> &'static str {
        match self.kind {
            PickerKind::Model => "Model for the next turn",
            PickerKind::Effort => "Thinking effort",
        }
    }
}

impl Ui {
    pub(super) fn open_picker(&mut self, kind: PickerKind) {
        self.picker = Some(Picker::new(kind, &self.model, self.effort.as_deref()));
    }

    /// Keys while a picker is open: it takes every key.
    pub(super) fn picker_key(&mut self, key: &KeyEvent) -> Action {
        let Some(picker) = self.picker.as_mut() else {
            return Action::None;
        };
        let last = picker.items.len().saturating_sub(1);
        let page = picker.rows.saturating_sub(1).max(1);
        match key.code {
            KeyCode::Up => picker.selected = picker.selected.saturating_sub(1),
            KeyCode::Down => picker.selected = (picker.selected + 1).min(last),
            KeyCode::PageUp => picker.selected = picker.selected.saturating_sub(page),
            KeyCode::PageDown => picker.selected = (picker.selected + page).min(last),
            KeyCode::Home => picker.selected = 0,
            KeyCode::End => picker.selected = last,
            KeyCode::Esc => self.picker = None,
            KeyCode::Enter => return self.pick(),
            _ if super::ui::is_ctrl(key, 'c') => self.picker = None,
            _ => {}
        }
        Action::None
    }

    fn pick(&mut self) -> Action {
        let Some(picker) = self.picker.take() else {
            return Action::None;
        };
        match (picker.kind, picker.items.get(picker.selected)) {
            (PickerKind::Model, Some((id, _))) => Action::SetModel(id.clone()),
            (PickerKind::Effort, Some((effort, _))) => Action::SetEffort(effort.clone()),
            _ => Action::None,
        }
    }

    /// Uses `model` from the next turn on and fits the effort to it;
    /// sets the status line to what applies.
    pub fn set_model(&mut self, model: String) {
        let before = self.effort.clone();
        self.effort = fit_effort(&model, before.as_deref());
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
        self.status = format!("{when}: {} · effort {effort}{note}", model_name(&model));
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
            let label = clip(label, label_width);
            let fill = " ".repeat(label_width.saturating_sub(text_width(&label)));
            let check = if Some(value) == picker.current.as_ref() {
                "✓"
            } else {
                " "
            };
            if index == picker.selected {
                let text = format!("› {label}{fill} {check}");
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
    let st = Styles::new(ui.theme);
    let Some(picker) = ui.picker.as_mut() else {
        return;
    };
    let widest = picker.items.iter().map(|(_, l)| text_width(l)).max();
    let wanted = widest.unwrap_or(0).max(text_width(picker.title()) + 2) + 6;
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
