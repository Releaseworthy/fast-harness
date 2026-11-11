//! The Google Antigravity section of the Accounts page (PLAN.md D23, D25):
//! browser sign-in, Use, Remove and the quota cards.

use std::sync::atomic::AtomicBool;
use std::sync::Arc;
use std::thread;

use fast_harness_core::accounts::google::{self, GoogleApi};
use fast_harness_core::platform;
use fast_harness_core::trace::now_ms;
use slint::{ModelRc, VecModel};

use crate::accounts::{login_done, refresh, say, LOGIN_WAIT};
use crate::app::{with_app, App};
use crate::google_rows::google_row;
use crate::rows::account_label;
use crate::MainWindow;

/// The Google account cards.
pub fn refresh_rows(app: &App, ui: &MainWindow, statuses: &[google::GoogleStatus]) {
    let now = now_ms();
    let rows: Vec<_> = statuses
        .iter()
        .map(|s| google_row(s, app.rt.pool.google_project(&s.account.id), now))
        .collect();
    ui.set_google_accounts(ModelRc::new(VecModel::from(rows)));
}

pub fn use_account(app: &App, ui: &MainWindow, id: &str) {
    if let Err(err) = app.rt.google.use_account(id) {
        say(ui, format!("Cannot use this Google account: {err}"));
    }
    refresh(app, ui);
}

pub fn remove(app: &App, ui: &MainWindow, id: &str) {
    match app.rt.google.remove(id) {
        Ok(account) => say(ui, format!("Removed {}.", account_label(&account))),
        Err(err) => say(ui, format!("Cannot remove this account: {err}")),
    }
    refresh(app, ui);
}

/// Opens Google's sign-in in the browser; without an OAuth client the page
/// shows how to set one instead (D25).
pub fn sign_in(app: &mut App, ui: &MainWindow) {
    if app.login_cancel.is_some() {
        return;
    }
    let api = GoogleApi::default();
    let client = match google::load_client(&app.rt.home) {
        Ok(client) => client,
        Err(err) => return say(ui, err.to_string()),
    };
    let flow = match google::begin_login(&api, &client) {
        Ok(flow) => flow,
        Err(err) => return say(ui, format!("Google sign-in failed: {err}")),
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
    let (pool, http) = (app.rt.pool.clone(), app.rt.transport.clone());
    thread::spawn(move || {
        let result = flow
            .wait(LOGIN_WAIT, &cancel)
            .and_then(|code| google::fetch_sign_in(&api, &*http, &client, &flow, &code))
            .and_then(|sign_in| pool.add_google(sign_in))
            .map(|account| account_label(&account))
            .map_err(|err| err.to_string());
        let _ = slint::invoke_from_event_loop(move || {
            with_app(|app, ui| login_done(app, ui, &cancel, result));
        });
    });
}
