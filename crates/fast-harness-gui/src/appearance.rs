//! Settings > Appearance (PLAN.md D33): the theme cards, the greyscale
//! setting and the user themes folder, through the `AppearanceUi` global.

use std::path::{Path, PathBuf};

use fast_harness_core::config::{save_setting, CHAT_FULL, CHAT_SUMMARY};
use fast_harness_core::platform;
use fast_harness_core::theme::{check, Theme as CoreTheme};
use slint::{ComponentHandle, Model, ModelRc, VecModel};

use crate::app::{show_error, with_app, App};
use crate::{theme, AppearanceUi, MainWindow, ThemeCard};

/// The note words of a contrast failure; the cards list those pairs.
const CONTRAST_NOTE: &str = "fails the contrast check";

/// The failing contrast pairs of a user theme, one per line ("" when it
/// passes; built-in themes always pass, `theme_tests.rs`).
pub fn failures(t: &CoreTheme) -> String {
    if t.is_builtin() {
        return String::new();
    }
    let pairs: Vec<String> = check(&t.palette).iter().map(ToString::to_string).collect();
    pairs.join("\n")
}

/// The card of `t` with its colours at greyscale `level`.
pub fn card(t: &CoreTheme, level: u8, current: bool) -> ThemeCard {
    let p = t.shown(level);
    let c = |role: &str| p.get(role).map(theme::color).unwrap_or_default();
    ThemeCard {
        id: t.id.as_str().into(),
        name: t.name.as_str().into(),
        dark: t.dark,
        custom: !t.is_builtin(),
        current,
        failures: failures(t).into(),
        bg: c("bg"),
        bg_end: c("bg_end"),
        surface: c("surface"),
        raised: c("raised"),
        line: c("line"),
        text: c("text"),
        dim: c("dim"),
        accent: c("accent"),
        accent2: c("accent2"),
        on_accent: c("on_accent"),
    }
}

/// Every theme's card, the theme `current` resolves to marked.
pub fn cards(current: &str, level: u8) -> Vec<ThemeCard> {
    let picked = theme::index_of(current);
    let all = theme::all();
    all.iter()
        .enumerate()
        .map(|(i, t)| card(t, level, i == picked))
        .collect()
}

/// The load notes for the page, without the contrast failures (the cards
/// show those).
pub fn notes() -> String {
    let notes = theme::notes();
    let shown: Vec<&str> = notes
        .iter()
        .map(String::as_str)
        .filter(|n| !n.contains(CONTRAST_NOTE))
        .collect();
    shown.join("\n")
}

pub fn themes_dir(home: &Path) -> PathBuf {
    home.join("themes")
}

/// Fills the cards, the greyscale level and the notes. Same-length lists
/// update in place, so a focused card keeps its focus.
pub fn fill(ui: &MainWindow, current: &str, level: u8) {
    let g = ui.global::<AppearanceUi>();
    let list = cards(current, level);
    let model = g.get_themes();
    match model.as_any().downcast_ref::<VecModel<ThemeCard>>() {
        Some(rows) if rows.row_count() == list.len() => {
            for (i, row) in list.into_iter().enumerate() {
                rows.set_row_data(i, row);
            }
        }
        _ => g.set_themes(ModelRc::new(VecModel::from(list))),
    }
    let failing: Vec<ThemeCard> = cards(current, level)
        .into_iter()
        .filter(|c| !c.failures.is_empty())
        .collect();
    g.set_failing(ModelRc::new(VecModel::from(failing)));
    g.set_greyscale(i32::from(level));
    g.set_notes(notes().into());
}

pub fn show(app: &App, ui: &MainWindow) {
    fill(ui, &app.rt.config.theme, app.rt.config.greyscale);
    let dir = themes_dir(&app.rt.home);
    let folder = format!(
        "Your theme files: {} (format: docs/themes.md)",
        dir.display()
    );
    ui.global::<AppearanceUi>().set_folder(folder.into());
    (ui.global::<AppearanceUi>()).set_chat_full(app.rt.config.chat_full());
    crate::limits::show(app, ui);
}

/// Applies `chatDetail` (D73) to the shown chat at once and saves it.
fn set_chat_detail(app: &mut App, ui: &MainWindow, full: bool) {
    let word = if full { CHAT_FULL } else { CHAT_SUMMARY };
    app.rt.config.chat_detail = word.into();
    app.mirror.set_full(full, &*app.rows);
    if let Err(err) = save_setting(&app.rt.home, "chatDetail", serde_json::json!(word)) {
        show_error(ui, format!("Cannot save the chat detail: {err}"));
    }
}

/// Creates `<home>/themes` when missing and opens it in Explorer.
fn open_folder(app: &App, ui: &MainWindow) {
    let dir = themes_dir(&app.rt.home);
    let opened = std::fs::create_dir_all(&dir)
        .map_err(|e| e.to_string())
        .and_then(|()| platform::open_folder(&dir).map_err(|e| e.to_string()));
    if let Err(err) = opened {
        show_error(ui, format!("Cannot open {}: {err}", dir.display()));
    }
}

pub fn connect(ui: &MainWindow) {
    crate::limits::connect(ui);
    let g = ui.global::<AppearanceUi>();
    g.on_pick(|index| {
        with_app(|app, ui| {
            theme::pick(app, ui, index);
            show(app, ui);
        });
    });
    g.on_greyscale_changed(|level| {
        with_app(|app, ui| {
            theme::set_greyscale(app, ui, level);
            show(app, ui);
        });
    });
    // The chat's step lines (D73): opened and folded here, switched off
    // by `chatDetail`.
    ui.on_step_toggle(|index| {
        with_app(|app, _| {
            let index = usize::try_from(index).unwrap_or(usize::MAX);
            app.mirror.toggle_step(index, &*app.rows);
        });
    });
    ui.on_turn_toggle(|head| {
        with_app(|app, _| {
            let head = usize::try_from(head).unwrap_or(usize::MAX);
            app.mirror.toggle_turn(head, &*app.rows);
        });
    });
    g.on_chat_detail_changed(|full| {
        with_app(|app, ui| set_chat_detail(app, ui, full));
    });
    g.on_open_folder(|| {
        with_app(|app, ui| open_folder(app, ui));
    });
    g.on_reload(|| {
        with_app(|app, ui| {
            theme::reload(app, ui);
            show(app, ui);
        });
    });
}
