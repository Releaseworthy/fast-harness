//! Colour themes: copies a `nh_core::theme::Theme` into the Slint `Theme`
//! global and saves the user's pick to `config.json`.

use nh_core::config::save_setting;
use nh_core::theme::{by_id, Rgb, Theme as CoreTheme, THEMES};
use slint::{Color, ComponentHandle, ModelRc, SharedString, VecModel};

use crate::app::{show_error, App};
use crate::{MainWindow, Theme};

fn color(c: Rgb) -> Color {
    Color::from_rgb_u8(c.0, c.1, c.2)
}

/// Sets every colour role and the colour scheme of the std widgets.
pub fn apply(ui: &MainWindow, theme: &CoreTheme) {
    let p = &theme.palette;
    let g = ui.global::<Theme>();
    g.set_dark(theme.dark);
    g.set_greyscale(theme.id.starts_with("greyscale"));
    g.set_bg(color(p.bg));
    g.set_surface(color(p.surface));
    g.set_raised(color(p.raised));
    g.set_hover(color(p.hover));
    g.set_selection(color(p.selection));
    g.set_bubble(color(p.bubble));
    g.set_code(color(p.code));
    g.set_line(color(p.line));
    g.set_border(color(p.border));
    g.set_text(color(p.text));
    g.set_dim(color(p.dim));
    g.set_accent(color(p.accent));
    g.set_on_accent(color(p.on_accent));
    g.set_error(color(p.error));
    g.set_warning(color(p.warning));
    g.set_success(color(p.success));
    ui.invoke_set_color_scheme(theme.dark);
}

/// The theme names in picker order.
pub fn names() -> Vec<&'static str> {
    THEMES.iter().map(|t| t.name).collect()
}

/// The picker index of the theme `id` resolves to (unknown ids: the default).
pub fn index_of(id: &str) -> usize {
    let theme = by_id(id);
    THEMES.iter().position(|t| t.id == theme.id).unwrap_or(0)
}

/// The theme at a picker index, `None` when it is out of range.
pub fn at(index: i32) -> Option<&'static CoreTheme> {
    usize::try_from(index).ok().and_then(|i| THEMES.get(i))
}

/// Fills the Settings list with the theme names and the current pick.
pub fn fill(ui: &MainWindow, id: &str) {
    let names: Vec<SharedString> = names().into_iter().map(SharedString::from).collect();
    ui.set_themes(ModelRc::new(VecModel::from(names)));
    ui.set_theme_index(index_of(id) as i32);
}

/// Applies the picked theme at once and saves it.
pub fn pick(app: &mut App, ui: &MainWindow, index: i32) {
    let Some(theme) = at(index) else {
        return;
    };
    apply(ui, theme);
    app.rt.config.theme = theme.id.to_string();
    if let Err(err) = save_setting(&app.rt.home, "theme", serde_json::json!(theme.id)) {
        show_error(ui, format!("Cannot save the theme: {err}"));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn names_follow_the_core_order() {
        assert_eq!(
            names(),
            vec!["Dark", "Light", "Greyscale dark", "Greyscale light"]
        );
    }

    #[test]
    fn index_lookups() {
        assert_eq!(index_of("dark"), 0);
        assert_eq!(index_of("greyscale-light"), 3);
        assert_eq!(index_of(" light "), 1);
        assert_eq!(index_of("no-such-theme"), 0);
        assert_eq!(at(2).map(|t| t.id), Some("greyscale-dark"));
        assert!(at(-1).is_none());
        assert!(at(4).is_none());
    }
}
