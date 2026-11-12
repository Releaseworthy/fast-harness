//! Text for the UI: percents, countdowns, durations, token counts, ages.

/// `utilization` (0 to 100) as `42%`; `?` when it is not a number.
pub fn percent(utilization: f64) -> String {
    if !utilization.is_finite() {
        return "?".into();
    }
    format!("{}%", utilization.clamp(0.0, 999.0).round() as u32)
}

/// `utilization` (0 to 100) as a bar fill from 0 to 1.
pub fn fraction(utilization: f64) -> f32 {
    if !utilization.is_finite() {
        return 0.0;
    }
    (utilization / 100.0).clamp(0.0, 1.0) as f32
}

/// Time left until `at_ms` from `now_ms` (Unix milliseconds), rounded up to
/// the minute: `3d 4h`, `2h 05m`, `7m`, or `now` once it has passed.
pub fn countdown(at_ms: i64, now_ms: i64) -> String {
    let left = at_ms.saturating_sub(now_ms);
    if left <= 0 {
        return "now".into();
    }
    let minutes = left.saturating_add(59_999) / 60_000;
    let (days, hours, mins) = (minutes / 1440, minutes / 60 % 24, minutes % 60);
    if days > 0 {
        format!("{days}d {hours}h")
    } else if hours > 0 {
        format!("{hours}h {mins:02}m")
    } else {
        format!("{mins}m")
    }
}

/// `42%, resets in 2h 05m` for a usage window.
pub fn window_caption(utilization: f64, resets_at: Option<i64>, now_ms: i64) -> String {
    let pct = percent(utilization);
    match resets_at {
        Some(at) if at > now_ms => format!("{pct}, resets in {}", countdown(at, now_ms)),
        Some(_) => format!("{pct}, resets now"),
        None => pct,
    }
}

/// `850 ms`, `3.2 s`, `2m 05s`; `950`, `12.3k`, `4.1M` (one home for both
/// front ends, PLAN.md D59).
pub use fast_harness_core::app::words::{duration, tokens};

/// How long ago `then_ms` was: `just now`, `5 min ago`, `3 h ago`, `2 d ago`.
pub fn ago(then_ms: u64, now_ms: u64) -> String {
    let secs = now_ms.saturating_sub(then_ms) / 1000;
    match secs {
        0..=59 => "just now".into(),
        60..=3599 => format!("{} min ago", secs / 60),
        3600..=86_399 => format!("{} h ago", secs / 3600),
        _ => format!("{} d ago", secs / 86_400),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn percent_and_fraction_clamp() {
        assert_eq!(percent(41.6), "42%");
        assert_eq!(percent(-3.0), "0%");
        assert_eq!(percent(f64::NAN), "?");
        assert_eq!(fraction(50.0), 0.5);
        assert_eq!(fraction(140.0), 1.0);
        assert_eq!(fraction(f64::INFINITY), 0.0);
    }

    #[test]
    fn countdown_rounds_up_to_minutes() {
        let now = 1_000_000;
        assert_eq!(countdown(now - 1, now), "now");
        assert_eq!(countdown(now + 1, now), "1m");
        assert_eq!(countdown(now + 7 * 60_000, now), "7m");
        assert_eq!(countdown(now + (2 * 60 + 5) * 60_000, now), "2h 05m");
        assert_eq!(countdown(now + (3 * 1440 + 4 * 60) * 60_000, now), "3d 4h");
        assert_eq!(countdown(i64::MAX, i64::MIN), countdown(i64::MAX, i64::MIN));
    }

    #[test]
    fn captions() {
        let now = 0;
        assert_eq!(
            window_caption(10.0, Some(90 * 60_000), now),
            "10%, resets in 1h 30m"
        );
        assert_eq!(window_caption(10.0, Some(-5), now), "10%, resets now");
        assert_eq!(window_caption(99.5, None, now), "100%");
    }

    #[test]
    fn durations_tokens_and_ages() {
        assert_eq!(duration(850), "850 ms");
        assert_eq!(duration(3_240), "3.2 s");
        assert_eq!(duration(125_000), "2m 05s");
        assert_eq!(tokens(950), "950");
        assert_eq!(tokens(12_345), "12.3k");
        assert_eq!(tokens(4_100_000), "4.1M");
        assert_eq!(ago(0, 30_000), "just now");
        assert_eq!(ago(0, 5 * 60_000), "5 min ago");
        assert_eq!(ago(0, 3 * 3_600_000), "3 h ago");
        assert_eq!(ago(10, 0), "just now");
    }
}
