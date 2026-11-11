use nh_core::accounts::dsh_import::*;
use std::path::PathBuf;

const FIXTURE: &str = r#"# written by a test, not by DSH
version: 1
records:
  claude-switch/u-0000aaaa-1111-2222-3333-444455556666:
    kind: grant
    payload:
      type: oauth
      refresh: test-refresh-1
      access: test-access-1   # trailing comment
      expires: 1794400565123
      account:
        uuid: 0000aaaa-1111-2222-3333-444455556666
        email: 'one@example.test'
        name: "Test \"One\""
        plan: max

        savedAt: '2026-01-01T00:00:00.000Z'
        updatedAt: "2026-01-02T00:00:00.000Z"
  llm-pi-ai/anthropic:
    kind: grant
    payload:
      type: oauth
      refresh: test-refresh-pi
      access: test-access-pi
      expires: 1794400565123
  "claude-switch/u-bbbb":
    kind: grant
    payload:
      type: 'oauth'
      refresh: "test-refresh-2"
      access: "test-access-2"
      expires: 1.7944e12
  claude-switch/u-no-refresh:
    payload:
      type: oauth
      access: test-access-3
      expires: 1
  claude-switch/u-apikey:
    payload:
      type: api
      key: test-key-4
  claude-switch/settings:
    payload:
      type: oauth
      refresh: test-refresh-5
      access: test-access-5
      expires: 1
  other/list:
    kind: data
    payload:
      items:
        - one
        - two: 2
      note: |
        line one
        access: test-access-6
      flow: {a: 1, b: [2, 3]}
refs:
  SOME_NAME: test-ref-value
"#;

#[test]
fn reads_only_claude_switch_oauth_accounts() {
    let linked = parse_linked(FIXTURE).unwrap();
    assert_eq!(linked.len(), 2);
    let first = &linked[0];
    assert_eq!(first.id, "u-0000aaaa-1111-2222-3333-444455556666");
    assert_eq!(first.uuid, "0000aaaa-1111-2222-3333-444455556666");
    assert_eq!(first.email.as_deref(), Some("one@example.test"));
    assert_eq!(first.name.as_deref(), Some("Test \"One\""));
    assert_eq!(first.plan.as_deref(), Some("max"));
    assert_eq!(first.token.access, "test-access-1");
    assert_eq!(first.token.expires_at, 1_794_400_565_123);
    let second = &linked[1];
    assert_eq!(second.id, "u-bbbb");
    assert_eq!(second.uuid, "bbbb");
    assert_eq!(second.email, None);
    assert_eq!(second.token.access, "test-access-2");
    assert_eq!(second.token.expires_at, 1_794_400_000_000);
}

#[test]
fn debug_hides_the_access_token() {
    let linked = parse_linked(FIXTURE).unwrap();
    let text = format!("{linked:?}");
    assert!(!text.contains("test-access"));
    assert!(text.contains("<redacted>"));
    assert!(text.contains("1794400565123"));
    assert!(!linked[0].token.is_expired(1_794_400_565_122));
    assert!(linked[0].token.is_expired(1_794_400_565_123));
}

#[test]
fn other_versions_and_odd_content() {
    assert!(matches!(
        parse_linked("version: 2\nrecords:\n"),
        Err(DshError::UnsupportedVersion)
    ));
    assert!(parse_linked("").unwrap().is_empty());
    assert!(parse_linked("---\n- a\n- b\n").unwrap().is_empty());
    let unterminated = "records:\n  claude-switch/u-x:\n    payload:\n      type: oauth\n      refresh: r\n      access: \"test-access-7\n      expires: 5\n";
    assert!(parse_linked(unterminated).unwrap().is_empty());
}

#[test]
fn crlf_and_escapes() {
    let text = "version: 1\r\nrecords:\r\n  claude-switch/u-c:\r\n    payload:\r\n      type: oauth\r\n      refresh: r\r\n      access: \"test-\\u0061ccess-8\"\r\n      expires: 9\r\n      account:\r\n        name: 'it''s'\r\n";
    let linked = parse_linked(text).unwrap();
    assert_eq!(linked.len(), 1);
    assert_eq!(linked[0].token.access, "test-access-8");
    assert_eq!(linked[0].name.as_deref(), Some("it's"));
}

struct TempDir(PathBuf);

impl Drop for TempDir {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

#[test]
fn file_reads_are_fresh() {
    let dir = TempDir(std::env::temp_dir().join(format!("nh-dsh-test-{}", std::process::id())));
    std::fs::create_dir_all(&dir.0).unwrap();
    let path = dir.0.join(".credentials.yaml");
    assert!(read_linked(&path).unwrap().is_empty());
    assert_eq!(read_linked_token(&path, "bbbb").unwrap(), None);

    std::fs::write(&path, FIXTURE).unwrap();
    assert_eq!(read_linked(&path).unwrap().len(), 2);
    let token = read_linked_token(&path, "0000AAAA-1111-2222-3333-444455556666")
        .unwrap()
        .unwrap();
    assert_eq!(token.access, "test-access-1");

    std::fs::write(&path, FIXTURE.replace("test-access-1", "test-access-9")).unwrap();
    let token = read_linked_token(&path, "0000aaaa-1111-2222-3333-444455556666")
        .unwrap()
        .unwrap();
    assert_eq!(token.access, "test-access-9");
    assert_eq!(read_linked_token(&path, "u-unknown").unwrap(), None);
}

#[test]
fn paths_follow_dsh_home() {
    let dir = std::env::temp_dir().join("nh-dsh-home-test");
    std::env::set_var("DSH_HOME", &dir);
    assert_eq!(dsh_home(), Some(dir.clone()));
    assert_eq!(credentials_path(), Some(dir.join(".credentials.yaml")));
    std::env::remove_var("DSH_HOME");
    if let Some(home) = nh_core::platform::home_dir() {
        assert_eq!(dsh_home(), Some(home.join(".dsh")));
    }
}
