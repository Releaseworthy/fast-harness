//! Usage windows from response headers and from the usage endpoint: an exact
//! port of claude-switch's `usageFromHeaders`, `usageOf`, `planOf` and
//! `currentUsage` (`plugins/dsh-claude-switch/host.js`). The readers the switch
//! uses (`exhaustedUntil`, `fullnessOf`, ...) live in `order` and are
//! re-exported here.
//!
//! Differences forced by the types: a reset is kept as Unix milliseconds
//! instead of an ISO string, so `None` stands for JavaScript's `NaN` reset (no
//! reset, or a string `Date.parse` rejects). `Date.parse` is ported for the
//! ECMAScript date-time format (any number of fraction digits, `T` or a space);
//! a date-time without an offset reads as UTC where JavaScript reads local time.

use serde::{Deserialize, Serialize};
use serde_json::Value;

pub use super::order::{
    exhausted_until, family_of, five_hour_of, fullness_of, seven_day_reset_of, windows_for,
};

/// The prefix of every unified rate-limit response header.
pub const UNIFIED_PREFIX: &str = "anthropic-ratelimit-unified-";

/// `organization_type` -> base plan name.
const PLAN_LABELS: [(&str, &str); 5] = [
    ("claude_free", "Free"),
    ("claude_max", "Max"),
    ("claude_pro", "Pro"),
    ("claude_team", "Team"),
    ("claude_enterprise", "Enterprise"),
];

/// Display names of the known `seven_day_*` model windows.
const WINDOW_LABELS: [(&str, &str); 4] = [
    ("seven_day_opus", "Opus"),
    ("seven_day_sonnet", "Sonnet"),
    ("seven_day_fable", "Fable"),
    ("seven_day_cowork", "Cowork"),
];

/// The largest absolute time a JavaScript `Date` holds, in milliseconds.
const MAX_TIME_MS: f64 = 8.64e15;

const DAY_MS: i64 = 86_400_000;

/// One usage window.
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct Window {
    /// `five_hour`, `seven_day`, `seven_day_<model>` or `weekly_scoped:<model>`.
    pub key: String,
    /// Display name.
    pub label: String,
    /// Percent used, 0 to 100.
    pub utilization: f64,
    /// When the window resets, Unix milliseconds.
    pub resets_at: Option<i64>,
}

/// The usage one account reports.
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct Usage {
    pub five_hour: Option<Window>,
    pub seven_day: Option<Window>,
    /// Model windows (`seven_day_sonnet`, Fable rows from `limits`).
    pub extra: Vec<Window>,
    /// `anthropic-ratelimit-unified-status` (`allowed`, `allowed_warning`, `rejected`).
    pub status: Option<String>,
    /// `anthropic-ratelimit-unified-representative-claim`.
    pub claim: Option<String>,
    /// `anthropic-ratelimit-unified-reset`, Unix milliseconds.
    pub reset_at: Option<i64>,
}

/// JavaScript white space and line terminators (what `String.prototype.trim` drops).
fn is_js_space(c: char) -> bool {
    c == '\u{feff}' || (c.is_whitespace() && c != '\u{85}')
}

/// `text`: a non-empty string, or `None`.
fn text(value: Option<&Value>) -> Option<&str> {
    match value {
        Some(Value::String(s)) if !s.is_empty() => Some(s),
        _ => None,
    }
}

/// `number`: a finite number (a string goes through `Number`), or `None`.
fn number(value: Option<&Value>) -> Option<f64> {
    let parsed = match value? {
        Value::Number(n) => n.as_f64()?,
        Value::String(s) => js_number(s)?,
        _ => return None,
    };
    parsed.is_finite().then_some(parsed)
}

/// JavaScript `Number(string)`; `None` stands for `NaN`.
fn js_number(raw: &str) -> Option<f64> {
    let trimmed = raw.trim_matches(is_js_space);
    if trimmed.is_empty() {
        return Some(0.0);
    }
    for (prefix, radix) in [
        ("0x", 16),
        ("0X", 16),
        ("0o", 8),
        ("0O", 8),
        ("0b", 2),
        ("0B", 2),
    ] {
        if let Some(digits) = trimmed.strip_prefix(prefix) {
            if digits.is_empty() {
                return None;
            }
            return digits.chars().try_fold(0.0, |acc: f64, c| {
                c.to_digit(radix)
                    .map(|d| acc * f64::from(radix) + f64::from(d))
            });
        }
    }
    match trimmed {
        "Infinity" | "+Infinity" => return Some(f64::INFINITY),
        "-Infinity" => return Some(f64::NEG_INFINITY),
        _ => {}
    }
    let lower = trimmed.to_ascii_lowercase();
    if lower.contains("inf") || lower.contains("nan") {
        return None;
    }
    trimmed.parse::<f64>().ok()
}

/// JavaScript `Math.round`: the nearest integer, a half going up.
fn js_round(value: f64) -> f64 {
    let floor = value.floor();
    if value - floor >= 0.5 {
        floor + 1.0
    } else {
        floor
    }
}

/// `Math.max(0, Math.min(100, value))`.
fn percent(value: f64) -> f64 {
    if value <= 0.0 {
        0.0
    } else {
        value.min(100.0)
    }
}

/// `/^\d+(\.\d+)?$/`.
fn is_decimal(value: &str) -> bool {
    let digits = |s: &str| !s.is_empty() && s.bytes().all(|b| b.is_ascii_digit());
    match value.split_once('.') {
        Some((whole, fraction)) => digits(whole) && digits(fraction),
        None => digits(value),
    }
}

/// `epochMs` for a header value: milliseconds, seconds (below 1e11) or a date
/// string, as epoch milliseconds; `None` when absent, unparseable or not positive.
pub(crate) fn epoch_ms(value: Option<&str>) -> Option<f64> {
    let raw = value?;
    let trimmed = raw.trim_matches(is_js_space);
    if !trimmed.is_empty() && !is_decimal(trimmed) {
        return date_parse(raw).map(|ms| ms as f64);
    }
    let parsed = js_number(raw).filter(|v| v.is_finite())?;
    if parsed <= 0.0 {
        return None;
    }
    Some(if parsed < 1e11 {
        js_round(parsed * 1000.0)
    } else {
        parsed
    })
}

/// `n` ASCII digits at the start of `s`, and the rest.
fn take_digits(s: &str, n: usize) -> Option<(i64, &str)> {
    let head = s.get(..n)?;
    if !head.bytes().all(|b| b.is_ascii_digit()) {
        return None;
    }
    Some((head.parse().ok()?, &s[n..]))
}

fn days_in_month(year: i64, month: i64) -> i64 {
    let leap = (year % 4 == 0 && year % 100 != 0) || year % 400 == 0;
    match month {
        2 if leap => 29,
        2 => 28,
        4 | 6 | 9 | 11 => 30,
        _ => 31,
    }
}

/// Days since 1970-01-01 of a proleptic Gregorian date.
fn days_from_civil(year: i64, month: i64, day: i64) -> i64 {
    let year = if month <= 2 { year - 1 } else { year };
    let era = year.div_euclid(400);
    let yoe = year - era * 400;
    let doy = (153 * ((month + 9) % 12) + 2) / 5 + day - 1;
    era * 146_097 + yoe * 365 + yoe / 4 - yoe / 100 + doy - 719_468
}

/// `YYYY`, `±YYYYYY`, then optional `-MM` and `-DD`: days since the epoch.
fn parse_date(s: &str) -> Option<i64> {
    let (year, mut rest) = match s.as_bytes().first()? {
        b'+' => take_digits(&s[1..], 6)?,
        b'-' => match take_digits(&s[1..], 6)? {
            (0, _) => return None,
            (year, rest) => (-year, rest),
        },
        _ => take_digits(s, 4)?,
    };
    let (mut month, mut day) = (1, 1);
    if let Some(r) = rest.strip_prefix('-') {
        (month, rest) = take_digits(r, 2)?;
        if let Some(r) = rest.strip_prefix('-') {
            (day, rest) = take_digits(r, 2)?;
        }
    }
    let valid = rest.is_empty() && (1..=12).contains(&month);
    if !valid || day < 1 || day > days_in_month(year, month) {
        return None;
    }
    Some(days_from_civil(year, month, day))
}

/// `Z`, `±HH:mm` or nothing (read as UTC): the offset in milliseconds.
fn parse_offset(s: &str) -> Option<i64> {
    match s.as_bytes().first() {
        None => Some(0),
        Some(b'Z') if s.len() == 1 => Some(0),
        Some(&sign @ (b'+' | b'-')) => {
            let (hours, rest) = take_digits(&s[1..], 2)?;
            let (minutes, rest) = take_digits(rest.strip_prefix(':')?, 2)?;
            if !rest.is_empty() || hours > 23 || minutes > 59 {
                return None;
            }
            let value = (hours * 60 + minutes) * 60_000;
            Some(if sign == b'-' { -value } else { value })
        }
        _ => None,
    }
}

/// `HH:mm[:ss[.fff...]]` plus offset: milliseconds after midnight UTC.
fn parse_time(s: &str) -> Option<i64> {
    let (hour, rest) = take_digits(s, 2)?;
    let (minute, mut rest) = take_digits(rest.strip_prefix(':')?, 2)?;
    let (mut second, mut millis) = (0, 0);
    if let Some(r) = rest.strip_prefix(':') {
        (second, rest) = take_digits(r, 2)?;
        if let Some(r) = rest.strip_prefix('.') {
            let len = r.bytes().take_while(u8::is_ascii_digit).count();
            if len == 0 {
                return None;
            }
            millis = format!("{:0<3}", &r[..len.min(3)]).parse().ok()?;
            rest = &r[len..];
        }
    }
    let offset = parse_offset(rest)?;
    let midnight = hour == 24 && minute == 0 && second == 0 && millis == 0;
    if (hour > 23 && !midnight) || minute > 59 || second > 59 {
        return None;
    }
    Some(((hour * 60 + minute) * 60 + second) * 1000 + millis - offset)
}

/// `Date.parse` for the ECMAScript date-time format, in Unix milliseconds.
fn date_parse(raw: &str) -> Option<i64> {
    let s = raw.trim_matches(is_js_space);
    let (date, time) = match s.split_once(['T', ' ']) {
        Some((date, time)) => (date, Some(time)),
        None => (s, None),
    };
    let days = parse_date(date)?;
    let ms = match time {
        Some(time) => parse_time(time)?,
        None => 0,
    };
    let value = days.checked_mul(DAY_MS)?.checked_add(ms)?;
    (value.unsigned_abs() as f64 <= MAX_TIME_MS).then_some(value)
}

/// The value of the last header named `<prefix><suffix>`, names compared lower-cased.
fn header<'a>(headers: &'a [(String, String)], suffix: &str) -> Option<&'a str> {
    let wanted = format!("{UNIFIED_PREFIX}{suffix}");
    headers
        .iter()
        .rev()
        .find(|(name, _)| name.to_lowercase() == wanted)
        .map(|(_, value)| value.as_str())
}

/// One header window. The outer `None` is the JavaScript `RangeError` a reset
/// beyond the `Date` range throws (the whole read is dropped).
fn header_window(
    headers: &[(String, String)],
    name: &str,
    key: &str,
    label: &str,
) -> Option<Option<Window>> {
    let fraction = header(headers, &format!("{name}-utilization"))
        .and_then(js_number)
        .filter(|v| v.is_finite());
    let Some(fraction) = fraction else {
        return Some(None);
    };
    let reset = epoch_ms(header(headers, &format!("{name}-reset")));
    if reset.is_some_and(|ms| ms.abs() > MAX_TIME_MS) {
        return None;
    }
    Some(Some(Window {
        key: key.to_owned(),
        label: label.to_owned(),
        utilization: percent(js_round(fraction * 10_000.0) / 100.0),
        resets_at: reset.map(|ms| ms.trunc() as i64),
    }))
}

/// `usageFromHeaders` over raw response headers (any case; headers outside
/// `anthropic-ratelimit-unified-*` are ignored). Utilization is a fraction, a
/// reset is epoch seconds. `None` when no window and no status is present.
pub fn usage_from_headers(headers: &[(String, String)]) -> Option<Usage> {
    let five_hour = header_window(headers, "5h", "five_hour", "5h")?;
    let seven_day = header_window(headers, "7d", "seven_day", "7d")?;
    let present = |value: Option<&str>| value.filter(|v| !v.is_empty()).map(str::to_owned);
    let status = present(header(headers, "status"));
    if five_hour.is_none() && seven_day.is_none() && status.is_none() {
        return None;
    }
    Some(Usage {
        five_hour,
        seven_day,
        extra: Vec::new(),
        status,
        claim: present(header(headers, "representative-claim")),
        reset_at: epoch_ms(header(headers, "reset")).map(|ms| ms as i64),
    })
}

/// `windowOf`: a `{ utilization, resets_at }` object (utilization in percent).
fn window_of(value: Option<&Value>, key: &str, label: &str) -> Option<Window> {
    let source = value?;
    let utilization = number(source.get("utilization"))?;
    Some(Window {
        key: key.to_owned(),
        label: label.to_owned(),
        utilization: percent(utilization),
        resets_at: text(source.get("resets_at")).and_then(date_parse),
    })
}

/// `foo_bar` -> `Foo bar`.
fn title_case(value: &str) -> String {
    let spaced = value.replace('_', " ");
    let mut chars = spaced.chars();
    match chars.next() {
        Some(first) => first.to_uppercase().chain(chars).collect(),
        None => String::new(),
    }
}

fn window_label(key: &str) -> String {
    match WINDOW_LABELS.iter().find(|(name, _)| *name == key) {
        Some((_, label)) => (*label).to_owned(),
        None => title_case(&key["seven_day_".len()..]),
    }
}

/// A `limits` row of kind `weekly_scoped` not already read from `seven_day_*`.
fn scoped_window(row: &Value, extra: &[Window]) -> Option<Window> {
    if row.get("kind").and_then(Value::as_str) != Some("weekly_scoped") {
        return None;
    }
    let model = row.get("scope").and_then(|scope| scope.get("model"));
    let label = text(model.and_then(|model| model.get("display_name")))?;
    let utilization = number(row.get("percent"))?;
    let lower = label.to_lowercase();
    if extra
        .iter()
        .any(|entry| entry.label.to_lowercase() == lower)
    {
        return None;
    }
    Some(Window {
        key: format!("weekly_scoped:{lower}"),
        label: label.to_owned(),
        utilization: percent(utilization),
        resets_at: text(row.get("resets_at")).and_then(date_parse),
    })
}

/// `usageOf`: the windows a `GET /api/oauth/usage` answer reports, or `None`
/// when it has neither a five-hour nor a seven-day window.
pub fn usage_from_endpoint(body: &Value) -> Option<Usage> {
    let object = body.as_object()?;
    let five_hour = window_of(object.get("five_hour"), "five_hour", "5h");
    let seven_day = window_of(object.get("seven_day"), "seven_day", "7d");
    if five_hour.is_none() && seven_day.is_none() {
        return None;
    }
    let mut extra = Vec::new();
    for (key, value) in object {
        if !key.starts_with("seven_day_") || key == "seven_day_oauth_apps" {
            continue;
        }
        if let Some(window) = window_of(Some(value), key, &window_label(key)) {
            extra.push(window);
        }
    }
    if let Some(Value::Array(rows)) = object.get("limits") {
        for row in rows {
            if let Some(window) = scoped_window(row, &extra) {
                extra.push(window);
            }
        }
    }
    Some(Usage {
        five_hour,
        seven_day,
        extra,
        ..Usage::default()
    })
}

/// `planOf`: the plan name of an `organization` object, or `None`.
pub fn plan_of(organization: &Value) -> Option<String> {
    if let Some(display) = text(organization.get("plan_display_name")) {
        return Some(display.to_owned());
    }
    let kind = text(organization.get("organization_type"))?;
    let base = PLAN_LABELS.iter().find(|(name, _)| *name == kind)?.1;
    let tier = text(organization.get("rate_limit_tier"));
    let plan = match (kind, tier) {
        ("claude_team", Some("default_claude_max_5x")) => "Team Premium",
        ("claude_team", _) => "Team Standard",
        ("claude_max", Some("default_claude_max_20x")) => "Max 20x",
        ("claude_max", Some("default_claude_max_5x")) => "Max 5x",
        _ => base,
    };
    Some(plan.to_owned())
}

/// `currentUsage`: a window whose reset has passed reads empty with no reset.
pub fn current_usage(usage: &Usage, at: i64) -> Usage {
    let roll = |window: &Window| {
        if window.resets_at.is_some_and(|reset| reset <= at) {
            Window {
                utilization: 0.0,
                resets_at: None,
                ..window.clone()
            }
        } else {
            window.clone()
        }
    };
    Usage {
        five_hour: usage.five_hour.as_ref().map(roll),
        seven_day: usage.seven_day.as_ref().map(roll),
        extra: usage.extra.iter().map(roll).collect(),
        ..usage.clone()
    }
}
