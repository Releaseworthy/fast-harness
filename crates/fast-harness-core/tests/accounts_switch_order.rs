//! Switch order, holds and candidates, ported from claude-switch's verify.ts
//! (switchOrder, the limit failover cases) and tests/claude-switch-reset.test.ts.

use fast_harness_core::accounts::order::{
    candidates, hold_for_limit, rejected_until, switch_order, Candidate, Holds, BROKEN_HOLD_MS,
    LIMIT_HOLD_MS, RETRY_INTERVAL_MS, SEVEN_DAY_URGENT_MS,
};
use fast_harness_core::accounts::usage::{usage_from_endpoint, Usage, Window};
use serde_json::{json, Value};
use std::cmp::Ordering;

const AT: i64 = 1_792_000_000_000;
const HOUR: i64 = 3_600_000;
const OPUS: &str = "claude-opus-5-5";

/// Unix milliseconds as `Date.prototype.toISOString` writes them.
fn iso(ms: i64) -> String {
    let (days, rem) = (ms.div_euclid(86_400_000), ms.rem_euclid(86_400_000));
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let day = doy - (153 * mp + 2) / 5 + 1;
    let month = if mp < 10 { mp + 3 } else { mp - 9 };
    let year = yoe + era * 400 + i64::from(month <= 2);
    let (h, m, s) = (rem / HOUR, rem / 60_000 % 60, rem / 1000 % 60);
    format!(
        "{year:04}-{month:02}-{day:02}T{h:02}:{m:02}:{s:02}.{:03}Z",
        rem % 1000
    )
}

/// verify.ts `usageBody`.
fn usage_body(five: f64, seven: f64) -> Value {
    json!({
        "five_hour": { "utilization": five, "resets_at": iso(AT + 3 * HOUR) },
        "seven_day": { "utilization": seven, "resets_at": iso(AT + 4 * 24 * HOUR) },
        "seven_day_opus": null,
        "seven_day_sonnet": { "utilization": 12, "resets_at": iso(AT + 4 * 24 * HOUR) },
        "seven_day_oauth_apps": null,
        "extra_usage": { "is_enabled": false }
    })
}

/// A seven-day reset at `AT` plus `ms` (a float, truncated the way `new Date` does).
fn seven_at(mut body: Value, seven: f64, ms: f64) -> Value {
    let reset = (AT as f64 + ms) as i64;
    body["seven_day"] = json!({ "utilization": seven, "resets_at": iso(reset) });
    body
}

/// verify.ts `usageIn`: a seven-day reset `days` from `AT`.
fn usage_in(five: f64, seven: f64, days: f64) -> Value {
    seven_at(usage_body(five, seven), seven, days * 24.0 * HOUR as f64)
}

fn read(body: Value) -> Usage {
    usage_from_endpoint(&body).expect("a usage body parses")
}

fn order(entries: Vec<(&str, Option<Value>)>) -> String {
    let mut list: Vec<(&str, Option<Usage>)> = entries
        .into_iter()
        .map(|(name, body)| (name, body.map(read)))
        .collect();
    list.sort_by(|left, right| switch_order(left.1.as_ref(), right.1.as_ref(), OPUS, AT));
    list.iter()
        .map(|entry| entry.0)
        .collect::<Vec<_>>()
        .join(",")
}

#[test]
fn switch_order_puts_urgent_first_then_soonest_reset_then_five_hour() {
    let got = order(vec![
        ("fresh", Some(usage_in(10.0, 30.0, 3.0))),
        ("soon", Some(usage_in(10.0, 60.0, 2.0 - 1.0 / 24.0))),
    ]);
    assert_eq!(
        got, "soon,fresh",
        "60% resetting in under 2 days beats 30% in 3 days"
    );
    let mixed = || {
        vec![
            ("late", Some(usage_in(0.0, 5.0, 6.0))),
            ("mid", Some(usage_in(50.0, 80.0, 4.0))),
            ("urgent", Some(usage_in(70.0, 90.0, 1.0))),
            ("unread", None),
        ]
    };
    assert_eq!(order(mixed()), "urgent,mid,late,unread");
    let got = order(vec![
        ("busyUrgent", Some(usage_in(10.0, 90.0, 1.0))),
        ("calmUrgent", Some(usage_in(80.0, 20.0, 1.9))),
    ]);
    assert_eq!(
        got, "calmUrgent,busyUrgent",
        "lower 7d % first, the 5h window does not override it"
    );
    assert_eq!(order(mixed()), "urgent,mid,late,unread");
    let got = order(vec![
        ("h15", Some(usage_in(10.0, 70.0, 15.0 / 24.0))),
        ("h26", Some(usage_in(10.0, 40.0, 26.0 / 24.0))),
        ("d3", Some(usage_in(10.0, 5.0, 3.0))),
    ]);
    assert_eq!(
        got, "h26,h15,d3",
        "inside 48 h the lower 7d % goes first whatever the time left"
    );
    let edge = |seconds: f64| seven_at(usage_body(10.0, 50.0), 50.0, seconds * 1000.0);
    let got = order(vec![
        ("out", Some(edge(172_800.0))),
        ("in", Some(edge(172_799.0))),
    ]);
    assert_eq!(
        got, "in,out",
        "172799 s is inside the 48 h group, 172800 s is not"
    );
    let got = order(vec![
        ("busy", Some(usage_in(60.0, 40.0, 3.0))),
        ("idle", Some(usage_in(10.0, 40.0, 3.0))),
    ]);
    assert_eq!(
        got, "idle,busy",
        "same seven-day reset: the emptier five-hour window first"
    );
    let rolled = seven_at(usage_body(20.0, 50.0), 50.0, -(HOUR as f64));
    let got = order(vec![
        ("rolled", Some(rolled)),
        ("week", Some(usage_in(20.0, 50.0, 5.0))),
    ]);
    assert_eq!(
        got, "week,rolled",
        "a seven-day window that already reset waits behind a pending reset"
    );
}

fn window(key: &str, utilization: f64, resets_at: i64) -> Option<Window> {
    let label = key.to_owned();
    Some(Window {
        key: key.to_owned(),
        label,
        utilization,
        resets_at: Some(resets_at),
    })
}

/// tests/claude-switch-reset.test.ts: a usage with both windows.
fn reset_usage(now: i64, five: (f64, i64), seven: (f64, i64)) -> Usage {
    Usage {
        five_hour: window("five_hour", five.0, now + five.1 * HOUR),
        seven_day: window("seven_day", seven.0, now + seven.1 * HOUR),
        ..Usage::default()
    }
}

#[test]
fn owner_rule_seven_day_urgent_is_exactly_48_hours() {
    assert_eq!(SEVEN_DAY_URGENT_MS, 48 * 60 * 60 * 1000);
    assert_eq!(
        RETRY_INTERVAL_MS, 60_000,
        "retry every minute, not every 5 minutes"
    );
    assert_eq!(LIMIT_HOLD_MS, 15 * 60_000);
    assert_eq!(BROKEN_HOLD_MS, 60 * 60_000);
}

#[test]
fn owner_rule_reset_within_48_hours_goes_before_one_outside() {
    let now = 1_730_000_000_000;
    let urgent = reset_usage(now, (80.0, 2), (60.0, 24));
    let calm = reset_usage(now, (20.0, 2), (30.0, 72));
    assert_eq!(
        switch_order(Some(&urgent), Some(&calm), OPUS, now),
        Ordering::Less
    );
}

#[test]
fn owner_rule_within_48_hours_lower_seven_day_percent_wins_whatever_the_five_hour() {
    let now = 1_730_000_000_000;
    let lower = reset_usage(now, (90.0, 1), (25.0, 30));
    let higher = reset_usage(now, (10.0, 1), (75.0, 15));
    assert_eq!(
        switch_order(Some(&lower), Some(&higher), OPUS, now),
        Ordering::Less
    );
}

#[test]
fn holds_keep_the_longer_hold_and_report_the_soonest() {
    let mut holds = Holds::new();
    holds.hold("a", AT + 10, "RATE_LIMIT");
    holds.hold("a", AT + 5, "unusable");
    assert_eq!(
        holds.get("a").map(|h| (h.until, h.reason.as_str())),
        Some((AT + 10, "RATE_LIMIT"))
    );
    holds.hold("a", AT + 20, "unusable");
    assert_eq!(
        holds.get("a").map(|h| (h.until, h.reason.as_str())),
        Some((AT + 20, "unusable"))
    );
    holds.hold("b", AT + 7, "RATE_LIMIT");
    assert_eq!(holds.held_until("a", AT), Some(AT + 20));
    assert_eq!(
        holds.held_until("a", AT + 20),
        None,
        "a hold ending at `at` is over"
    );
    assert_eq!(holds.held_until("c", AT), None);
    assert_eq!(holds.soonest(AT), Some(AT + 7));
    assert_eq!(holds.soonest(AT + 7), Some(AT + 20));
    assert_eq!(holds.soonest(AT + 20), None);
}

fn candidate(id: &str, plan: Option<&str>, usage: Option<Usage>) -> Candidate {
    Candidate {
        id: id.to_owned(),
        plan: plan.map(str::to_owned),
        usage,
    }
}

#[test]
fn candidates_prefer_the_seven_day_reset_within_two_days() {
    let list = vec![
        candidate("a", Some("Max 5x"), Some(read(usage_body(100.0, 50.0)))),
        candidate("b", Some("Max 5x"), Some(read(usage_in(10.0, 30.0, 3.0)))),
        candidate("c", Some("Max 5x"), Some(read(usage_in(40.0, 60.0, 1.5)))),
    ];
    assert_eq!(candidates(&list, "a", &Holds::new(), OPUS, AT), ["c", "b"]);
}

#[test]
fn candidates_prefer_the_most_headroom() {
    let list = vec![
        candidate("a", None, Some(read(usage_body(100.0, 50.0)))),
        candidate("b", None, Some(read(usage_body(60.0, 70.0)))),
        candidate("c", None, Some(read(usage_body(20.0, 30.0)))),
    ];
    assert_eq!(candidates(&list, "a", &Holds::new(), OPUS, AT), ["c", "b"]);
}

fn rejected(reset_at: i64) -> Usage {
    Usage {
        status: Some("rejected".into()),
        reset_at: Some(reset_at),
        ..read(usage_body(10.0, 10.0))
    }
}

#[test]
fn candidates_drop_current_held_free_rejected_and_exhausted_accounts() {
    let mut holds = Holds::new();
    holds.hold("held", AT + HOUR, "RATE_LIMIT");
    holds.hold("expired", AT, "RATE_LIMIT");
    let list = vec![
        candidate("unread", None, None),
        candidate("a", None, Some(read(usage_body(10.0, 10.0)))),
        candidate("held", None, Some(read(usage_body(1.0, 1.0)))),
        candidate("free", Some("Free"), Some(read(usage_body(1.0, 1.0)))),
        candidate("full", None, Some(read(usage_body(100.0, 10.0)))),
        candidate("rejected", None, Some(rejected(AT + HOUR))),
        candidate("lapsed", None, Some(rejected(AT))),
        candidate("expired", None, Some(read(usage_body(30.0, 30.0)))),
        candidate("ok", Some("Pro"), Some(read(usage_body(20.0, 10.0)))),
    ];
    let got = candidates(&list, "a", &holds, OPUS, AT);
    assert_eq!(got, ["lapsed", "ok", "expired", "unread"]);
}

#[test]
fn hold_for_limit_uses_rejected_then_exhausted_then_fifteen_minutes() {
    let full = read(usage_body(100.0, 50.0));
    assert_eq!(hold_for_limit(Some(&full), OPUS, AT), AT + 3 * HOUR);
    let open = read(usage_body(10.0, 10.0));
    assert_eq!(hold_for_limit(Some(&open), OPUS, AT), AT + LIMIT_HOLD_MS);
    assert_eq!(hold_for_limit(None, OPUS, AT), AT + LIMIT_HOLD_MS);
    let limited = rejected(AT + 2 * HOUR);
    assert_eq!(rejected_until(Some(&limited), AT), Some(AT + 2 * HOUR));
    assert_eq!(hold_for_limit(Some(&limited), OPUS, AT), AT + 2 * HOUR);
    assert_eq!(
        rejected_until(Some(&rejected(AT)), AT),
        None,
        "a passed reset holds nothing"
    );
}
