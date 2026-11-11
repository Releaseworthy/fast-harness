//! Usage reads, ported from claude-switch's verify.ts (usageOf, planOf,
//! usageFromHeaders, currentUsage, exhaustedUntil and fullnessOf).

use nh_core::accounts::order::LIMIT_HOLD_MS;
use nh_core::accounts::usage::{
    current_usage, exhausted_until, family_of, fullness_of, plan_of, usage_from_endpoint,
    usage_from_headers, windows_for, Usage,
};
use serde_json::{json, Value};

const AT: i64 = 1_792_000_000_000;
const HOUR: i64 = 3_600_000;

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

fn with(mut body: Value, key: &str, value: Value) -> Value {
    body[key] = value;
    body
}

fn pairs(list: &[(&str, &str)]) -> Vec<(String, String)> {
    list.iter()
        .map(|(name, value)| ((*name).to_owned(), (*value).to_owned()))
        .collect()
}

#[test]
fn iso_helper_matches_to_iso_string() {
    assert_eq!(iso(1_792_602_600_000), "2026-10-21T17:10:00.000Z");
    assert_eq!(iso(0), "1970-01-01T00:00:00.000Z");
}

#[test]
fn usage_of_normalizes_the_windows_and_skips_null_model_buckets() {
    let usage = usage_from_endpoint(&usage_body(94.0, 26.0)).expect("a valid body parses");
    assert_eq!(usage.five_hour.as_ref().map(|w| w.utilization), Some(94.0));
    assert_eq!(usage.seven_day.as_ref().map(|w| w.utilization), Some(26.0));
    assert_eq!(
        usage.five_hour.as_ref().and_then(|w| w.resets_at),
        Some(AT + 3 * HOUR)
    );
    assert_eq!(
        usage.extra.len(),
        1,
        "only the non-null model window is kept"
    );
    assert_eq!(usage.extra[0].label, "Sonnet");
    assert_eq!(usage.extra[0].key, "seven_day_sonnet");
    let clamped = usage_from_endpoint(
        &json!({ "five_hour": { "utilization": 140 }, "seven_day": { "utilization": -3 } }),
    )
    .expect("clamped body parses");
    assert_eq!(clamped.five_hour.map(|w| w.utilization), Some(100.0));
    assert_eq!(clamped.seven_day.map(|w| w.utilization), Some(0.0));
    let none = json!({ "five_hour": null, "seven_day": null });
    assert_eq!(usage_from_endpoint(&none), None, "no window at all is None");
    assert_eq!(usage_from_endpoint(&json!("nope")), None);
}

#[test]
fn usage_of_reads_model_scoped_weekly_rows_from_limits() {
    let reset = "2026-10-18T18:00:00+00:00";
    let mut body = with(usage_body(6.0, 93.0), "seven_day_sonnet", Value::Null);
    body["limits"] = json!([
        { "kind": "session", "group": "session", "percent": 6, "resets_at": reset, "scope": null },
        { "kind": "weekly_all", "group": "weekly", "percent": 93, "resets_at": reset, "scope": null },
        { "kind": "weekly_scoped", "group": "weekly", "percent": 0, "resets_at": reset,
          "scope": { "model": { "id": null, "display_name": "Fable" }, "surface": null } }
    ]);
    let usage = usage_from_endpoint(&body).expect("parses");
    assert_eq!(usage.extra.len(), 1, "one model window");
    assert_eq!(usage.extra[0].label, "Fable");
    assert_eq!(usage.extra[0].key, "weekly_scoped:fable");
    assert_eq!(usage.extra[0].utilization, 0.0);
    assert_eq!(usage.extra[0].resets_at, Some(1_792_346_400_000));
    let row = json!([{ "kind": "weekly_scoped", "percent": 12, "scope": { "model": { "display_name": "Sonnet" } } }]);
    let both = usage_from_endpoint(&with(usage_body(1.0, 2.0), "limits", row)).expect("parses");
    assert_eq!(
        both.extra.len(),
        1,
        "a model already read from seven_day_* is not doubled"
    );
}

#[test]
fn usage_of_reads_string_numbers_and_unknown_model_windows() {
    let body = json!({
        "five_hour": { "utilization": " 12.5 ", "resets_at": "2026-10-21T17:10:00.123456+07:00" },
        "seven_day_new_model": { "utilization": 3 }
    });
    let usage = usage_from_endpoint(&body).expect("parses");
    let five = usage.five_hour.expect("five-hour window");
    assert_eq!(five.utilization, 12.5);
    assert_eq!(five.resets_at, Some(1_792_602_600_123 - 7 * HOUR));
    assert_eq!(usage.extra[0].label, "New model");
    let bad = json!({ "five_hour": { "utilization": 1, "resets_at": "soon" } });
    let read = usage_from_endpoint(&bad).and_then(|u| u.five_hour);
    assert_eq!(read.and_then(|w| w.resets_at), None);
}

#[test]
fn plan_of_names_the_team_seat_and_the_max_tier() {
    let plan = |value: Value| plan_of(&value);
    let team5x = json!({ "organization_type": "claude_team", "rate_limit_tier": "default_claude_max_5x", "seat_tier": "team_tier_1" });
    assert_eq!(plan(team5x).as_deref(), Some("Team Premium"));
    let team =
        json!({ "organization_type": "claude_team", "rate_limit_tier": "default_claude_ai" });
    assert_eq!(plan(team).as_deref(), Some("Team Standard"));
    let max =
        json!({ "organization_type": "claude_max", "rate_limit_tier": "default_claude_max_20x" });
    assert_eq!(plan(max).as_deref(), Some("Max 20x"));
    let pro = json!({ "organization_type": "claude_pro", "rate_limit_tier": "default_claude_ai" });
    assert_eq!(plan(pro).as_deref(), Some("Pro"));
    let display = json!({ "organization_type": "claude_team", "plan_display_name": "Team Plus" });
    assert_eq!(plan(display).as_deref(), Some("Team Plus"));
    assert_eq!(plan(Value::Null), None);
    assert_eq!(
        plan(json!({ "organization_type": "claude_free" })).as_deref(),
        Some("Free")
    );
}

#[test]
fn usage_from_headers_reads_an_observed_claude_response() {
    let observed = pairs(&[
        ("anthropic-ratelimit-unified-5h-reset", "1792602600"),
        ("anthropic-ratelimit-unified-5h-status", "allowed_warning"),
        ("anthropic-ratelimit-unified-5h-surpassed-threshold", "0.9"),
        ("Anthropic-Ratelimit-Unified-5h-Utilization", "0.9"),
        ("anthropic-ratelimit-unified-7d-reset", "1792753200"),
        ("anthropic-ratelimit-unified-7d-status", "allowed"),
        ("anthropic-ratelimit-unified-7d-utilization", "0.61"),
        ("anthropic-ratelimit-unified-fallback", "available"),
        ("anthropic-ratelimit-unified-fallback-percentage", "0.5"),
        (
            "anthropic-ratelimit-unified-overage-disabled-reason",
            "org_level_disabled",
        ),
        ("anthropic-ratelimit-unified-overage-status", "rejected"),
        (
            "anthropic-ratelimit-unified-representative-claim",
            "five_hour",
        ),
        ("anthropic-ratelimit-unified-reset", "1792602600"),
        ("anthropic-ratelimit-unified-status", "allowed_warning"),
        ("content-type", "application/json"),
    ]);
    let parsed = usage_from_headers(&observed).expect("parses");
    let five = parsed.five_hour.expect("five-hour window");
    let seven = parsed.seven_day.expect("seven-day window");
    assert_eq!((five.utilization, seven.utilization), (90.0, 61.0));
    assert_eq!(
        five.resets_at.map(iso).as_deref(),
        Some("2026-10-21T17:10:00.000Z")
    );
    assert_eq!(
        seven.resets_at.map(iso).as_deref(),
        Some("2026-10-23T11:00:00.000Z")
    );
    assert_eq!(parsed.status.as_deref(), Some("allowed_warning"));
    assert_eq!(parsed.claim.as_deref(), Some("five_hour"));
    assert_eq!(parsed.reset_at, Some(1_792_602_600_000));
    assert_eq!(usage_from_headers(&[]), None);
    assert_eq!(
        usage_from_headers(&pairs(&[("content-type", "application/json")])),
        None
    );
    let overage = pairs(&[("anthropic-ratelimit-unified-overage-status", "rejected")]);
    assert_eq!(
        usage_from_headers(&overage),
        None,
        "no window and no status is nothing"
    );
}

#[test]
fn usage_from_headers_follows_epoch_ms_and_clamps() {
    let headers = pairs(&[
        ("anthropic-ratelimit-unified-5h-utilization", "1.4"),
        ("anthropic-ratelimit-unified-5h-reset", "1792602600000"),
        ("anthropic-ratelimit-unified-7d-utilization", "0.123456"),
        (
            "anthropic-ratelimit-unified-7d-reset",
            "2026-10-23T11:00:00Z",
        ),
        ("anthropic-ratelimit-unified-status", ""),
    ]);
    let parsed = usage_from_headers(&headers).expect("parses");
    assert_eq!(
        parsed.five_hour.as_ref().map(|w| w.utilization),
        Some(100.0)
    );
    assert_eq!(
        parsed.five_hour.and_then(|w| w.resets_at),
        Some(1_792_602_600_000)
    );
    assert_eq!(
        parsed.seven_day.as_ref().map(|w| w.utilization),
        Some(12.35)
    );
    assert_eq!(
        parsed.seven_day.and_then(|w| w.resets_at),
        Some(1_792_753_200_000)
    );
    assert_eq!(parsed.status, None, "an empty status is no status");
    let status_only = pairs(&[("anthropic-ratelimit-unified-status", "rejected")]);
    let parsed = usage_from_headers(&status_only).expect("a status alone is a read");
    assert_eq!(parsed.status.as_deref(), Some("rejected"));
    let beyond = pairs(&[
        ("anthropic-ratelimit-unified-5h-utilization", "0.5"),
        (
            "anthropic-ratelimit-unified-5h-reset",
            "100000000000000000000",
        ),
    ]);
    assert_eq!(
        usage_from_headers(&beyond),
        None,
        "toISOString throws: the read is dropped"
    );
}

#[test]
fn current_usage_reads_a_window_whose_reset_has_passed_as_empty() {
    let usage = usage_from_endpoint(&usage_body(80.0, 50.0)).expect("parses");
    let later = current_usage(&usage, AT + 3 * HOUR + 1);
    let five = later.five_hour.expect("five-hour window");
    assert_eq!(
        (five.utilization, five.resets_at),
        (0.0, None),
        "the five-hour window rolled over"
    );
    assert_eq!(
        later.seven_day.map(|w| w.utilization),
        Some(50.0),
        "the seven-day has not"
    );
    let now = current_usage(&usage, AT);
    assert_eq!(
        now.five_hour.map(|w| w.utilization),
        Some(80.0),
        "before the reset nothing changes"
    );
}

#[test]
fn exhausted_until_and_fullness_of_read_the_windows_that_bound_the_model() {
    let usage = usage_from_endpoint(&json!({
        "five_hour": { "utilization": 40, "resets_at": iso(AT + HOUR) },
        "seven_day": { "utilization": 70, "resets_at": iso(AT + 2 * 24 * HOUR) },
        "limits": [{ "kind": "weekly_scoped", "scope": { "model": { "display_name": "Fable" } },
                     "percent": 100, "resets_at": iso(AT + 5 * HOUR) }]
    }))
    .expect("parses");
    let usage = Some(&usage);
    assert_eq!(
        exhausted_until(usage, "claude-fable-5-1", AT),
        Some(AT + 5 * HOUR)
    );
    assert_eq!(
        exhausted_until(usage, "claude-opus-5-5", AT),
        None,
        "an Opus request is free"
    );
    assert_eq!(
        fullness_of(usage, "claude-opus-5-5", AT),
        70.0,
        "the 7d 70% is the fullest"
    );
    assert_eq!(
        exhausted_until(usage, "claude-fable-5-1", AT + 6 * HOUR),
        None
    );
    assert_eq!(
        fullness_of(None, "claude-opus-5-5", AT),
        101.0,
        "unread sorts last"
    );
}

#[test]
fn exhausted_until_holds_a_full_window_without_reset_and_fullness_skips_passed() {
    let full = Usage {
        five_hour: usage_from_endpoint(&json!({ "five_hour": { "utilization": 100 } }))
            .and_then(|u| u.five_hour),
        ..Usage::default()
    };
    assert_eq!(
        exhausted_until(Some(&full), "", AT),
        Some(AT + LIMIT_HOLD_MS)
    );
    assert_eq!(
        fullness_of(Some(&full), "", AT),
        100.0,
        "no reset counts as not passed"
    );
    let usage = usage_from_endpoint(&usage_body(80.0, 50.0)).expect("parses");
    assert_eq!(
        fullness_of(Some(&usage), "claude-opus-5-5", AT + 3 * HOUR),
        50.0
    );
    assert_eq!(fullness_of(Some(&Usage::default()), "x", AT), 101.0);
}

#[test]
fn family_and_windows_follow_the_model() {
    assert_eq!(family_of("claude-OPUS-5-5"), Some("opus"));
    assert_eq!(family_of("claude-sonnet-4-5"), Some("sonnet"));
    assert_eq!(family_of("gpt-5"), None);
    let usage = usage_from_endpoint(&usage_body(1.0, 2.0)).expect("parses");
    assert_eq!(windows_for(&usage, "claude-sonnet-4-5").len(), 3);
    assert_eq!(windows_for(&usage, "claude-opus-5-5").len(), 2);
    let text = serde_json::to_string(&usage).expect("serializes");
    let back: Usage = serde_json::from_str(&text).expect("deserializes");
    assert_eq!(back, usage);
}
