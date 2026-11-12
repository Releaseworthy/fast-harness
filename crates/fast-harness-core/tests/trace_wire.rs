use fast_harness_core::http::{Flow, HttpRequest, ScriptedResponse, ScriptedTransport, Transport};
use fast_harness_core::trace::wire::WireCapture;
use serde_json::{json, Value};
use std::path::PathBuf;

struct TempDir(PathBuf);

impl TempDir {
    fn new(name: &str) -> TempDir {
        let path = std::env::temp_dir().join(format!(
            "fast-harness-test-{name}-{}-{}",
            std::process::id(),
            fast_harness_core::trace::now_ms()
        ));
        let _ = std::fs::remove_dir_all(&path);
        TempDir(path)
    }
}

impl Drop for TempDir {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

fn read_json(path: PathBuf) -> Value {
    serde_json::from_slice(&std::fs::read(path).unwrap()).unwrap()
}

const SECRET: &str = "sk-ant-secret-value";

fn send(capture: &WireCapture<'_>, body: &[u8]) -> (u16, Vec<u8>) {
    let auth = format!("Bearer {SECRET}");
    let headers = [
        ("Authorization", auth.as_str()),
        ("X-Api-Key", SECRET),
        ("cookie", SECRET),
        ("anthropic-version", "2023-06-01"),
    ];
    let url = "https://api.anthropic.com/v1/messages";
    let req = HttpRequest::new("POST", url, &headers, body);
    let mut seen = Vec::new();
    let head = capture
        .send(&req, &mut |_, chunk| {
            seen.extend_from_slice(chunk);
            Flow::Continue
        })
        .unwrap();
    (head.status, seen)
}

#[test]
fn captures_redacted_request_head_and_raw_bytes() {
    let tmp = TempDir::new("wire");
    let wire = tmp.0.join("s.wire");
    let body = "data: {\"x\":\"h\u{e9}llo\"}\n\n";
    let response = ScriptedResponse::new(200, body, 3).with_header("request-id", "req_1");
    let inner = ScriptedTransport::new(vec![response]);
    let capture = WireCapture::new(&inner, &wire, "t1-s2-a1");
    let (status, seen) = send(&capture, br#"{"model":"m","stream":true}"#);
    assert_eq!((status, seen.as_slice()), (200, body.as_bytes()));
    assert_eq!(capture.failures(), 0);
    let request = read_json(wire.join("t1-s2-a1.request.json"));
    assert_eq!(request["method"], json!("POST"));
    assert_eq!(
        request["url"],
        json!("https://api.anthropic.com/v1/messages")
    );
    assert_eq!(request["body"], json!({"model": "m", "stream": true}));
    assert_eq!(
        request["headers"],
        json!([
            ["Authorization", "<redacted>"],
            ["X-Api-Key", "<redacted>"],
            ["cookie", "<redacted>"],
            ["anthropic-version", "2023-06-01"]
        ])
    );
    let head = read_json(wire.join("t1-s2-a1.head.json"));
    assert_eq!(
        head,
        json!({"status": 200, "headers": [["request-id", "req_1"]]})
    );
    let raw = std::fs::read(wire.join("t1-s2-a1.response.txt")).unwrap();
    assert_eq!(raw, body.as_bytes());
    for entry in std::fs::read_dir(&wire).unwrap() {
        let text =
            String::from_utf8_lossy(&std::fs::read(entry.unwrap().path()).unwrap()).into_owned();
        assert!(!text.contains(SECRET), "secret leaked into wire capture");
    }
}

#[test]
fn non_json_body_is_a_string_and_empty_response_still_has_files() {
    let tmp = TempDir::new("wire-empty");
    let wire = tmp.0.join("s.wire");
    let inner = ScriptedTransport::new(vec![ScriptedResponse::new(204, "", 1)]);
    let capture = WireCapture::new(&inner, &wire, "t1-s1-a1");
    let (status, seen) = send(&capture, b"plain text");
    assert_eq!((status, seen.len()), (204, 0));
    let request = read_json(wire.join("t1-s1-a1.request.json"));
    assert_eq!(request["body"], json!("plain text"));
    let head = read_json(wire.join("t1-s1-a1.head.json"));
    assert_eq!(head["status"], json!(204));
    assert_eq!(
        std::fs::read(wire.join("t1-s1-a1.response.txt")).unwrap(),
        b""
    );
    assert_eq!(capture.failures(), 0);
}

#[test]
fn write_failures_are_counted_and_never_fail_the_call() {
    let tmp = TempDir::new("wire-fail");
    std::fs::create_dir_all(&tmp.0).unwrap();
    let blocker = tmp.0.join("not-a-dir");
    std::fs::write(&blocker, b"file").unwrap();
    let inner = ScriptedTransport::new(vec![ScriptedResponse::new(200, "abc", 1)]);
    let capture = WireCapture::new(&inner, &blocker, "t1-s1-a1");
    let (status, seen) = send(&capture, b"{}");
    assert_eq!((status, seen.as_slice()), (200, b"abc".as_slice()));
    // folder, request.json, response.txt and head.json each fail once.
    assert_eq!(capture.failures(), 4);
}

#[test]
fn transport_errors_pass_through() {
    let tmp = TempDir::new("wire-err");
    let inner = ScriptedTransport::new(Vec::new());
    let capture = WireCapture::new(&inner, &tmp.0, "t1-s1-a2");
    let req = HttpRequest::new("GET", "http://x", &[], b"");
    assert!(capture.send(&req, &mut |_, _| Flow::Continue).is_err());
    assert!(tmp.0.join("t1-s1-a2.request.json").is_file());
    assert!(!tmp.0.join("t1-s1-a2.head.json").exists());
}
