//! The Google switch order and hold length (PLAN.md D23): pure functions
//! over the last quota read of each account.

use super::quota::{GoogleQuota, QuotaGroup, QuotaWindow};
use std::cmp::Ordering;

/// Hold after a rate limit when neither the quota nor the error names a reset.
pub const LIMIT_HOLD_MS: u64 = 15 * 60_000;

/// A window with less than this fraction left counts as exhausted.
const EXHAUSTED: f64 = 0.99995;

/// One Google account as the order sees it.
#[derive(Clone, Debug, PartialEq)]
pub struct GoogleCandidate {
    pub id: String,
    /// The last quota read; `None` when the account was never read.
    pub quota: Option<GoogleQuota>,
    /// Unix milliseconds when a running hold ends.
    pub held_until: Option<u64>,
}

/// The model id without `antigravity/`, lower case.
fn bare(model: &str) -> String {
    let model = model.trim().to_ascii_lowercase();
    match model.strip_prefix("antigravity/") {
        Some(rest) => rest.to_string(),
        None => model,
    }
}

/// The quota groups of `model`: names containing "gemini" for `gemini-*`,
/// "claude" or "gpt" for `claude-*` and `gpt-*` (case-insensitive). An
/// unknown model, or no group of that name, falls back to every group.
pub fn groups_for<'a>(quota: &'a GoogleQuota, model: &str) -> Vec<&'a QuotaGroup> {
    let model = bare(model);
    let words: &[&str] = if model.starts_with("gemini") {
        &["gemini"]
    } else if model.starts_with("claude") || model.starts_with("gpt") {
        &["claude", "gpt"]
    } else {
        &[]
    };
    let matched: Vec<&QuotaGroup> = quota
        .groups
        .iter()
        .filter(|g| {
            let name = g.name.to_lowercase();
            words.iter().any(|w| name.contains(w))
        })
        .collect();
    if matched.is_empty() {
        quota.groups.iter().collect()
    } else {
        matched
    }
}

/// The enabled windows of a group (5 h, then weekly).
fn windows(group: &QuotaGroup) -> impl Iterator<Item = &QuotaWindow> {
    [group.five_hour.as_ref(), group.weekly.as_ref()]
        .into_iter()
        .flatten()
        .filter(|w| !w.disabled)
}

/// The fraction left for `model`: the tighter of the 5 h and weekly windows
/// of its groups (1 without any window).
pub fn remaining(quota: &GoogleQuota, model: &str, now: u64) -> f64 {
    groups_for(quota, model)
        .into_iter()
        .flat_map(windows)
        .map(|w| 1.0 - w.used_at(now))
        .fold(1.0, f64::min)
}

/// When `model` can run again: in each of its groups, the latest reset of an
/// exhausted window; the soonest of those over the groups. `None` when no
/// window of its groups is exhausted with a known reset.
pub fn exhausted_until(quota: &GoogleQuota, model: &str, now: u64) -> Option<u64> {
    groups_for(quota, model)
        .into_iter()
        .filter_map(|group| {
            windows(group)
                .filter(|w| w.used_at(now) >= EXHAUSTED)
                .filter_map(|w| w.resets_at.filter(|at| *at > now))
                .max()
        })
        .min()
}

/// The end of a rate-limit hold: the exhausted window's reset, else the
/// error's retry delay, else [`LIMIT_HOLD_MS`].
pub fn hold_until(
    quota: Option<&GoogleQuota>,
    model: &str,
    retry_after_ms: Option<u64>,
    now: u64,
) -> u64 {
    quota
        .and_then(|q| exhausted_until(q, model, now))
        .or_else(|| retry_after_ms.map(|ms| now.saturating_add(ms.max(1))))
        .unwrap_or_else(|| now.saturating_add(LIMIT_HOLD_MS))
}

/// The accounts to switch to, best first: never `current` or a held account;
/// the most fraction left for `model` first; unread accounts last; ties keep
/// the store order.
pub fn google_order(list: &[GoogleCandidate], current: &str, model: &str, now: u64) -> Vec<String> {
    let mut free: Vec<(&GoogleCandidate, Option<f64>)> = list
        .iter()
        .filter(|c| c.id != current)
        .filter(|c| c.held_until.is_none_or(|until| until <= now))
        .map(|c| (c, c.quota.as_ref().map(|q| remaining(q, model, now))))
        .collect();
    free.sort_by(|(_, left), (_, right)| match (left, right) {
        (Some(l), Some(r)) => r.total_cmp(l),
        (Some(_), None) => Ordering::Less,
        (None, Some(_)) => Ordering::Greater,
        (None, None) => Ordering::Equal,
    });
    free.into_iter().map(|(c, _)| c.id.clone()).collect()
}
