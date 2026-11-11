//! ratatui styles for the colour roles of an [`nh_core::theme::Theme`].
//!
//! Every widget takes its colours from [`Styles`]; no other TUI module names
//! a colour. Meaning never rests on colour alone (WCAG 2.2 SC 1.4.1): marks
//! such as `✓`, `✗`, `↻` and `›` go with the status colours, so the
//! greyscale themes read the same.

use std::path::Path;

use nh_core::config;
use nh_core::theme::{self, Palette, Rgb, Theme, DEFAULT_THEME};
use pulldown_cmark::HeadingLevel;
use ratatui::style::{Color, Modifier, Style};
use ratatui::widgets::{Block, BorderType, Borders};

use super::ui::Ui;

/// A theme colour as a ratatui true colour.
pub fn color(c: Rgb) -> Color {
    Color::Rgb(c.0, c.1, c.2)
}

/// The styles of one theme, by role.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Styles {
    pub theme: &'static Theme,
}

impl Default for Styles {
    fn default() -> Styles {
        Styles::new(theme::by_id(DEFAULT_THEME))
    }
}

impl Styles {
    pub fn new(theme: &'static Theme) -> Styles {
        Styles { theme }
    }

    pub fn palette(&self) -> &'static Palette {
        &self.theme.palette
    }

    fn fg(&self, c: Rgb) -> Style {
        Style::default().fg(color(c))
    }

    /// Body text on the page background: painted under the whole frame.
    pub fn base(&self) -> Style {
        self.fg(self.palette().text).bg(color(self.palette().bg))
    }

    pub fn text(&self) -> Style {
        self.fg(self.palette().text)
    }

    /// Secondary text: timings, footers, tool output.
    pub fn dim(&self) -> Style {
        self.fg(self.palette().dim)
    }

    pub fn accent(&self) -> Style {
        self.fg(self.palette().accent)
    }

    pub fn error(&self) -> Style {
        self.fg(self.palette().error)
    }

    pub fn warning(&self) -> Style {
        self.fg(self.palette().warning)
    }

    pub fn success(&self) -> Style {
        self.fg(self.palette().success)
    }

    /// The header bar: body text on `surface`.
    pub fn bar(&self) -> Style {
        self.text().bg(color(self.palette().surface))
    }

    /// The hint bar at rest.
    pub fn hint(&self) -> Style {
        self.dim().bg(color(self.palette().bg))
    }

    /// The hint bar holding a status message.
    pub fn status(&self) -> Style {
        self.warning().bg(color(self.palette().bg))
    }

    /// A selected list row (with a `›` marker, never reverse video).
    pub fn selected(&self) -> Style {
        self.text()
            .bg(color(self.palette().selection))
            .add_modifier(Modifier::BOLD)
    }

    /// A small label over the content, such as "↓ more".
    pub fn badge(&self) -> Style {
        self.text().bg(color(self.palette().raised))
    }

    /// A screen frame: rounded, in the `border` colour, with a titled label.
    pub fn block(&self, title: &str) -> Block<'static> {
        self.rounded(self.palette().border)
            .title(format!(" {title} "))
            .title_style(self.text().add_modifier(Modifier::BOLD))
    }

    /// The frame of the focused input: rounded, in `accent`.
    pub fn focus_block(&self) -> Block<'static> {
        self.rounded(self.palette().accent)
    }

    fn rounded(&self, c: Rgb) -> Block<'static> {
        Block::default()
            .borders(Borders::ALL)
            .border_type(BorderType::Rounded)
            .border_style(self.fg(c))
    }

    /// The `›` before a user message.
    pub fn user_marker(&self) -> Style {
        self.accent().add_modifier(Modifier::BOLD)
    }

    pub fn thinking(&self) -> Style {
        self.dim().add_modifier(Modifier::ITALIC)
    }

    /// Inline code and code blocks: body text on `code`.
    pub fn code(&self) -> Style {
        self.text().bg(color(self.palette().code))
    }

    pub fn heading(&self, level: HeadingLevel) -> Style {
        let style = self.accent().add_modifier(Modifier::BOLD);
        match level {
            HeadingLevel::H1 => style.add_modifier(Modifier::UNDERLINED),
            _ => style,
        }
    }

    pub fn link(&self) -> Style {
        self.accent().add_modifier(Modifier::UNDERLINED)
    }

    /// List bullets and numbers.
    pub fn marker(&self) -> Style {
        self.accent()
    }

    /// Quote bars and table separators.
    pub fn quote(&self) -> Style {
        self.fg(self.palette().border)
    }

    /// Horizontal rules: decorative.
    pub fn rule(&self) -> Style {
        self.fg(self.palette().line)
    }
}

/// Switches `ui` to the theme after its current one and saves it as
/// `theme` in `<home>/config.json`; returns the new theme. A failed save
/// keeps the theme for this run and reports the error in the status bar.
pub fn cycle(ui: &mut Ui, home: &Path) -> &'static Theme {
    let next = theme::next(ui.theme.id);
    ui.theme = next;
    let saved = config::save_setting(home, "theme", serde_json::Value::from(next.id));
    ui.status = match saved {
        Ok(()) => format!("theme {} (saved)", next.name),
        Err(err) => format!("theme {} (not saved: {err})", next.name),
    };
    next
}
