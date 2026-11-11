//! ratatui styles for the colour roles of an [`fast_harness_core::theme::Theme`].
//!
//! Every widget takes its colours from [`Styles`]; no other TUI module names
//! a colour. Meaning never rests on colour alone (WCAG 2.2 SC 1.4.1): marks
//! such as `✓`, `✗`, `↻` and `›` go with the status colours, so every
//! greyscale level reads the same.

use std::path::Path;

use fast_harness_core::config;
use fast_harness_core::theme::{self, Palette, Rgb, Theme, DEFAULT_THEME, GREYSCALE_MAX};
use pulldown_cmark::HeadingLevel;
use ratatui::style::{Color, Modifier, Style};
use ratatui::widgets::{Block, BorderType, Borders};

use super::icons::{self, Icons};
use super::ui::Ui;

/// A theme colour as a ratatui true colour.
pub fn color(c: Rgb) -> Color {
    Color::Rgb(c.0, c.1, c.2)
}

/// The styles of one palette (a theme at a greyscale level), by role.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Styles {
    pub palette: Palette,
    /// The marks that go with the colours.
    pub icons: &'static Icons,
}

impl Default for Styles {
    fn default() -> Styles {
        Styles::new(theme::by_id(DEFAULT_THEME))
    }
}

impl Styles {
    pub fn new(theme: &Theme) -> Styles {
        Styles {
            palette: theme.palette,
            icons: &icons::PLAIN,
        }
    }

    /// The theme and the icon set of the UI.
    pub fn of(ui: &Ui) -> Styles {
        Styles {
            palette: ui.palette,
            icons: ui.icons,
        }
    }

    pub fn palette(&self) -> Palette {
        self.palette
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

/// Switches `ui` to the theme after its current one (built-in, then user
/// themes) and saves it as `theme` in `<home>/config.json`; returns the new
/// theme. A failed save keeps the theme for this run and reports the error
/// in the status bar.
pub fn cycle(ui: &mut Ui, home: &Path) -> Theme {
    let next = ui.themes.next(&ui.theme.id).clone();
    ui.set_theme(next.clone());
    let saved = config::save_setting(home, "theme", serde_json::Value::from(next.id.as_str()));
    ui.status = match saved {
        Ok(()) => format!("theme {} (saved)", next.name),
        Err(err) => format!("theme {} (not saved: {err})", next.name),
    };
    next
}

/// Steps the greyscale level of `ui` by `delta`, wrapping past 0 and 100,
/// and saves it as `greyscale` in `<home>/config.json`; returns the level.
pub fn step_greyscale(ui: &mut Ui, home: &Path, delta: i16) -> u8 {
    let (max, now) = (i16::from(GREYSCALE_MAX), i16::from(ui.greyscale));
    let level = match now + delta {
        n if n > max => 0,
        n if n < 0 => max,
        n => n,
    };
    // 0..=100 by the match above.
    ui.set_greyscale(u8::try_from(level).unwrap_or(0));
    let level = ui.greyscale;
    let words = match level {
        0 => "greyscale off, full colour".to_string(),
        GREYSCALE_MAX => "greyscale 100 of 100, pure greys".to_string(),
        n => format!("greyscale {n} of 100"),
    };
    let saved = config::save_setting(home, "greyscale", serde_json::Value::from(level));
    ui.status = match saved {
        Ok(()) => format!("{words} (saved)"),
        Err(err) => format!("{words} (not saved: {err})"),
    };
    level
}
