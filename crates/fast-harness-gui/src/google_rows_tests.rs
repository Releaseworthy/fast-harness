use super::*;
use crate::google_fixtures::{statuses, MIN};
use slint::Model;

const NOW: u64 = 1_800_000_000_000;

#[test]
fn quota_cells() {
    let window = |used, resets_at, disabled| QuotaWindow {
        used,
        resets_at,
        disabled,
    };
    let reset = window(0.42, Some(NOW + 125 * MIN), false);
    assert_eq!(quota_cell(Some(&reset), NOW).1, "42%, resets in 2h 05m");
    assert!((quota_cell(Some(&reset), NOW).0 - 0.42).abs() < 1e-6);
    // No reset: the percent alone.
    assert_eq!(quota_cell(Some(&window(0.1, None, false)), NOW).1, "10%");
    // A passed reset reads as unused.
    let passed = window(0.9, Some(NOW - MIN), false);
    assert_eq!(
        quota_cell(Some(&passed), NOW),
        (0.0, "0%, resets now".into())
    );
    assert_eq!(
        quota_cell(Some(&window(0.5, None, true)), NOW).1,
        "disabled"
    );
    assert_eq!(quota_cell(None, NOW).1, "no data");
}

#[test]
fn holds_and_reads() {
    let hold = |reason| GoogleHold {
        until: NOW + 90 * MIN,
        reason,
        url: None,
    };
    assert_eq!(
        hold_text(&hold(HoldReason::RateLimit), NOW),
        "Held for 1h 30m: rate limit"
    );
    assert_eq!(
        hold_text(&hold(HoldReason::Refused), NOW),
        "Held for 1h 30m: refused by Google"
    );
    assert_eq!(
        hold_text(&hold(HoldReason::Validation), NOW),
        "Needs verification in the browser"
    );
    assert_eq!(read_text(Some(NOW - 5 * MIN), NOW), "Quota read 5 min ago");
    assert_eq!(read_text(None, NOW), "Quota not read yet");
}

#[test]
fn google_rows() {
    let list = statuses(NOW);
    let active = google_row(&list[0], Some("proj-1".into()), NOW);
    assert_eq!(active.name, "ana@example.com");
    assert_eq!(
        (active.plan.as_str(), active.project.as_str()),
        ("Pro", "proj-1")
    );
    assert!(active.active && active.hold.is_empty() && active.verify_url.is_empty());
    assert_eq!(active.quota.row_count(), 2);
    let claude = active.quota.row_data(1).unwrap();
    assert_eq!(claude.name, "Claude and GPT models");
    assert_eq!(claude.five_text, "80%, resets in 40m");
    assert_eq!(claude.week_text, "55%, resets in 3d 0h");
    let held = google_row(&list[1], None, NOW);
    assert_eq!(held.hold, "Needs verification in the browser");
    assert_eq!(held.verify_url, "https://accounts.example.com/verify");
    assert_eq!(held.quota.row_count(), 0);
    assert_eq!(held.read, "Quota not read yet");
    assert!(held.error.contains("VALIDATION_REQUIRED"));
}
