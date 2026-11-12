//! The provider summaries the Accounts page and panel open on (PLAN.md D32):
//! per provider the account count and a status in words.

use super::model::Account;
use super::pool::AccountStatus;
use super::usage::current_usage;

/// The providers, in the order the Accounts page lists them.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ProviderKey {
    Claude,
}

impl ProviderKey {
    pub const ALL: [ProviderKey; 1] = [ProviderKey::Claude];

    /// The id the GUI routes on.
    pub fn id(self) -> &'static str {
        match self {
            ProviderKey::Claude => "claude",
        }
    }

    pub fn name(self) -> &'static str {
        match self {
            ProviderKey::Claude => "Claude",
        }
    }
}

/// How a summary reads; the UIs pair its colour with the status words.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Tone {
    /// An account is in use.
    Ready,
    /// Nothing to use yet (no account, none active).
    Empty,
    /// Needs the user: quota nearly out.
    Attention,
}

/// One provider card or panel row.
#[derive(Clone, Debug, PartialEq)]
pub struct ProviderSummary {
    pub key: ProviderKey,
    pub count: usize,
    /// `No accounts`, `1 account`, `3 accounts`.
    pub count_text: String,
    /// `Not set up`, `No account in use`, `Active: <label>`.
    pub status: String,
    pub tone: Tone,
    /// The lowest quota left of the active account, percent, when known.
    pub left: Option<f64>,
}

/// Below this percent left the summary asks for attention.
const LOW_LEFT: f64 = 10.0;

/// The provider summaries (only Claude on this branch: Google Antigravity
/// moved to branch `antigravity`, PLAN.md D53).
pub fn summaries(claude: &[AccountStatus], now_ms: u64) -> Vec<ProviderSummary> {
    let active_claude = claude.iter().find(|s| s.active);
    let claude_left = active_claude.and_then(|s| claude_left(s, now_ms));
    vec![summary(
        ProviderKey::Claude,
        claude.len(),
        active_claude.map(|s| &s.account),
        claude_left,
    )]
}

fn summary(
    key: ProviderKey,
    count: usize,
    active: Option<&Account>,
    left: Option<f64>,
) -> ProviderSummary {
    let (status, tone) = match (count, active) {
        // The count line already says `No accounts`: the status says what to do.
        (0, _) => ("Not set up".to_string(), Tone::Empty),
        (_, None) => ("No account in use".to_string(), Tone::Empty),
        (_, Some(account)) if left.is_some_and(|l| l < LOW_LEFT) => {
            (format!("Active: {}", label(account)), Tone::Attention)
        }
        (_, Some(account)) => (format!("Active: {}", label(account)), Tone::Ready),
    };
    ProviderSummary {
        key,
        count,
        count_text: count_text(count),
        status,
        tone,
        left,
    }
}

/// `No accounts`, `1 account`, `3 accounts`.
pub fn count_text(count: usize) -> String {
    match count {
        0 => "No accounts".into(),
        1 => "1 account".into(),
        n => format!("{n} accounts"),
    }
}

/// The email, else the name, else the id.
fn label(account: &Account) -> String {
    account
        .email
        .clone()
        .or_else(|| account.name.clone())
        .unwrap_or_else(|| account.id.clone())
}

/// The lowest percent left over the Claude windows (reset ones read empty).
fn claude_left(status: &AccountStatus, now_ms: u64) -> Option<f64> {
    let now = i64::try_from(now_ms).unwrap_or(i64::MAX);
    let usage = current_usage(status.usage.as_ref()?, now);
    let windows = usage
        .five_hour
        .iter()
        .chain(&usage.seven_day)
        .chain(&usage.extra);
    lowest(windows.map(|w| 100.0 - w.utilization))
}

fn lowest(values: impl Iterator<Item = f64>) -> Option<f64> {
    values
        .filter(|v| v.is_finite())
        .map(|v| v.clamp(0.0, 100.0))
        .reduce(f64::min)
}

#[cfg(test)]
#[path = "providers_tests.rs"]
mod tests;
