//! The provider summaries the Accounts page and panel open on (PLAN.md D32):
//! per provider the account count and a status in words.

use std::path::Path;

use super::google::{self, GoogleError, GoogleStatus};
use super::model::Account;
use super::pool::AccountStatus;
use super::usage::current_usage;

/// The providers, in the order the Accounts page lists them.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ProviderKey {
    Claude,
    Google,
}

impl ProviderKey {
    pub const ALL: [ProviderKey; 2] = [ProviderKey::Claude, ProviderKey::Google];

    /// The id the GUI routes on.
    pub fn id(self) -> &'static str {
        match self {
            ProviderKey::Claude => "claude",
            ProviderKey::Google => "google",
        }
    }

    pub fn name(self) -> &'static str {
        match self {
            ProviderKey::Claude => "Claude",
            ProviderKey::Google => "Google Antigravity",
        }
    }
}

/// Whether Google sign-in has an OAuth client (D25).
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ClientState {
    /// A client is set; its id (not a secret).
    Ready(String),
    /// No client anywhere: the provider needs setup.
    Missing,
    /// A client file that cannot be read; the reason carries no secret.
    Broken(String),
}

impl ClientState {
    /// Reads the client as Google sign-in would ([`google::load_client`]).
    pub fn of(app_home: &Path) -> ClientState {
        match google::load_client(app_home) {
            Ok(client) => ClientState::Ready(client.id),
            Err(GoogleError::NoClient) => ClientState::Missing,
            Err(err) => ClientState::Broken(err.to_string()),
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
    /// Needs the user: setup missing, client unreadable, quota nearly out.
    Attention,
}

/// One provider card or panel row.
#[derive(Clone, Debug, PartialEq)]
pub struct ProviderSummary {
    pub key: ProviderKey,
    pub count: usize,
    /// `No accounts`, `1 account`, `3 accounts`.
    pub count_text: String,
    /// `Needs setup`, `No accounts`, `No account in use`, `Active: <label>`.
    pub status: String,
    pub tone: Tone,
    /// The lowest quota left of the active account, percent, when known.
    pub left: Option<f64>,
}

/// Below this percent left the summary asks for attention.
const LOW_LEFT: f64 = 10.0;

/// The summaries of both providers, Claude first.
pub fn summaries(
    claude: &[AccountStatus],
    google: &[GoogleStatus],
    client: &ClientState,
    now_ms: u64,
) -> Vec<ProviderSummary> {
    let active_claude = claude.iter().find(|s| s.active);
    let active_google = google.iter().find(|s| s.active);
    let claude_left = active_claude.and_then(|s| claude_left(s, now_ms));
    let google_left = active_google.and_then(|s| google_left(s, now_ms));
    let mut google_card = summary(
        ProviderKey::Google,
        google.len(),
        active_google.map(|s| &s.account),
        google_left,
    );
    match client {
        ClientState::Ready(_) => {}
        ClientState::Missing => attention(&mut google_card, "Needs setup".into()),
        ClientState::Broken(_) => attention(&mut google_card, "OAuth client unreadable".into()),
    }
    vec![
        summary(
            ProviderKey::Claude,
            claude.len(),
            active_claude.map(|s| &s.account),
            claude_left,
        ),
        google_card,
    ]
}

fn attention(card: &mut ProviderSummary, status: String) {
    card.status = status;
    card.tone = Tone::Attention;
}

fn summary(
    key: ProviderKey,
    count: usize,
    active: Option<&Account>,
    left: Option<f64>,
) -> ProviderSummary {
    let (status, tone) = match (count, active) {
        (0, _) => ("No accounts".to_string(), Tone::Empty),
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

/// The lowest percent left over the Google quota windows that are on.
fn google_left(status: &GoogleStatus, now_ms: u64) -> Option<f64> {
    let groups = &status.quota.as_ref()?.groups;
    let windows = groups
        .iter()
        .flat_map(|g| g.five_hour.iter().chain(&g.weekly))
        .filter(|w| !w.disabled);
    lowest(windows.map(|w| (1.0 - w.used_at(now_ms)) * 100.0))
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
