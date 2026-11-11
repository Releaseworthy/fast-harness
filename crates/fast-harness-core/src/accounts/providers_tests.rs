use super::*;
use crate::accounts::google::{GoogleQuota, QuotaGroup, QuotaWindow};
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

fn google(id: &str, active: bool, used: Option<f64>) -> GoogleStatus {
    let window = |used| QuotaWindow {
        used,
        resets_at: None,
        disabled: false,
    };
    let quota = used.map(|u| GoogleQuota {
        groups: vec![QuotaGroup {
            name: "Gemini Models".into(),
            five_hour: Some(window(u)),
            weekly: Some(QuotaWindow {
                disabled: true,
                ..window(1.0)
            }),
        }],
    });
    GoogleStatus {
        account: account(id, Provider::Google),
        active,
        hold: None,
        quota,
        read_at: None,
        last_error: None,
    }
}

#[test]
fn empty_providers_say_no_accounts() {
    let cards = summaries(&[], &[], &ClientState::Ready("id".into()), NOW);
    assert_eq!(cards.len(), 2);
    assert_eq!(cards[0].key, ProviderKey::Claude);
    assert_eq!(cards[0].status, "No accounts");
    assert_eq!(cards[0].count_text, "No accounts");
    assert_eq!(cards[0].tone, Tone::Empty);
    assert_eq!(cards[1].key.name(), "Google Antigravity");
    assert_eq!(cards[1].left, None);
}

#[test]
fn active_account_and_lowest_left() {
    let list = [
        claude("work", true, &[42.0, 70.0]),
        claude("home", false, &[99.0]),
    ];
    let card = &summaries(&list, &[], &ClientState::Ready("id".into()), NOW)[0];
    assert_eq!(card.count, 2);
    assert_eq!(card.count_text, "2 accounts");
    assert_eq!(card.status, "Active: work@example.com");
    assert_eq!(card.tone, Tone::Ready);
    // 7d at 70% used is the lowest left; the idle account does not count.
    assert_eq!(card.left, Some(30.0));
    let low = [claude("work", true, &[95.0])];
    assert_eq!(
        summaries(&low, &[], &ClientState::Ready("id".into()), NOW)[0].tone,
        Tone::Attention
    );
    let idle = [claude("work", false, &[])];
    let card = &summaries(&idle, &[], &ClientState::Ready("id".into()), NOW)[0];
    assert_eq!(
        (card.status.as_str(), card.left),
        ("No account in use", None)
    );
    assert_eq!(card.count_text, "1 account");
}

#[test]
fn google_quota_skips_disabled_windows() {
    let list = [google("ana", true, Some(0.25)), google("bo", false, None)];
    let card = &summaries(&[], &list, &ClientState::Ready("id".into()), NOW)[1];
    assert_eq!(card.status, "Active: ana@example.com");
    assert_eq!(card.left, Some(75.0));
    assert_eq!(card.count, 2);
}

#[test]
fn google_without_client_needs_setup() {
    let list = [google("ana", true, Some(0.25))];
    let card = &summaries(&[], &list, &ClientState::Missing, NOW)[1];
    assert_eq!(
        (card.status.as_str(), card.tone),
        ("Needs setup", Tone::Attention)
    );
    let broken = ClientState::Broken("google-client.json is not valid".into());
    let card = &summaries(&[], &[], &broken, NOW)[1];
    assert_eq!(card.status, "OAuth client unreadable");
    assert_eq!(card.count_text, "No accounts");
}

#[test]
fn client_state_reads_the_home() {
    let home = std::env::temp_dir().join(format!("fh-providers-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&home);
    std::fs::create_dir_all(&home).unwrap();
    let env_set = std::env::var("FAST_HARNESS_GOOGLE_CLIENT_ID").is_ok();
    if !env_set {
        assert_eq!(ClientState::of(&home), ClientState::Missing);
    }
    google::save_client(&home, "client-id", "not-a-real-secret").unwrap();
    if !env_set {
        assert_eq!(
            ClientState::of(&home),
            ClientState::Ready("client-id".into())
        );
    }
    std::fs::write(home.join("google-client.json"), "{").unwrap();
    if !env_set {
        assert!(matches!(ClientState::of(&home), ClientState::Broken(_)));
    }
    std::fs::remove_dir_all(&home).unwrap();
}
