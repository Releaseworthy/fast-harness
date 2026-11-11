//! Usage windows from response headers and the usage endpoint (chunk 1D-b).

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Window {
    pub key: String,
    pub label: String,
    pub utilization: f64,
    pub resets_at: Option<i64>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Usage {
    pub five_hour: Option<Window>,
    pub seven_day: Option<Window>,
    pub extra: Vec<Window>,
    pub status: Option<String>,
    pub claim: Option<String>,
    pub reset_at: Option<i64>,
}

const UNIFIED_PREFIX: &str = "anthropic-ratelimit-unified-";

pub fn usage_from_headers(headers: &[(String, String)]) -> Option<Usage> {
    let mut five_hour = None;
    let mut seven_day = None;
    let mut status = None;
    let mut claim = None;
    let mut reset_at = None;

    for (name, value) in headers {
        let name_lower = name.to_lowercase();
        if !name_lower.starts_with(UNIFIED_PREFIX) {
            continue;
        }

        let suffix = &name_lower[UNIFIED_PREFIX.len()..];
        if suffix == "5h-utilization" {
            if let Ok(util) = value.parse::<f64>() {
                let resets = headers
                    .iter()
                    .find(|(n, _)| n.to_lowercase() == format!("{}5h-reset", UNIFIED_PREFIX))
                    .and_then(|(_, v)| v.parse::<i64>().ok());
                five_hour = Some(Window {
                    key: "five_hour".to_string(),
                    label: "Five Hour".to_string(),
                    utilization: (util * 100.0).clamp(0.0, 100.0),
                    resets_at: resets,
                });
            }
        } else if suffix == "7d-utilization" {
            if let Ok(util) = value.parse::<f64>() {
                let resets = headers
                    .iter()
                    .find(|(n, _)| n.to_lowercase() == format!("{}7d-reset", UNIFIED_PREFIX))
                    .and_then(|(_, v)| v.parse::<i64>().ok());
                seven_day = Some(Window {
                    key: "seven_day".to_string(),
                    label: "Seven Day".to_string(),
                    utilization: (util * 100.0).clamp(0.0, 100.0),
                    resets_at: resets,
                });
            }
        } else if suffix == "status" {
            status = Some(value.clone());
        } else if suffix == "representative-claim" {
            claim = Some(value.clone());
        } else if suffix == "reset" {
            reset_at = value.parse::<i64>().ok();
        }
    }

    if five_hour.is_none() && seven_day.is_none() && status.is_none() {
        return None;
    }

    Some(Usage {
        five_hour,
        seven_day,
        extra: vec![],
        status,
        claim,
        reset_at,
    })
}

pub fn usage_from_endpoint(_body: &serde_json::Value) -> Option<Usage> {
    // TODO: implement
    None
}

pub fn current_usage(usage: &Usage, at: i64) -> Usage {
    let mut usage = usage.clone();
    let roll = |w: &Option<Window>| {
        if let Some(window) = w {
            if let Some(resets_at) = window.resets_at {
                if resets_at <= at {
                    return Some(Window {
                        utilization: 0.0,
                        resets_at: None,
                        ..window.clone()
                    });
                }
            }
        }
        w.clone()
    };
    usage.five_hour = roll(&usage.five_hour);
    usage.seven_day = roll(&usage.seven_day);
    usage.extra = usage
        .extra
        .iter()
        .map(|w| roll(&Some(w.clone())).unwrap())
        .collect();
    usage
}

pub fn family_of(_model: &str) -> Option<String> {
    None
}

pub fn windows_for(usage: &Usage, _model: &str) -> Vec<Window> {
    let mut windows = vec![];
    if let Some(ref w) = usage.five_hour {
        windows.push(w.clone());
    }
    if let Some(ref w) = usage.seven_day {
        windows.push(w.clone());
    }
    windows
}

pub fn exhausted_until(usage: Option<&Usage>, model: &str, at: i64) -> Option<i64> {
    let usage = usage?;
    let mut until = None;
    for window in windows_for(usage, model) {
        if window.utilization < 100.0 {
            continue;
        }
        let end = window.resets_at.unwrap_or(at + 15 * 60 * 1000);
        if end <= at {
            continue;
        }
        until = Some(until.map_or(end, |u: i64| u.max(end)));
    }
    until
}

pub fn fullness_of(usage: Option<&Usage>, model: &str, at: i64) -> f64 {
    let usage = usage.expect("No usage provided"); // Should handle properly
    let windows = windows_for(usage, model);
    if windows.is_empty() {
        return 101.0;
    }
    windows
        .iter()
        .map(|w| {
            if w.resets_at.unwrap_or(0) <= at {
                0.0
            } else {
                w.utilization
            }
        })
        .fold(0.0, |a, b| a.max(b))
}

pub fn seven_day_reset_of(usage: Option<&Usage>, at: i64) -> i64 {
    usage
        .and_then(|u| u.seven_day.as_ref())
        .and_then(|w| w.resets_at)
        .filter(|&r| r > at)
        .unwrap_or(i64::MAX)
}

pub fn five_hour_of(usage: Option<&Usage>, at: i64) -> f64 {
    usage
        .and_then(|u| u.five_hour.as_ref())
        .map(|w| {
            if w.resets_at.unwrap_or(0) <= at {
                0.0
            } else {
                w.utilization
            }
        })
        .unwrap_or(101.0)
}
