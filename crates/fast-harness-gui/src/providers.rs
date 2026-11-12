//! The provider cards of the Accounts page (PLAN.md D32). Only Claude is on
//! this branch: the Google Antigravity card moved to branch `antigravity`
//! (D53).

use fast_harness_core::accounts::pool::AccountStatus;
use fast_harness_core::accounts::providers::{summaries, ProviderSummary, Tone};
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
fn left_text(summary: &ProviderSummary) -> String {
    match summary.left {
        Some(left) => format!("{left:.0}% left at the lowest"),
        None if summary.count == 0 => "Sign in or add an API key".into(),
        None => "Quota not read yet".into(),
    }
}

/// One card of the grid.
pub fn provider_card(summary: &ProviderSummary) -> ProviderCard {
    let left = summary.left.unwrap_or(0.0);
    ProviderCard {
        id: summary.key.id().into(),
        name: summary.key.name().into(),
        count: summary.count_text.as_str().into(),
        status: summary.status.as_str().into(),
        tone: tone_word(summary.tone).into(),
        has_left: summary.left.is_some(),
        left: (left / 100.0) as f32,
        left_text: left_text(summary).into(),
    }
}

/// The provider cards.
pub fn fill(ui: &MainWindow, claude: &[AccountStatus], now_ms: u64) {
    let cards: Vec<_> = summaries(claude, now_ms)
        .iter()
        .map(provider_card)
        .collect();
    ui.set_providers(ModelRc::new(VecModel::from(cards)));
}

#[cfg(test)]
#[path = "providers_tests.rs"]
mod tests;