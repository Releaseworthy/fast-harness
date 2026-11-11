//! Google pool quota reads: pacing, 429 back-off, google-usage.json and the order.

mod google_pool_support;

use fast_harness_core::accounts::google::pool_order::{
    google_order, groups_for, hold_until, GoogleCandidate, LIMIT_HOLD_MS,
};
use fast_harness_core::accounts::google::{
    load_usage, parse_quota_summary, save_usage, usage_path, GoogleQuota, QuotaRead,
};
use fast_harness_core::auth::AuthSource;
use fast_harness_core::http::{ScriptedResponse, ScriptedTransport};
use google_pool_support::{add, google_at, home, manifest, quota, reply, HOUR, MINUTE, NOW};
use serde_json::json;
use std::collections::HashMap;

fn reads(n: usize) -> ScriptedTransport {
    let mut list = vec![manifest()];
    list.extend((0..n).map(|_| quota(0.5, "2026-11-01T03:00:00Z", 0.5, "2026-11-01T03:00:00Z")));
    ScriptedTransport::new(list)
}

#[test]
fn idle_accounts_every_ten_minutes_and_after_a_turn() {
    let home = home();
    let a = add(&home, "a", NOW + 24 * HOUR);
    add(&home, "b", NOW + 24 * HOUR);
    home.google.use_account(&a).unwrap();
    let http = reads(8);
    let g = &home.google;
    assert_eq!(g.poll(&http, NOW), 2);
    assert_eq!(g.poll(&http, NOW + 5 * MINUTE), 0);
    g.lease(&ScriptedTransport::new(vec![])).unwrap();
    g.after_turn(NOW + 5 * MINUTE);
    assert_eq!(g.poll(&http, NOW + 5 * MINUTE), 1, "the used account");
    g.after_turn(NOW + 5 * MINUTE + 10_000);
    assert_eq!(
        g.poll(&http, NOW + 5 * MINUTE + 10_000),
        0,
        "30 s after its read"
    );
    assert_eq!(g.poll(&http, NOW + 5 * MINUTE + 30_000), 1);
    assert_eq!(g.poll(&http, NOW + 10 * MINUTE), 1, "only the idle one");
    let read_at: Vec<Option<u64>> = g.status().iter().map(|s| s.read_at).collect();
    assert!(read_at.contains(&Some(NOW + 5 * MINUTE + 30_000)));
    assert!(read_at.contains(&Some(NOW + 10 * MINUTE)));
}

#[test]
fn quota_429_backs_off_five_minutes_doubling_to_an_hour() {
    let home = home();
    add(&home, "a", NOW + 24 * HOUR);
    let limited = || {
        reply(
            429,
            r#"{"error":{"code":429,"status":"RESOURCE_EXHAUSTED"}}"#,
        )
    };
    let mut answers: Vec<ScriptedResponse> = vec![manifest()];
    answers.extend((0..6).map(|_| limited()));
    let http = ScriptedTransport::new(answers);
    let g = &home.google;
    let mut at = NOW;
    assert_eq!(g.poll(&http, at), 1);
    assert!(g.status()[0].last_error.as_deref().unwrap().contains("429"));
    for pause in [5, 10, 20, 40, 60, 60] {
        let pause = pause * MINUTE;
        assert_eq!(g.poll(&http, at + pause - 1), 0, "paused {pause}");
        if pause == 60 * MINUTE && at > NOW + 2 * HOUR {
            break;
        }
        assert_eq!(g.poll(&http, at + pause), 1, "read after {pause}");
        at += pause;
    }
}

#[test]
fn usage_file_round_trip_and_corrupt_file() {
    let home = home();
    add(&home, "a", NOW + 24 * HOUR);
    assert_eq!(home.google.poll(&reads(1), NOW), 1);
    let path = usage_path(&home.dir);
    let text = std::fs::read_to_string(&path).unwrap();
    assert!(text.contains("\"version\": 1"), "{text}");
    let again = google_at(&home.dir, &home.pool);
    let status = again.status();
    assert_eq!(status[0].read_at, Some(NOW));
    assert_eq!(status[0].quota, home.google.status()[0].quota);
    std::fs::write(&path, "{not json").unwrap();
    assert!(google_at(&home.dir, &home.pool).status()[0].quota.is_none());
    std::fs::write(&path, r#"{"version":2,"accounts":{}}"#).unwrap();
    assert!(load_usage(&path).is_empty());
    let quota = sample(0.25);
    let mut map = HashMap::new();
    map.insert("g-1".to_string(), QuotaRead { quota, read_at: 7 });
    save_usage(&path, &map).unwrap();
    assert_eq!(load_usage(&path), map);
}

fn sample(gemini_left: f64) -> GoogleQuota {
    let reply = json!({"groups": [
        {"displayName": "GEMINI MODELS", "buckets": [
            {"window": "5h", "remainingFraction": gemini_left, "resetTime": "2026-11-01T03:00:00Z"},
            {"window": "weekly", "remainingFraction": 0.9, "resetTime": "2026-11-05T00:00:00Z"}]},
        {"displayName": "Claude and GPT models", "buckets": [
            {"window": "5h", "remainingFraction": 0.6, "resetTime": "2026-11-01T02:00:00Z"}]}
    ]});
    parse_quota_summary(&reply).unwrap()
}

fn candidate(id: &str, quota: Option<GoogleQuota>, held_until: Option<u64>) -> GoogleCandidate {
    GoogleCandidate {
        id: id.into(),
        quota,
        held_until,
    }
}

#[test]
fn candidates_free_first_most_left_unread_last() {
    let list = vec![
        candidate("held", Some(sample(0.9)), Some(NOW + 1)),
        candidate("unread", None, None),
        candidate("low", Some(sample(0.2)), None),
        candidate("current", Some(sample(1.0)), None),
        candidate("high", Some(sample(0.8)), Some(NOW - 1)),
        candidate("weekly-tight", Some(sample(0.95)), None),
    ];
    // weekly-tight: 5 h 0.95 left but the weekly window only 0.9.
    let order = google_order(&list, "current", "gemini-3-pro", NOW);
    assert_eq!(order, ["weekly-tight", "high", "low", "unread"]);
    // The Claude group has the same 0.6 left everywhere: store order.
    let order = google_order(&list, "current", "antigravity/claude-sonnet-4-6", NOW);
    assert_eq!(order, ["low", "high", "weekly-tight", "unread"]);
}

#[test]
fn groups_and_hold_fallbacks() {
    let quota = sample(0.0);
    assert_eq!(
        groups_for(&quota, "Gemini-3-Flash")[0].name,
        "GEMINI MODELS"
    );
    assert_eq!(
        groups_for(&quota, "gpt-oss-120b")[0].name,
        "Claude and GPT models"
    );
    assert_eq!(groups_for(&quota, "mystery").len(), 2);
    let reset = NOW + 3 * HOUR;
    assert_eq!(
        hold_until(Some(&quota), "gemini-3-pro", Some(5), NOW),
        reset
    );
    assert_eq!(
        hold_until(Some(&quota), "claude-x", Some(5_000), NOW),
        NOW + 5_000
    );
    assert_eq!(hold_until(None, "claude-x", None, NOW), NOW + LIMIT_HOLD_MS);
}
