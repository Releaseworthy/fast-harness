use super::*;

const NOW: u64 = 1_800_000_000_000;
// Not a real secret: only checks that the words and the file never echo it.
const SECRET: &str = "test-only-client-secret-42";

fn temp_home(name: &str) -> std::path::PathBuf {
    let home = std::env::temp_dir().join(format!("fh-gui-{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&home);
    std::fs::create_dir_all(&home).expect("temp home");
    home
}

#[test]
fn cards_carry_count_status_and_hint() {
    let google = crate::google_fixtures::statuses(NOW);
    let cards: Vec<_> = summaries(&[], &google, &ClientState::Missing, NOW)
        .iter()
        .map(|s| provider_card(s, &ClientState::Missing))
        .collect();
    assert_eq!(cards[0].id, "claude");
    assert_eq!(cards[0].count, "No accounts");
    assert_eq!(cards[0].status, "Not set up");
    assert_eq!(cards[0].tone, "empty");
    assert_eq!(cards[0].left_text, "Sign in or add an API key");
    assert!(!cards[0].has_left);
    assert_eq!(cards[1].name, "Google Antigravity");
    assert_eq!(cards[1].count, format!("{} accounts", google.len()));
    assert_eq!(cards[1].status, "Needs setup");
    assert_eq!(cards[1].tone, "attention");
}

#[test]
fn ready_google_card_shows_the_lowest_left() {
    let google = crate::google_fixtures::statuses(NOW);
    let client = ClientState::Ready("id".into());
    let summary = &summaries(&[], &google, &client, NOW)[1];
    let card = provider_card(summary, &client);
    assert!(card.status.starts_with("Active: "), "{}", card.status);
    assert!(card.has_left);
    assert!(
        card.left_text.ends_with("% left at the lowest"),
        "{}",
        card.left_text
    );
    assert!((0.0..=1.0).contains(&card.left));
}

#[test]
fn saving_the_client_never_echoes_the_secret() {
    let home = temp_home("client");
    let (error, note) = save_client(&home, " client-id.apps ", &format!(" {SECRET}\n"));
    assert!(!error, "{note}");
    assert!(note.contains("client-id.apps"), "{note}");
    assert!(!note.contains(SECRET));
    let file = std::fs::read_to_string(home.join("google-client.json")).expect("file");
    assert!(!file.contains(SECRET), "the secret is stored encrypted");
    let client = google::client_from(None, None, &home).expect("client");
    assert_eq!(client.secret, SECRET);
    let (error, note) = save_client(&home, "client-id", "   ");
    assert!(error);
    assert!(note.starts_with("Cannot save the OAuth client"), "{note}");
    std::fs::remove_dir_all(&home).expect("cleanup");
}

#[test]
fn needs_setup_stops_after_a_save() {
    let home = temp_home("setup");
    let before = ClientState::of(&home);
    // The client variables, when set, win over the file.
    if std::env::var("FAST_HARNESS_GOOGLE_CLIENT_ID").is_err() {
        assert_eq!(before, ClientState::Missing);
    }
    save_client(&home, "client-id", SECRET);
    let after = google::client_from(None, None, &home).map(|c| c.id);
    assert_eq!(after.ok().as_deref(), Some("client-id"));
    let summary = &summaries(&[], &[], &ClientState::Ready("client-id".into()), NOW)[1];
    assert_eq!(summary.status, "Not set up");
    std::fs::remove_dir_all(&home).expect("cleanup");
}
