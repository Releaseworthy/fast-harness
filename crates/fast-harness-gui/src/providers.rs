//! The provider cards of the Accounts page and the Google OAuth client form
//! (PLAN.md D32, D25).

use std::path::Path;

use fast_harness_core::accounts::google::{self, GoogleStatus};
use fast_harness_core::accounts::pool::AccountStatus;
use fast_harness_core::accounts::providers::{
    summaries, ClientState, ProviderKey, ProviderSummary, Tone,
};
use slint::{ModelRc, VecModel};

use crate::{MainWindow, ProviderCard};

fn tone_word(tone: Tone) -> &'static str {
    match tone {
        Tone::Ready => "ready",
        Tone::Empty => "empty",
        Tone::Attention => "attention",
    }
}

/// The meter caption, or what to do next when no quota is known.
fn left_text(summary: &ProviderSummary, client: &ClientState) -> String {
    let google = summary.key == ProviderKey::Google;
    match summary.left {
        Some(left) => format!("{left:.0}% left at the lowest"),
        None if google && *client == ClientState::Missing => {
            "Add an OAuth client, then sign in".into()
        }
        None if google && matches!(client, ClientState::Broken(_)) => {
            "Save the OAuth client again".into()
        }
        None if summary.count == 0 && google => "Sign in with a Google account".into(),
        None if summary.count == 0 => "Sign in or add an API key".into(),
        None => "Quota not read yet".into(),
    }
}

/// One card of the grid.
pub fn provider_card(summary: &ProviderSummary, client: &ClientState) -> ProviderCard {
    let left = summary.left.unwrap_or(0.0);
    ProviderCard {
        id: summary.key.id().into(),
        name: summary.key.name().into(),
        count: summary.count_text.as_str().into(),
        status: summary.status.as_str().into(),
        tone: tone_word(summary.tone).into(),
        has_left: summary.left.is_some(),
        left: (left / 100.0) as f32,
        left_text: left_text(summary, client).into(),
    }
}

/// The cards and the OAuth client state of the Google page.
pub fn fill(
    ui: &MainWindow,
    home: &Path,
    claude: &[AccountStatus],
    google: &[GoogleStatus],
    now_ms: u64,
) {
    let client = ClientState::of(home);
    let cards: Vec<_> = summaries(claude, google, &client, now_ms)
        .iter()
        .map(|s| provider_card(s, &client))
        .collect();
    ui.set_providers(ModelRc::new(VecModel::from(cards)));
    let id = match &client {
        ClientState::Ready(id) => id.as_str(),
        _ => "",
    };
    ui.set_google_client_ready(matches!(client, ClientState::Ready(_)));
    ui.set_google_client_id(id.into());
}

/// Saves the client (secret encrypted) and says how it went: `(error, words)`.
/// The words never carry the secret.
pub fn save_client(home: &Path, id: &str, secret: &str) -> (bool, String) {
    match google::save_client(home, id, secret) {
        Ok(()) => (
            false,
            format!(
                "Saved the OAuth client {}. Add Google account can sign in now.",
                id.trim()
            ),
        ),
        Err(err) => (true, format!("Cannot save the OAuth client: {err}")),
    }
}

/// The form's Save: the result in words, then the cards again.
pub fn save_from_form(app: &crate::app::App, ui: &MainWindow, id: &str, secret: &str) {
    let (error, note) = save_client(&app.rt.home, id, secret);
    ui.set_google_client_error(error);
    ui.set_google_client_note(note.into());
    crate::accounts::refresh(app, ui);
}

#[cfg(test)]
#[path = "providers_tests.rs"]
mod tests;
