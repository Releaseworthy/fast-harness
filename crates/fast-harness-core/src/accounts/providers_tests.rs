use super::*;
use crate::accounts::model::{AccountKind, Provider};
use crate::accounts::usage::{Usage, Window};

const NOW: u64 = 1_800_000_000_000;

fn account(id: &str, provider: Provider) -> Account {
    Account {
        id: id.into(),
        kind: AccountKind::Own,
        provider,
        email: Some(format!("{id}@example.com")),
        name: None,
        plan: None,
        added_at: 0,
    }
}

fn claude(id: &str, active: bool, used: &[f64]) -> AccountStatus {
    let window = |utilization| Window {
        utilization,
        resets_at: Some(NOW as i64 + 3_600_000),
        ..Window::default()
    };
    let usage = (!used.is_empty()).then(|| Usage {
        five_hour: used.first().copied().map(window),
        seven_day: used.get(1).copied().map(window),
        ..Usage::default()
    });
    AccountStatus {
        account: account(id, Provider::Anthropic),
        active,
        usage,
        held_until: None,
        hold_reason: None,
        token_ok: true,
    }
}

#[test]
fn empty_providers_say_no_accounts() {
    let cards = summaries(&[], NOW);
    assert_eq!(cards.len(), 1);
    assert_eq!(cards[0].key, ProviderKey::Claude);
    assert_eq!(cards[0].status, "Not set up");
    assert_eq!(cards[0].count_text, "No accounts");
    assert_eq!(cards[0].tone, Tone::Empty);
}

/// Bug: an empty card said `No accounts` twice, in the count line and the status.
#[test]
fn empty_card_says_no_accounts_once() {
    for card in summaries(&[], NOW) {
        assert_ne!(card.status, card.count_text, "{:?}", card.key);
        assert!(!card.status.is_empty(), "the status stays in words");
    }
}

#[test]
fn active_account_and_lowest_left() {
    let list = [
        claude("work", true, &[42.0, 70.0]),
        claude("home", false, &[99.0]),
    ];
    let card = &summaries(&list, NOW)[0];
    assert_eq!(card.count, 2);
    assert_eq!(card.count_text, "2 accounts");
    assert_eq!(card.status, "Active: work@example.com");
    assert_eq!(card.tone, Tone::Ready);
    // 7d at 70% used is the lowest left; the idle account does not count.
    assert_eq!(card.left, Some(30.0));
    let low = [claude("work", true, &[95.0])];
    assert_eq!(summaries(&low, NOW)[0].tone, Tone::Attention);
    let idle = [claude("work", false, &[])];
    let card = &summaries(&idle, NOW)[0];
    assert_eq!(
        (card.status.as_str(), card.left),
        ("No account in use", None)
    );
    assert_eq!(card.count_text, "1 account");
}
