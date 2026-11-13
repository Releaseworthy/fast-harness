//! Pure parsing only: the process-wide version is tested in
//! `tests/claude_code_version.rs`, a process of its own.

use super::*;

#[test]
fn parse_takes_plain_releases_only() {
    assert_eq!(parse("2.1.323"), Some((2, 1, 323)));
    assert_eq!(parse(" 10.0.1 "), Some((10, 0, 1)));
    for bad in [
        "",
        "2.1",
        "2.1.3.4",
        "2.1.x",
        "2.1.3-beta.1",
        "v2.1.3",
        "2..3",
        "2.1.3\r\nx: y",
    ] {
        assert_eq!(parse(bad), None, "{bad:?}");
    }
    assert!(parse("2.1.290") > parse("2.1.280"));
    assert!(parse("2.10.0") > parse("2.9.999"));
}

#[test]
fn required_by_reads_the_api_refusal() {
    let message = "invalid_request_error: Claude Code 2.1.280 does not support this model; \
        version 2.1.290 or newer is required. Run 'claude update', or update the Claude \
        desktop app, then try again.";
    assert_eq!(required_by(message).as_deref(), Some("2.1.290"));
    assert_eq!(
        required_by("invalid_request_error: max_tokens too large"),
        None
    );
    assert_eq!(required_by("version banana or newer is required"), None);
}
