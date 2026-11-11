//! Slint callbacks to the app.

use fast_harness_core::platform;
use slint::ComponentHandle;

use crate::app::{show_error, with_app};
use crate::{
    accounts, mirror, model_refresh, settings, theme, trace_rows, turn, window_state, MainWindow,
};

pub fn connect(ui: &MainWindow) {
    ui.on_send(|text| with_app(|app, ui| turn::send(app, ui, &text)).unwrap_or(false));
    ui.on_stop(|| {
        with_app(turn::stop);
    });
    ui.on_new_session(|| {
        with_app(|app, ui| app.new_session(ui));
    });
    ui.on_open_session(|id| {
        with_app(|app, ui| app.open_session(ui, &id));
    });
    ui.on_open_page(|page| {
        with_app(|app, ui| open_page(app, ui, &page));
    });
    ui.on_toggle(|index| {
        with_app(|app, _| mirror::toggle(&app.rows, usize::try_from(index).unwrap_or(usize::MAX)));
    });
    ui.on_refresh_trace(|| {
        with_app(|app, ui| app.refresh_trace(ui));
    });
    ui.on_trace_toggle(|index| {
        with_app(|_, ui| trace_rows::toggle(ui, index));
    });
    ui.on_model_picked(|index| {
        with_app(|app, ui| settings::pick_model(app, ui, index));
    });
    ui.on_effort_picked(|index| {
        with_app(|app, ui| settings::pick_effort(app, ui, index));
    });
    ui.on_pickers_opened(|| {
        with_app(model_refresh::start);
    });
    ui.on_theme_changed(|index| {
        with_app(|app, ui| theme::pick(app, ui, index));
    });
    connect_accounts(ui);
    connect_open_url(ui);
    window_state::connect(ui);
}

fn connect_accounts(ui: &MainWindow) {
    ui.on_sign_in(|| {
        with_app(accounts::sign_in);
    });
    ui.on_cancel_sign_in(|| {
        with_app(accounts::cancel_sign_in);
    });
    ui.on_add_key(|key, label| {
        with_app(|app, ui| accounts::add_key(app, ui, &key, &label));
    });
    ui.on_refresh_usage(|| {
        with_app(|app, ui| accounts::refresh_usage(app, ui));
    });
    ui.on_use_account(|id| {
        with_app(|app, ui| accounts::use_account(app, ui, &id));
    });
    ui.on_remove_account(|id| {
        with_app(|app, ui| accounts::remove(app, ui, &id));
    });
}

fn connect_open_url(ui: &MainWindow) {
    let weak = ui.as_weak();
    ui.on_open_url(move |url| {
        let opened = url.starts_with("https://") || url.starts_with("http://");
        let result = if opened {
            platform::open_url(&url).map_err(|e| e.to_string())
        } else {
            Err("only web links open".into())
        };
        if let (Err(err), Some(ui)) = (result, weak.upgrade()) {
            show_error(&ui, format!("Cannot open {url}: {err}"));
        }
    });
}

/// When the settings could not load: the window shows the error and the
/// send button explains it.
pub fn connect_minimal(ui: &MainWindow) {
    let weak = ui.as_weak();
    ui.on_send(move |_| {
        if let Some(ui) = weak.upgrade() {
            if ui.get_error_text().is_empty() {
                show_error(
                    &ui,
                    "The settings did not load; fix them and restart fast-harness-gui.",
                );
            }
        }
        false
    });
    connect_open_url(ui);
    window_state::connect(ui);
}

fn open_page(app: &mut crate::app::App, ui: &MainWindow, page: &str) {
    match page {
        "accounts" => accounts::refresh(app, ui),
        "trace" => app.refresh_trace(ui),
        _ => {}
    }
    ui.set_page(page.into());
}

/// Fills every list once the app is installed.
pub fn first_paint() {
    with_app(|app, ui| {
        settings::fill(app, ui);
        app.refresh_sessions(ui);
        app.refresh_status(ui);
        accounts::refresh(app, ui);
    });
}
