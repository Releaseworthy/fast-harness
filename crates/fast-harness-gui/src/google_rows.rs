//! Google Antigravity accounts to the Slint rows (PLAN.md D23).

use fast_harness_core::accounts::google::{
    GoogleHold, GoogleQuota, GoogleStatus, HoldReason, QuotaWindow,
};
use slint::{ModelRc, SharedString, VecModel};

use crate::format::{ago, countdown, fraction, window_caption};
use crate::rows::account_label;
use crate::{GoogleQuotaRow, GoogleRow};

/// Bar fill and caption of one quota window: `42%, resets in 2h 05m`, just
/// `42%` without a reset, `disabled` or `no data`.
pub fn quota_cell(window: Option<&QuotaWindow>, now_ms: u64) -> (f32, SharedString) {
    let Some(w) = window else {
        return (0.0, "no data".into());
    };
    if w.disabled {
        return (0.0, "disabled".into());
    }
    let used = w.used_at(now_ms) * 100.0;
    let reset = w.resets_at.and_then(|at| i64::try_from(at).ok());
    let now = i64::try_from(now_ms).unwrap_or(i64::MAX);
    (fraction(used), window_caption(used, reset, now).into())
}

/// One row per quota group, in Google's order.
pub fn quota_rows(quota: Option<&GoogleQuota>, now_ms: u64) -> Vec<GoogleQuotaRow> {
    let Some(quota) = quota else {
        return Vec::new();
    };
    quota
        .groups
        .iter()
        .map(|group| {
            let five = quota_cell(group.five_hour.as_ref(), now_ms);
            let week = quota_cell(group.weekly.as_ref(), now_ms);
            GoogleQuotaRow {
                name: group.name.as_str().into(),
                five: five.0,
                five_text: five.1,
                week: week.0,
                week_text: week.1,
            }
        })
        .collect()
}

/// `Held for 2h 05m: rate limit`, `Needs verification in the browser`.
pub fn hold_text(hold: &GoogleHold, now_ms: u64) -> String {
    let left = countdown(
        i64::try_from(hold.until).unwrap_or(i64::MAX),
        i64::try_from(now_ms).unwrap_or(i64::MAX),
    );
    match hold.reason {
        HoldReason::RateLimit => format!("Held for {left}: rate limit"),
        HoldReason::Validation => "Needs verification in the browser".into(),
        HoldReason::Refused => format!("Held for {left}: refused by Google"),
    }
}

/// `Quota read 5 min ago`, or `Quota not read yet`.
pub fn read_text(read_at: Option<u64>, now_ms: u64) -> String {
    match read_at {
        Some(at) => format!("Quota read {}", ago(at, now_ms)),
        None => "Quota not read yet".into(),
    }
}

/// The card of one Google account; `project` comes from the store.
pub fn google_row(status: &GoogleStatus, project: Option<String>, now_ms: u64) -> GoogleRow {
    let hold = status.hold.as_ref();
    let verify = hold
        .filter(|h| h.reason == HoldReason::Validation)
        .and_then(|h| h.url.clone());
    GoogleRow {
        id: status.account.id.as_str().into(),
        name: account_label(&status.account).into(),
        plan: status.account.plan.clone().unwrap_or_default().into(),
        project: project.unwrap_or_default().into(),
        active: status.active,
        quota: ModelRc::new(VecModel::from(quota_rows(status.quota.as_ref(), now_ms))),
        hold: hold
            .map(|h| hold_text(h, now_ms))
            .unwrap_or_default()
            .into(),
        verify_url: verify.unwrap_or_default().into(),
        read: read_text(status.read_at, now_ms).into(),
        error: status.last_error.clone().unwrap_or_default().into(),
    }
}

#[cfg(test)]
#[path = "google_rows_tests.rs"]
mod tests;
