//! Colour themes: copies a `fast_harness_core::theme::Theme`, at the
//! greyscale level of the settings, into the Slint `Theme` global and saves
//! the user's pick to `config.json`. The theme list (built-in, then the
//! user files of `<home>/themes`) lives here, read at start and on [`reload`].

use std::cell::RefCell;
use std::path::Path;

use fast_harness_core::config::{save_setting, Config};
use fast_harness_core::theme::{Rgb, Theme as CoreTheme, ThemeSet, GREYSCALE_MAX};
use slint::{Color, ComponentHandle};

use crate::app::{show_error, App};
use crate::{MainWindow, Theme};

thread_local! {
    static THEMES: RefCell<ThemeSet> = RefCell::new(ThemeSet::builtin());
}

fn with_set<R>(f: impl FnOnce(&ThemeSet) -> R) -> R {
    THEMES.with(|set| f(&set.borrow()))
}

/// Reads the built-in and user themes; returns the notes to show (skipped
/// files, errors, themes failing the contrast check).
pub fn load(app_home: &Path) -> Vec<String> {
    let set = ThemeSet::load(app_home);
    let notes = set.notes().to_vec();
    THEMES.with(|cell| *cell.borrow_mut() = set);
    notes
}

pub fn color(c: Rgb) -> Color {
    Color::from_rgb_u8(c.0, c.1, c.2)
}

/// Sets every colour role of `theme` at greyscale `level` and the colour
/// scheme of the std widgets.
pub fn apply(ui: &MainWindow, theme: &CoreTheme, level: u8) {
    let p = theme.shown(level);
    let g = ui.global::<Theme>();
    g.set_dark(theme.dark);
    g.set_greyscale(level >= GREYSCALE_MAX / 2);
    g.set_greyscale_level(i32::from(level.min(GREYSCALE_MAX)));
    for (role, c) in p.roles() {
        let c = color(c);
        match role {
            "bg" => g.set_bg(c),
            "bg_end" => g.set_bg_end(c),
            "surface" => g.set_surface(c),
            "raised" => g.set_raised(c),
            "hover" => g.set_hover(c),
            "selection" => g.set_selection(c),
            "bubble" => g.set_bubble(c),
            "code" => g.set_code(c),
            "accent_surface" => g.set_accent_surface(c),
            "line" => g.set_line(c),
            "border" => g.set_border(c),
            "text" => g.set_text(c),
            "dim" => g.set_dim(c),
            "accent" => g.set_accent(c),
            "accent2" => g.set_accent2(c),
            "on_accent" => g.set_on_accent(c),
            "glow" => g.set_glow(c),
            "shadow" => g.set_shadow(c),
            "error" => g.set_error(c),
            "warning" => g.set_warning(c),
            "success" => g.set_success(c),
            "info" => g.set_info(c),
            _ => {}
        }
    }
    ui.invoke_set_color_scheme(theme.dark);
}

/// Applies the configured theme and greyscale level.
pub fn apply_config(ui: &MainWindow, config: &Config) {
    let theme = with_set(|set| set.get(&config.theme).clone());
    apply(ui, &theme, config.greyscale);
}

/// The theme names in picker order.
#[cfg(test)]
pub fn names() -> Vec<String> {
    with_set(|set| set.themes().iter().map(|t| t.name.clone()).collect())
}

/// The picker index of the theme `id` resolves to (unknown ids: the default).
pub fn index_of(id: &str) -> usize {
    with_set(|set| set.position(id))
}

/// The theme at a picker index, `None` when it is out of range.
pub fn at(index: i32) -> Option<CoreTheme> {
    let index = usize::try_from(index).ok()?;
    with_set(|set| set.at(index).cloned())
}

/// Every theme in picker order (built-ins, then user themes).
pub fn all() -> Vec<CoreTheme> {
    with_set(|set| set.themes().to_vec())
}

/// The notes of the last load (skipped files, errors, contrast failures).
pub fn notes() -> Vec<String> {
    with_set(|set| set.notes().to_vec())
}

/// Sets `config.theme` to the theme at `index` and saves it to
/// `config.json`; `None` when the index is out of range.
pub fn choose(
    config: &mut Config,
    home: &Path,
    index: i32,
) -> Option<(CoreTheme, Result<(), String>)> {
    let theme = at(index)?;
    config.theme = theme.id.clone();
    let saved = save_setting(home, "theme", serde_json::json!(theme.id));
    Some((theme, saved))
}

/// Sets `config.greyscale` to `level` clamped to 0..=100 and saves it.
pub fn save_level(config: &mut Config, home: &Path, level: i32) -> Result<(), String> {
    let level = u8::try_from(level.clamp(0, i32::from(GREYSCALE_MAX))).unwrap_or(0);
    config.greyscale = level;
    save_setting(home, "greyscale", serde_json::json!(level))
}

/// Applies the picked theme at once and saves it.
pub fn pick(app: &mut App, ui: &MainWindow, index: i32) {
    let Some((theme, saved)) = choose(&mut app.rt.config, &app.rt.home, index) else {
        return;
    };
    apply(ui, &theme, app.rt.config.greyscale);
    if let Err(err) = saved {
        show_error(ui, format!("Cannot save the theme: {err}"));
    }
}

/// Sets the greyscale level (clamped to 0..=100), applies it at once and
/// saves it; for the Appearance slider and number box.
pub fn set_greyscale(app: &mut App, ui: &MainWindow, level: i32) {
    let saved = save_level(&mut app.rt.config, &app.rt.home, level);
    apply_config(ui, &app.rt.config);
    if let Err(err) = saved {
        show_error(ui, format!("Cannot save the greyscale level: {err}"));
    }
}

/// Reads the theme files again and applies the configured theme (the
/// default when its file is gone).
pub fn reload(app: &mut App, ui: &MainWindow) {
    load(&app.rt.home);
    apply_config(ui, &app.rt.config);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn names_follow_the_core_order() {
        assert_eq!(
            names(),
            [
                "Dark",
                "Light",
                "Aurora",
                "Synthwave",
                "Sunset",
                "Emerald",
                "Sakura",
                "Citrus"
            ]
        );
    }

    #[test]
    fn index_lookups() {
        assert_eq!(index_of("dark"), 0);
        assert_eq!(index_of("greyscale-light"), 1);
        assert_eq!(index_of(" sakura "), 6);
        assert_eq!(index_of("no-such-theme"), 0);
        assert_eq!(at(2).map(|t| t.id), Some("aurora".to_string()));
        assert!(at(-1).is_none());
        assert!(at(8).is_none());
    }

    #[test]
    fn user_themes_join_the_list() {
        let home =
            std::env::temp_dir().join(format!("fast-harness-gui-themes-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&home);
        std::fs::create_dir_all(home.join("themes")).unwrap();
        let mine = r#"{"version":1,"id":"mine","name":"Mine","kind":"light","extends":"citrus","colors":{}}"#;
        std::fs::write(home.join("themes").join("mine.json"), mine).unwrap();
        std::fs::write(home.join("themes").join("bad.json"), "{").unwrap();
        let notes = load(&home);
        assert_eq!(notes.len(), 1, "{notes:?}");
        assert!(notes[0].contains("bad.json"));
        assert_eq!(names().last().map(String::as_str), Some("Mine"));
        assert_eq!(index_of("mine"), 8);
        load(&home.join("missing"));
        assert_eq!(names().len(), 8);
        std::fs::remove_dir_all(&home).unwrap();
    }
}
