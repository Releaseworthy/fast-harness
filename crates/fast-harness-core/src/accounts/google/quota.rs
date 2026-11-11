//! `retrieveUserQuotaSummary`: quota groups ("Gemini Models", "Claude and GPT
//! models", ...) each with a 5 h and a weekly window. Rules from the
//! antigravity-switch plugin: a missing `remainingFraction` is 0 (proto3 JSON
//! drops zero values), and an unused bucket (fraction 1) carries a moving
//! `resetTime` (now plus the window), so it has no reset.

use super::oauth::refusal;
use super::{text, GoogleApi, GoogleError};
use crate::accounts::usage::epoch_ms;
use crate::http::Transport;
use serde_json::{json, Value};

/// A bucket at this fraction left counts as unused (agy prints it from 99.995%).
const FULL: f64 = 0.99995;

/// The quota of one Google account.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct GoogleQuota {
    pub groups: Vec<QuotaGroup>,
}

/// One quota group: the models in it share its windows.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct QuotaGroup {
    /// `displayName`, else `description`, else empty.
    pub name: String,
    pub five_hour: Option<QuotaWindow>,
    pub weekly: Option<QuotaWindow>,
}

/// One window of a group.
#[derive(Clone, Debug, PartialEq)]
pub struct QuotaWindow {
    /// Used fraction, 0 to 1.
    pub used: f64,
    /// Unix milliseconds of a real reset; `None` for an unused bucket.
    pub resets_at: Option<u64>,
    pub disabled: bool,
}

impl QuotaWindow {
    /// The used fraction at `now_ms`: 0 once the reset has passed.
    pub fn used_at(&self, now_ms: u64) -> f64 {
        match self.resets_at {
            Some(at) if at <= now_ms => 0.0,
            _ => self.used,
        }
    }
}

impl GoogleApi {
    /// Reads the quota summary of `project`.
    pub fn retrieve_quota(
        &self,
        http: &dyn Transport,
        access: &str,
        project: &str,
    ) -> Result<GoogleQuota, GoogleError> {
        let body = json!({ "project": project });
        let answer = self
            .hub
            .control_call(http, access, "retrieveUserQuotaSummary", &body, &[])?;
        let reply = answer.json().unwrap_or(Value::Null);
        if answer.status != 200 {
            return Err(refusal(answer.status, &reply));
        }
        parse_quota_summary(&reply).ok_or(GoogleError::Invalid("quota summary"))
    }
}

/// The groups of a `retrieveUserQuotaSummary` reply; `None` without `groups`.
pub fn parse_quota_summary(reply: &Value) -> Option<GoogleQuota> {
    let groups = reply["groups"].as_array()?;
    let groups = groups
        .iter()
        .filter(|g| g.is_object())
        .map(group_of)
        .collect();
    Some(GoogleQuota { groups })
}

fn group_of(group: &Value) -> QuotaGroup {
    let mut out = QuotaGroup {
        name: text(&group["displayName"])
            .or_else(|| text(&group["description"]))
            .unwrap_or_default(),
        ..QuotaGroup::default()
    };
    let buckets = group["buckets"]
        .as_array()
        .map(Vec::as_slice)
        .unwrap_or(&[]);
    for bucket in buckets.iter().filter(|b| b.is_object()) {
        let slot = match bucket_window(bucket) {
            Some(Window::FiveHour) => &mut out.five_hour,
            Some(Window::Weekly) => &mut out.weekly,
            None => continue,
        };
        if slot.is_none() {
            *slot = Some(window_of(bucket));
        }
    }
    out
}

enum Window {
    FiveHour,
    Weekly,
}

/// `window` and `bucketId` words: `5h` or `hour`, `week` or `7d`.
fn bucket_window(bucket: &Value) -> Option<Window> {
    let words = format!(
        "{} {}",
        bucket["window"].as_str().unwrap_or(""),
        bucket["bucketId"].as_str().unwrap_or("")
    )
    .to_lowercase();
    if words.contains("5h") || words.contains("hour") {
        Some(Window::FiveHour)
    } else if words.contains("week") || words.contains("7d") {
        Some(Window::Weekly)
    } else {
        None
    }
}

fn window_of(bucket: &Value) -> QuotaWindow {
    let left = bucket["remainingFraction"]
        .as_f64()
        .filter(|f| f.is_finite())
        .map_or(0.0, |f| f.clamp(0.0, 1.0));
    let reset = match &bucket["resetTime"] {
        Value::String(s) => epoch_ms(Some(s)),
        Value::Number(n) => epoch_ms(Some(&n.to_string())),
        _ => None,
    };
    QuotaWindow {
        used: 1.0 - left,
        resets_at: reset
            .filter(|_| left < FULL)
            .filter(|ms| *ms > 0.0)
            .map(|ms| ms.min(u64::MAX as f64) as u64),
        disabled: bucket["disabled"] == Value::Bool(true),
    }
}
