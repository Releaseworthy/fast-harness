//! Holds, candidate filtering and claude-switch's switch order: an exact port
//! of `switchOrder`, `hold`, the candidate filter of `decideFailover` and the
//! usage readers they rest on (`familyOf`, `windowsFor`, `exhaustedUntil`,
//! `fullnessOf`, `sevenDayResetOf`, `fiveHourOf`) from
//! `plugins/dsh-claude-switch/host.js`. A reset `None` stands for JavaScript's
//! `NaN` (no reset), a seven-day reset `None` for `Infinity`.

use std::cmp::Ordering;
use std::collections::HashMap;

use super::usage::{Usage, Window};

/// Model families, in the order `familyOf` tries them.
const MODEL_FAMILIES: [&str; 5] = ["opus", "sonnet", "fable", "haiku", "mythos"];

/// How long an account that hit a limit with no known reset is passed over.
pub const LIMIT_HOLD_MS: i64 = 15 * 60_000;
/// How long an account whose credentials failed is passed over.
pub const BROKEN_HOLD_MS: i64 = 60 * 60_000;
/// A seven-day window resetting sooner than this is used first: its unused
/// quota is about to be lost.
pub const SEVEN_DAY_URGENT_MS: i64 = 2 * 24 * 60 * 60_000;
/// Owner rule: with every account held, retry every minute (not 5 minutes).
pub const RETRY_INTERVAL_MS: i64 = 60_000;
/// The plan name of a Free account, which cannot serve Claude Code.
pub const FREE_PLAN: &str = "Free";

/// One account passed over until `until` (Unix milliseconds).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Hold {
    pub until: i64,
    pub reason: String,
}

/// The accounts passed over, by account id.
#[derive(Debug, Clone, Default)]
pub struct Holds {
    entries: HashMap<String, Hold>,
}

impl Holds {
    pub fn new() -> Self {
        Self::default()
    }

    /// `hold`: pass an account over until `until`; a longer hold already in
    /// place is kept.
    pub fn hold(&mut self, id: &str, until: i64, reason: &str) {
        if self.entries.get(id).is_some_and(|hold| hold.until >= until) {
            return;
        }
        let reason = reason.to_owned();
        self.entries.insert(id.to_owned(), Hold { until, reason });
    }

    /// The hold on record for an account, expired or not.
    pub fn get(&self, id: &str) -> Option<&Hold> {
        self.entries.get(id)
    }

    /// When an account's hold ends, or `None` when it is not held at `at`.
    pub fn held_until(&self, id: &str, at: i64) -> Option<i64> {
        self.entries
            .get(id)
            .map(|hold| hold.until)
            .filter(|&until| until > at)
    }

    /// The soonest end among holds still running at `at`.
    pub fn soonest(&self, at: i64) -> Option<i64> {
        self.entries
            .values()
            .map(|hold| hold.until)
            .filter(|&until| until > at)
            .min()
    }
}

/// One saved account the switch may install.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Candidate {
    pub id: String,
    /// The plan name (`Free`, `Pro`, `Max 20x`, ...), when known.
    pub plan: Option<String>,
    pub usage: Option<Usage>,
}

/// JavaScript `a - b` as a comparator result (`NaN` counts as equal).
fn compare(left: f64, right: f64) -> Ordering {
    left.partial_cmp(&right).unwrap_or(Ordering::Equal)
}

/// Two seven-day resets, `None` standing for `Infinity`.
fn compare_reset(left: Option<i64>, right: Option<i64>) -> Ordering {
    match (left, right) {
        (Some(left), Some(right)) => left.cmp(&right),
        (Some(_), None) => Ordering::Less,
        (None, Some(_)) => Ordering::Greater,
        (None, None) => Ordering::Equal,
    }
}

fn urgent(reset: Option<i64>, at: i64) -> bool {
    reset.is_some_and(|reset| reset.saturating_sub(at) < SEVEN_DAY_URGENT_MS)
}

fn seven_day_percent(usage: Option<&Usage>) -> f64 {
    usage
        .and_then(|usage| usage.seven_day.as_ref())
        .map_or(101.0, |window| window.utilization)
}

/// `switchOrder`: unread accounts last; then every account whose seven-day
/// window resets within `SEVEN_DAY_URGENT_MS`, the lower seven-day percent
/// first whatever the time left; outside that group the soonest seven-day
/// reset first; then the emptier five-hour window; then the emptier fullest
/// window.
pub fn switch_order(left: Option<&Usage>, right: Option<&Usage>, model: &str, at: i64) -> Ordering {
    let unread = |usage| fullness_of(usage, model, at) == 101.0;
    match (unread(left), unread(right)) {
        (true, false) => return Ordering::Greater,
        (false, true) => return Ordering::Less,
        _ => {}
    }
    let left_reset = seven_day_reset_of(left, at);
    let right_reset = seven_day_reset_of(right, at);
    let left_urgent = urgent(left_reset, at);
    if left_urgent != urgent(right_reset, at) {
        return if left_urgent {
            Ordering::Less
        } else {
            Ordering::Greater
        };
    }
    if left_urgent {
        let seven = compare(seven_day_percent(left), seven_day_percent(right));
        if seven != Ordering::Equal {
            return seven;
        }
    } else if left_reset != right_reset {
        return compare_reset(left_reset, right_reset);
    }
    let five = compare(five_hour_of(left, at), five_hour_of(right, at));
    if five != Ordering::Equal {
        return five;
    }
    compare(fullness_of(left, model, at), fullness_of(right, model, at))
}

/// `rejectedUntil`: the reset of a `rejected` unified status still ahead of `at`.
pub fn rejected_until(usage: Option<&Usage>, at: i64) -> Option<i64> {
    let usage = usage?;
    if usage.status.as_deref() != Some("rejected") {
        return None;
    }
    usage.reset_at.filter(|&reset| reset > at)
}

/// How long to hold the account that just hit a limit: its rejected status
/// reset, else its exhausted windows' reset, else `LIMIT_HOLD_MS` from `at`.
pub fn hold_for_limit(usage: Option<&Usage>, model: &str, at: i64) -> i64 {
    rejected_until(usage, at)
        .or_else(|| exhausted_until(usage, model, at))
        .unwrap_or_else(|| at.saturating_add(LIMIT_HOLD_MS))
}

/// The accounts `decideFailover` tries, in order: every account but the
/// current one that is not held, not rejected, not on the Free plan and not
/// exhausted for `model`, sorted with `switch_order` (stable).
pub fn candidates(
    list: &[Candidate],
    current: &str,
    holds: &Holds,
    model: &str,
    at: i64,
) -> Vec<String> {
    let kept: Vec<&Candidate> = list
        .iter()
        .filter(|c| c.id != current && holds.held_until(&c.id, at).is_none())
        .filter(|c| rejected_until(c.usage.as_ref(), at).is_none())
        .filter(|c| c.plan.as_deref() != Some(FREE_PLAN))
        .filter(|c| exhausted_until(c.usage.as_ref(), model, at).is_none())
        .collect();
    // A stable insertion sort: `slice::sort_by` may panic on a comparator that
    // is not a total order (a NaN utilization), this never does.
    let mut sorted: Vec<&Candidate> = Vec::with_capacity(kept.len());
    for candidate in kept {
        let position = sorted
            .iter()
            .rposition(|placed| {
                switch_order(placed.usage.as_ref(), candidate.usage.as_ref(), model, at)
                    != Ordering::Greater
            })
            .map_or(0, |index| index + 1);
        sorted.insert(position, candidate);
    }
    sorted.into_iter().map(|c| c.id.clone()).collect()
}

/// `familyOf`: the model family a model id names (`claude-opus-5-5` -> `opus`).
pub fn family_of(model: &str) -> Option<&'static str> {
    let id = model.to_lowercase();
    MODEL_FAMILIES
        .into_iter()
        .find(|family| id.contains(family))
}

/// `windowsFor`: five-hour, seven-day, and the model's own weekly windows.
pub fn windows_for<'a>(usage: &'a Usage, model: &str) -> Vec<&'a Window> {
    let mut windows: Vec<&Window> = [&usage.five_hour, &usage.seven_day]
        .into_iter()
        .flatten()
        .collect();
    if let Some(family) = family_of(model) {
        for extra in &usage.extra {
            if format!("{} {}", extra.key, extra.label)
                .to_lowercase()
                .contains(family)
            {
                windows.push(extra);
            }
        }
    }
    windows
}

fn windows_of<'a>(usage: Option<&'a Usage>, model: &str) -> Vec<&'a Window> {
    usage
        .map(|usage| windows_for(usage, model))
        .unwrap_or_default()
}

/// `exhaustedUntil`: the latest future reset among full windows (a full window
/// with no reset counts as `LIMIT_HOLD_MS` from `at`), or `None`.
pub fn exhausted_until(usage: Option<&Usage>, model: &str, at: i64) -> Option<i64> {
    let mut until: Option<i64> = None;
    for window in windows_of(usage, model) {
        if window.utilization < 100.0 {
            continue;
        }
        let end = window
            .resets_at
            .unwrap_or_else(|| at.saturating_add(LIMIT_HOLD_MS));
        if end <= at {
            continue;
        }
        until = Some(until.map_or(end, |until| until.max(end)));
    }
    until
}

/// `fullnessOf`: the fullest window in percent, a passed window counting as
/// empty; 101 when nothing was read.
pub fn fullness_of(usage: Option<&Usage>, model: &str, at: i64) -> f64 {
    let windows = windows_of(usage, model);
    if windows.is_empty() {
        return 101.0;
    }
    windows
        .iter()
        .map(|window| {
            if window.resets_at.is_some_and(|reset| reset <= at) {
                0.0
            } else {
                window.utilization
            }
        })
        .fold(f64::NEG_INFINITY, f64::max)
}

/// `sevenDayResetOf`: the future seven-day reset, `None` for JavaScript's
/// `Infinity` (unread, no reset, or already rolled over).
pub fn seven_day_reset_of(usage: Option<&Usage>, at: i64) -> Option<i64> {
    usage
        .and_then(|usage| usage.seven_day.as_ref())
        .and_then(|window| window.resets_at)
        .filter(|&reset| reset > at)
}

/// `fiveHourOf`: the five-hour percent, 0 once its reset passed, 101 when unread.
pub fn five_hour_of(usage: Option<&Usage>, at: i64) -> f64 {
    match usage.and_then(|usage| usage.five_hour.as_ref()) {
        None => 101.0,
        Some(window) if window.resets_at.is_some_and(|reset| reset <= at) => 0.0,
        Some(window) => window.utilization,
    }
}
