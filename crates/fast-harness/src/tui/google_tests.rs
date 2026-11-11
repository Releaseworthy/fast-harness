use super::*;
use fast_harness_core::accounts::google::{GoogleHold, QuotaGroup};
use fast_harness_core::accounts::model::{Account, AccountKind, Provider};

const NOW: u64 = 1_800_000_000_000;
const MIN: u64 = 60_000;

fn window(used: f64, resets_at: Option<u64>) -> Option<QuotaWindow> {
    Some(QuotaWindow {
        used,
        resets_at,
        disabled: false,
    })
}

fn status(hold: Option<GoogleHold>, quota: Option<GoogleQuota>) -> GoogleStatus {
    GoogleStatus {
        account: Account {
            id: "g-1".into(),
            kind: AccountKind::Own,
            provider: Provider::Google,
            email: Some("ana@example.com".into()),
            name: None,
            plan: Some("Pro".into()),
            added_at: 0,
        },
        active: true,
        hold,
        quota,
        read_at: Some(NOW - 5 * MIN),
        last_error: None,
    }
}

#[test]
fn google_row_with_quota() {
    let quota = GoogleQuota {
        groups: vec![
            QuotaGroup {
                name: "Gemini Models".into(),
                five_hour: window(0.42, Some(NOW + 125 * MIN)),
                weekly: window(0.10, None),
            },
            QuotaGroup {
                name: "Claude and GPT models".into(),
                // The reset passed: reads as unused.
                five_hour: window(0.80, Some(NOW - MIN)),
                weekly: window(0.55, Some(NOW + 3 * 1440 * MIN)),
            },
        ],
    };
    let row = google_row(&status(None, Some(quota)), NOW, "*");
    assert!(row.starts_with("* google ana@example.com"), "{row}");
    assert!(row.contains(" Pro "), "{row}");
    assert!(
        row.contains("Gemini 5h  42% wk  10% | Claude/GPT 5h   0% wk  55% ok"),
        "{row}"
    );
}

#[test]
fn google_row_states() {
    let hold = |reason| GoogleHold {
        until: NOW + 90 * MIN,
        reason,
        url: None,
    };
    let held = google_row(&status(Some(hold(HoldReason::RateLimit)), None), NOW, "*");
    assert!(held.contains("Gemini 5h    - wk    -"), "{held}");
    assert!(
        held.ends_with("held (rate limit), resumes in 1h 30m"),
        "{held}"
    );
    let verify = google_row(&status(Some(hold(HoldReason::Validation)), None), NOW, "*");
    assert!(verify.ends_with("needs verification"), "{verify}");
    let mut failed = status(None, None);
    failed.last_error = Some("Google refused (HTTP 500)".into());
    failed.active = false;
    let row = google_row(&failed, NOW, "*");
    assert!(row.starts_with("  google"), "{row}");
    assert!(row.ends_with("error: Google refused (HTTP 500)"), "{row}");
}

#[test]
fn accounts_keys_reach_google_rows() {
    use super::super::ui::{Action, Screen, Ui};
    use ratatui::crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
    let mut ui = Ui::new("m".into(), None);
    ui.screen = Screen::Accounts;
    ui.google = vec![status(None, None)];
    let mut press = |code| ui.key(KeyEvent::new(code, KeyModifiers::NONE));
    assert_eq!(press(KeyCode::Char('g')), Action::LoginGoogle);
    // No Claude account: the first row is the Google one.
    assert_eq!(press(KeyCode::Enter), Action::UseAccount("g-1".into()));
}
