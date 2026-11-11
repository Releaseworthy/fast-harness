//! Holds, candidate filtering and claude-switch's switch order (chunk 1D-b).

use crate::accounts::usage::{five_hour_of, fullness_of, seven_day_reset_of, Usage};
use std::cmp::Ordering;

pub const LIMIT_HOLD_MS: i64 = 15 * 60 * 1000;
pub const BROKEN_HOLD_MS: i64 = 60 * 60 * 1000;
pub const SEVEN_DAY_URGENT_MS: i64 = 48 * 60 * 60 * 1000;
pub const RETRY_INTERVAL_MS: i64 = 60 * 1000;

pub struct Hold {
    pub id: String,
    pub until: i64,
    pub reason: String,
}

pub struct Holds {
    pub holds: Vec<Hold>,
}

pub fn switch_order(left: Option<&Usage>, right: Option<&Usage>, model: &str, at: i64) -> Ordering {
    let unread_left = fullness_of(left, model, at) == 101.0;
    let unread_right = fullness_of(right, model, at) == 101.0;

    if unread_left != unread_right {
        return if unread_left {
            Ordering::Greater
        } else {
            Ordering::Less
        };
    }

    let left_reset = seven_day_reset_of(left, at);
    let right_reset = seven_day_reset_of(right, at);

    let left_urgent = left_reset - at < SEVEN_DAY_URGENT_MS;
    let right_urgent = right_reset - at < SEVEN_DAY_URGENT_MS;

    if left_urgent != right_urgent {
        return if left_urgent {
            Ordering::Less
        } else {
            Ordering::Greater
        };
    }

    if left_urgent {
        let percent = |u: Option<&Usage>| {
            u.and_then(|u| u.seven_day.as_ref())
                .map(|w| w.utilization)
                .unwrap_or(101.0)
        };
        let seven = percent(left)
            .partial_cmp(&percent(right))
            .unwrap_or(Ordering::Equal);
        if seven != Ordering::Equal {
            return seven;
        }
    } else if left_reset != right_reset {
        return left_reset.cmp(&right_reset);
    }

    let five = five_hour_of(left, at)
        .partial_cmp(&five_hour_of(right, at))
        .unwrap_or(Ordering::Equal);
    if five != Ordering::Equal {
        return five;
    }

    fullness_of(left, model, at)
        .partial_cmp(&fullness_of(right, model, at))
        .unwrap_or(Ordering::Equal)
}

pub fn held_until(_id: &str, _holds: &Holds, _at: i64) -> Option<i64> {
    None
}

pub fn soonest(_holds: &Holds, _at: i64) -> Option<i64> {
    None
}

pub struct Candidate {
    pub id: String,
    pub plan: Option<String>,
    pub usage: Option<Usage>,
}

pub fn candidates(
    _list: &[Candidate],
    _current: &str,
    _holds: &Holds,
    _model: &str,
    _at: i64,
) -> Vec<String> {
    vec![]
}

pub fn hold_for_limit(_usage: Option<&Usage>, _model: &str, _at: i64) -> i64 {
    _at + LIMIT_HOLD_MS
}
