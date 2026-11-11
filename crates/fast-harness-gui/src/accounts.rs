//! The Accounts page actions and the usage poller.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::thread;
use std::time::Duration;

use fast_harness_core::accounts::google::GoogleAuth;
use fast_harness_core::accounts::model::Provider;
use fast_harness_core::accounts::oauth;
use fast_harness_core::accounts::pool::{AccountPool, USAGE_EVERY_MS};
use fast_harness_core::http::Transport;
use fast_harness_core::platform;
use fast_harness_core::trace::now_ms;
use slint::{ModelRc, VecModel};

use crate::app::{with_app, App};
use crate::rows::{account_label, account_row};
use crate::MainWindow;

/// How long a sign-in waits for the browser callback.
pub(crate) const LOGIN_WAIT: Duration = Duration::from_secs(600);

/// How often the poller offers the Google pool a quota read; the pool paces
/// each account itself (at most every 10 minutes, 30 s after a turn).
const GOOGLE_TICK_SECS: u64 = 30;

fn transport(app: &App) -> Arc<dyn Transport> {
    app.rt.transport.clone()
}

pub(crate) fn say(ui: &MainWindow, text: impl Into<slint::SharedString>) {
    ui.set_accounts_message(text.into());
}

/// The provider cards, the Claude account cards and the Google ones.
pub fn refresh(app: &App, ui: &MainWindow) {
    let now_ms = now_ms();
    let now = now_ms as i64;
    let mut statuses = app.rt.pool.status();
    statuses.retain(|s| s.account.provider != Provider::Google);
    let rows: Vec<_> = statuses.iter().map(|s| account_row(s, now)).collect();
    ui.set_accounts(ModelRc::new(VecModel::from(rows)));
    let google = app.rt.google.status();
    crate::google::refresh_rows(app, ui, &google);
    crate::providers::fill(ui, &app.rt.home, &statuses, &google, now_ms);
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

pub(crate) fn login_done(
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

fn repaint() {
    let _ = slint::invoke_from_event_loop(|| {
        with_app(|app, ui| refresh(app, ui));
    });
}

/// Reads the Claude usage now and then every `USAGE_EVERY_MS`, and offers
/// the Google pool a quota read every `GOOGLE_TICK_SECS`, until `stop` is
/// set (when the window closes). The page refreshes after any read.
pub fn start_poller(
    pool: Arc<AccountPool>,
    google: Arc<GoogleAuth>,
    http: Arc<dyn Transport>,
    stop: Arc<AtomicBool>,
) {
    let spawned = thread::Builder::new()
        .name("fast-harness-usage".into())
        .spawn(move || {
            let ticks = USAGE_EVERY_MS / 1000;
            let mut tick = 0;
            while !stop.load(Ordering::SeqCst) {
                let claude = tick % ticks == 0 && pool.poll_usage(&*http) > 0;
                let google = tick % GOOGLE_TICK_SECS == 0 && google.poll(&*http, now_ms()) > 0;
                if claude || google {
                    repaint();
                }
                thread::sleep(Duration::from_secs(1));
                tick += 1;
            }
        });
    drop(spawned);
}
