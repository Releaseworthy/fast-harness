//! The Accounts page actions and the usage poller.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::thread;
use std::time::Duration;

use nh_core::accounts::oauth;
use nh_core::accounts::pool::{AccountPool, USAGE_EVERY_MS};
use nh_core::http::Transport;
use nh_core::platform;
use nh_core::trace::now_ms;
use slint::{ModelRc, VecModel};

use crate::app::{with_app, App};
use crate::rows::{account_label, account_row, window_cell};
use crate::MainWindow;

/// How long a sign-in waits for the browser callback.
const LOGIN_WAIT: Duration = Duration::from_secs(600);

fn transport(app: &App) -> Arc<dyn Transport> {
    app.rt.transport.clone()
}

fn say(ui: &MainWindow, text: impl Into<slint::SharedString>) {
    ui.set_accounts_message(text.into());
}

/// Account cards, the sidebar chip and the status bar account.
pub fn refresh(app: &App, ui: &MainWindow) {
    let now = now_ms() as i64;
    let statuses = app.rt.pool.status();
    let rows: Vec<_> = statuses.iter().map(|s| account_row(s, now)).collect();
    let active = statuses.iter().find(|s| s.active);
    let usage = active.and_then(|s| s.usage.as_ref());
    let five = window_cell(usage.and_then(|u| u.five_hour.as_ref()), now);
    let seven = window_cell(usage.and_then(|u| u.seven_day.as_ref()), now);
    let name = active.map_or_else(
        || "No account: open Accounts".to_string(),
        |s| account_label(&s.account),
    );
    ui.set_chip_name(name.into());
    ui.set_chip_five(five.0);
    ui.set_chip_five_text(five.1);
    ui.set_chip_seven(seven.0);
    ui.set_chip_seven_text(seven.1);
    ui.set_accounts(ModelRc::new(VecModel::from(rows)));
}

pub fn use_account(app: &App, ui: &MainWindow, id: &str) {
    if let Err(err) = app.rt.pool.set_active(id) {
        say(ui, format!("Cannot use this account: {err}"));
    }
    refresh(app, ui);
}

pub fn remove(app: &App, ui: &MainWindow, id: &str) {
    match app.rt.pool.remove(id) {
        Ok(account) => say(ui, format!("Removed {}.", account_label(&account))),
        Err(err) => say(ui, format!("Cannot remove this account: {err}")),
    }
    refresh(app, ui);
}

pub fn import(app: &App, ui: &MainWindow) {
    match app.rt.pool.import_linked() {
        Ok(count) => say(
            ui,
            format!("Imported or updated {count} claude-switch account(s)."),
        ),
        Err(err) => say(ui, format!("Import failed: {err}")),
    }
    refresh(app, ui);
}

/// The key is never shown or logged: only the label comes back.
pub fn add_key(app: &App, ui: &MainWindow, key: &str, label: &str) {
    match app.rt.pool.add_api_key(key.trim(), label.trim()) {
        Ok(account) => {
            say(
                ui,
                format!("Added the API key account {}.", account_label(&account)),
            );
            ui.set_adding_key(false);
        }
        Err(err) => say(ui, format!("Cannot add the key: {err}")),
    }
    refresh(app, ui);
}

pub fn refresh_usage(app: &App, ui: &MainWindow) {
    ui.set_accounts_working(true);
    say(ui, "Reading usage...");
    let (pool, http) = (app.rt.pool.clone(), transport(app));
    thread::spawn(move || {
        let read = pool.poll_usage(&*http);
        let _ = slint::invoke_from_event_loop(move || {
            with_app(|app, ui| {
                ui.set_accounts_working(false);
                say(ui, format!("Usage read for {read} account(s)."));
                refresh(app, ui);
            });
        });
    });
}

pub fn sign_in(app: &mut App, ui: &MainWindow) {
    if app.login_cancel.is_some() {
        return;
    }
    let flow = match oauth::begin_login() {
        Ok(flow) => flow,
        Err(err) => return say(ui, format!("Sign-in failed: {err}")),
    };
    let url = flow.url().to_string();
    match platform::open_url(&url) {
        Ok(()) => say(ui, ""),
        Err(err) => say(
            ui,
            format!("Cannot open the browser ({err}); open the URL below."),
        ),
    }
    ui.set_login_url(url.into());
    let cancel = Arc::new(AtomicBool::new(false));
    app.login_cancel = Some(cancel.clone());
    let (pool, http) = (app.rt.pool.clone(), transport(app));
    thread::spawn(move || {
        let result = pool
            .finish_login(&*http, &flow, LOGIN_WAIT, &cancel)
            .map(|account| account_label(&account))
            .map_err(|err| err.to_string());
        let _ = slint::invoke_from_event_loop(move || {
            with_app(|app, ui| login_done(app, ui, &cancel, result));
        });
    });
}

fn login_done(
    app: &mut App,
    ui: &MainWindow,
    flag: &Arc<AtomicBool>,
    result: Result<String, String>,
) {
    let current = app
        .login_cancel
        .as_ref()
        .is_some_and(|c| Arc::ptr_eq(c, flag));
    if !current {
        return;
    }
    app.login_cancel = None;
    ui.set_login_url("".into());
    match result {
        Ok(label) => say(ui, format!("Signed in as {label}.")),
        Err(_) if flag.load(Ordering::SeqCst) => say(ui, "Sign-in cancelled."),
        Err(err) => say(ui, format!("Sign-in failed: {err}")),
    }
    refresh(app, ui);
}

pub fn cancel_sign_in(app: &mut App, ui: &MainWindow) {
    if let Some(flag) = app.login_cancel.take() {
        flag.store(true, Ordering::SeqCst);
    }
    ui.set_login_url("".into());
    say(ui, "Sign-in cancelled.");
}

/// Reads the usage of every account now and then every `USAGE_EVERY_MS`
/// until `stop` is set (when the window closes).
pub fn start_poller(pool: Arc<AccountPool>, http: Arc<dyn Transport>, stop: Arc<AtomicBool>) {
    let spawned = thread::Builder::new()
        .name("nh-usage".into())
        .spawn(move || {
            let ticks = USAGE_EVERY_MS / 1000;
            while !stop.load(Ordering::SeqCst) {
                if pool.poll_usage(&*http) > 0 {
                    let _ = slint::invoke_from_event_loop(|| {
                        with_app(|app, ui| refresh(app, ui));
                    });
                }
                for _ in 0..ticks {
                    if stop.load(Ordering::SeqCst) {
                        return;
                    }
                    thread::sleep(Duration::from_secs(1));
                }
            }
        });
    drop(spawned);
}
