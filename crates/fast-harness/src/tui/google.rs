//! The Google Antigravity rows of the accounts screen and their sign-in
//! (PLAN.md D23, D25).

use std::sync::atomic::AtomicBool;
use std::sync::Arc;
use std::thread;

use fast_harness_core::accounts::google::{
    self, GoogleApi, GoogleQuota, GoogleStatus, HoldReason, QuotaWindow,
};
use fast_harness_core::platform;

use super::app::{Msg, Tui};
use super::ui::Dialog;
use super::wrap::clip;
use crate::accounts_cmd::{label, relative};

/// How long a Google sign-in waits for the browser.
const LOGIN_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(600);

/// `42%`, `off` when disabled, `-` when unknown; 0 once the reset passed.
fn used(window: Option<&QuotaWindow>, now: u64) -> String {
    match window {
        Some(w) if w.disabled => "off".into(),
        Some(w) => format!("{:.0}%", w.used_at(now) * 100.0),
        None => "-".into(),
    }
}

/// `5h 42% wk 10%` of the first group whose name starts with `prefix`.
fn group(quota: Option<&GoogleQuota>, prefix: &str, now: u64) -> String {
    let found = quota.and_then(|q| q.groups.iter().find(|g| g.name.starts_with(prefix)));
    format!(
        "5h {:>4} wk {:>4}",
        used(found.and_then(|g| g.five_hour.as_ref()), now),
        used(found.and_then(|g| g.weekly.as_ref()), now)
    )
}

/// `held (rate limit), resumes in 1h 30m`, `needs verification`, the last error, or `ok`.
fn state(status: &GoogleStatus, now: u64) -> String {
    let now_i = i64::try_from(now).unwrap_or(i64::MAX);
    if let Some(hold) = &status.hold {
        let left = relative(i64::try_from(hold.until).unwrap_or(i64::MAX) - now_i);
        return match hold.reason {
            HoldReason::RateLimit => format!("held (rate limit), resumes {left}"),
            HoldReason::Validation => "needs verification".into(),
            HoldReason::Refused => format!("held (refused), resumes {left}"),
            HoldReason::Disabled => "disabled by the provider".into(),
        };
    }
    match &status.last_error {
        Some(err) => format!("error: {err}"),
        None if status.read_at.is_none() => "quota not read".into(),
        None => "ok".into(),
    }
}

/// One Google row: active marker, provider, name, plan, Gemini and Claude
/// and GPT quota, state.
pub fn google_row(status: &GoogleStatus, now: u64, mark: &str) -> String {
    let account = &status.account;
    let quota = status.quota.as_ref();
    format!(
        "{} {:<6} {:<28} {:<6} Gemini {} | Claude/GPT {} {}",
        if status.active { mark } else { " " },
        "google",
        clip(&label(&account.email, &account.name), 28),
        clip(account.plan.as_deref().unwrap_or("-"), 6),
        group(quota, "Gemini", now),
        group(quota, "Claude", now),
        state(status, now)
    )
}

impl Tui {
    /// Starts the Google browser sign-in; without an OAuth client the hint
    /// bar says how to set one.
    pub fn login_google(&mut self) {
        let api = GoogleApi::default();
        let started = google::load_client(&self.rt.home)
            .and_then(|client| google::begin_login(&api, &client).map(|flow| (client, flow)));
        let (client, flow) = match started {
            Ok(pair) => pair,
            Err(err) => {
                self.ui.status = err.to_string();
                return;
            }
        };
        let url = flow.url().to_string();
        if let Err(err) = platform::open_url(&url) {
            self.ui.status = format!("could not open the browser: {err}");
        }
        let cancel = Arc::new(AtomicBool::new(false));
        let (pool, http, tx, flag) = (
            self.rt.pool.clone(),
            self.rt.transport.clone(),
            self.tx.clone(),
            cancel.clone(),
        );
        let spawned = thread::Builder::new()
            .name("fast-harness-google-login".into())
            .spawn(move || {
                let result = flow
                    .wait(LOGIN_TIMEOUT, &flag)
                    .and_then(|code| google::fetch_sign_in(&api, &*http, &client, &flow, &code))
                    .and_then(|sign_in| pool.add_google(sign_in))
                    .map(|account| label(&account.email, &account.name))
                    .map_err(|err| err.to_string());
                let _ = tx.send(Msg::Login(result));
            });
        match spawned {
            Ok(_) => {
                self.ui.dialog = Dialog::Login(url);
                self.set_login_cancel(Some(cancel));
            }
            Err(err) => self.ui.status = format!("sign-in failed: {err}"),
        }
    }
}

#[cfg(test)]
#[path = "google_tests.rs"]
mod tests;
