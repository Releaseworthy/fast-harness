//! Two fake Google accounts for the row tests and the shots: one active with
//! quota, one held for verification. No real email or token.

use fast_harness_core::accounts::google::{
    GoogleHold, GoogleQuota, GoogleStatus, HoldReason, QuotaGroup, QuotaWindow,
};
use fast_harness_core::accounts::model::{Account, AccountKind, Provider};

pub const MIN: u64 = 60_000;

fn account(id: &str, email: &str, plan: &str) -> Account {
    Account {
        id: id.into(),
        kind: AccountKind::Own,
        provider: Provider::Google,
        email: Some(email.into()),
        name: None,
        plan: Some(plan.into()),
        added_at: 0,
    }
}

fn window(used: f64, resets_at: Option<u64>) -> Option<QuotaWindow> {
    Some(QuotaWindow {
        used,
        resets_at,
        disabled: false,
    })
}

/// Quota at `now`: Gemini 42 % (5 h, resets in 2h 05m) and 10 % weekly
/// without a reset; Claude and GPT 80 % and 55 %.
pub fn quota(now: u64) -> GoogleQuota {
    GoogleQuota {
        groups: vec![
            QuotaGroup {
                name: "Gemini Models".into(),
                five_hour: window(0.42, Some(now + 125 * MIN)),
                weekly: window(0.10, None),
            },
            QuotaGroup {
                name: "Claude and GPT models".into(),
                five_hour: window(0.80, Some(now + 40 * MIN)),
                weekly: window(0.55, Some(now + 3 * 1440 * MIN)),
            },
        ],
    }
}

pub fn statuses(now: u64) -> Vec<GoogleStatus> {
    vec![
        GoogleStatus {
            account: account("g-1", "ana@example.com", "Pro"),
            active: true,
            hold: None,
            quota: Some(quota(now)),
            read_at: Some(now - 5 * MIN),
            last_error: None,
        },
        GoogleStatus {
            account: account("g-2", "bao@example.com", "Free"),
            active: false,
            hold: Some(GoogleHold {
                until: now + 600 * MIN,
                reason: HoldReason::Validation,
                url: Some("https://accounts.example.com/verify".into()),
            }),
            quota: None,
            read_at: None,
            last_error: Some("Google refused (HTTP 403, VALIDATION_REQUIRED)".into()),
        },
    ]
}
