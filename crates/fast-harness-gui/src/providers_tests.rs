use super::*;
use fast_harness_core::accounts::providers::ProviderKey;

const NOW: u64 = 1_800_000_000_000;

#[test]
fn the_claude_card_says_what_to_do_first() {
    let cards: Vec<_> = summaries(&[], NOW).iter().map(provider_card).collect();
    assert_eq!(cards.len(), 1, "only Claude on this branch (D53)");
    assert_eq!(cards[0].id, ProviderKey::Claude.id());
    assert_eq!(cards[0].name, "Claude");
    assert_eq!(cards[0].count, "No accounts");
    assert_eq!(cards[0].tone, "empty");
    assert!(!cards[0].has_left);
    assert_eq!(cards[0].left_text, "Sign in or add an API key");
}