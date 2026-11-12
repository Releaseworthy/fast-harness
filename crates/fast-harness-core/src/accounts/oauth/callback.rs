//! The loopback side of the sign-in: one tiny HTTP/1.1 server that waits for
//! `GET <path>?code=...&state=...`, as claude-switch's `handleCallback`. The
//! Claude sign-in waits on `/callback`.

use super::OAuthError;
use std::io::{self, Read, Write};
use std::net::{TcpListener, TcpStream};
use std::sync::atomic::{AtomicBool, Ordering};
use std::thread;
use std::time::{Duration, Instant};

/// How often the listener looks for a connection, the deadline and `cancel`.
const POLL: Duration = Duration::from_millis(25);
/// Bound on reading one request head (and writing its answer).
const IO_TIMEOUT: Duration = Duration::from_secs(5);
/// Largest request head read.
const MAX_HEAD: usize = 16 * 1024;

/// Accepts connections until the callback arrives, `timeout` passes or `cancel` is set.
pub(crate) fn wait_for_code(
    listener: &TcpListener,
    path: &str,
    state: &str,
    timeout: Duration,
    cancel: &AtomicBool,
) -> Result<String, OAuthError> {
    listener.set_nonblocking(true)?;
    let deadline = Instant::now() + timeout;
    loop {
        if cancel.load(Ordering::Relaxed) {
            return Err(OAuthError::Cancelled);
        }
        if Instant::now() >= deadline {
            return Err(OAuthError::Timeout);
        }
        match listener.accept() {
            Ok((stream, _)) => {
                if let Some(result) = serve(stream, path, state) {
                    return result;
                }
            }
            Err(err) if err.kind() == io::ErrorKind::WouldBlock => thread::sleep(POLL),
            Err(err) if err.kind() == io::ErrorKind::Interrupted => {}
            Err(err) => return Err(OAuthError::Io(err)),
        }
    }
}

/// Answers one connection; `Some` when it was the callback.
fn serve(mut stream: TcpStream, want: &str, state: &str) -> Option<Result<String, OAuthError>> {
    let _ = stream.set_nonblocking(false);
    let _ = stream.set_read_timeout(Some(IO_TIMEOUT));
    let _ = stream.set_write_timeout(Some(IO_TIMEOUT));
    let Some((method, target)) = read_request_line(&mut stream) else {
        respond(&mut stream, 400, "text/plain; charset=utf-8", "bad request");
        return None;
    };
    let (path, query) = target.split_once('?').unwrap_or((target.as_str(), ""));
    if path != want {
        respond(&mut stream, 404, "text/plain; charset=utf-8", "not found");
        return None;
    }
    if method != "GET" {
        respond(
            &mut stream,
            405,
            "text/plain; charset=utf-8",
            "method not allowed",
        );
        return None;
    }
    let (status, message, result) = judge(&parse_query(query), state);
    respond(
        &mut stream,
        status,
        "text/html; charset=utf-8",
        &page(message),
    );
    Some(result)
}

/// The answer to a callback query.
fn judge(
    params: &[(String, String)],
    expected_state: &str,
) -> (u16, &'static str, Result<String, OAuthError>) {
    let get = |name: &str| {
        params
            .iter()
            .find(|(key, _)| key == name)
            .map(|(_, value)| value.as_str())
    };
    if let Some(error) = get("error") {
        let code: String = error.chars().take(200).collect();
        let result = Err(OAuthError::Denied(code));
        return (400, "Sign-in was not completed.", result);
    }
    match (get("code"), get("state")) {
        (Some(code), Some(state)) if !code.is_empty() && state == expected_state => {
            (200, "Account signed in.", Ok(code.to_string()))
        }
        _ => (
            400,
            "Sign-in answer did not match.",
            Err(OAuthError::Mismatch),
        ),
    }
}

/// `<method> <target>` of the request line, once the head has arrived.
fn read_request_line(stream: &mut TcpStream) -> Option<(String, String)> {
    let mut head = Vec::new();
    let mut buf = [0u8; 2048];
    while !head.windows(4).any(|w| w == b"\r\n\r\n") && head.len() < MAX_HEAD {
        let read = stream.read(&mut buf).ok()?;
        if read == 0 {
            break;
        }
        head.extend_from_slice(&buf[..read]);
    }
    let text = String::from_utf8_lossy(&head);
    let line = text.lines().next()?;
    let mut parts = line.split(' ');
    let method = parts.next()?.to_string();
    let target = parts.next()?.to_string();
    parts
        .next()?
        .starts_with("HTTP/")
        .then_some((method, target))
}

fn respond(stream: &mut TcpStream, status: u16, content_type: &str, body: &str) {
    let reason = match status {
        200 => "OK",
        400 => "Bad Request",
        404 => "Not Found",
        _ => "Method Not Allowed",
    };
    let answer = format!(
        "HTTP/1.1 {status} {reason}\r\ncontent-type: {content_type}\r\n\
cache-control: no-store\r\ncontent-length: {}\r\nconnection: close\r\n\r\n{body}",
        body.len()
    );
    let _ = stream.write_all(answer.as_bytes());
    let _ = stream.flush();
}

/// The page the browser shows after the redirect.
fn page(message: &str) -> String {
    format!(
        "<!doctype html><meta charset=\"utf-8\"><title>Fast Harness sign-in</title>\
<body style=\"font:14px/1.5 system-ui,sans-serif;max-width:34rem;margin:12vh auto;\
padding:0 1.5rem;color:#1f2329\"><h1 style=\"font-size:18px\">{message}</h1>\
<p style=\"color:#6b7280\">You can close this tab and return to fast-harness.</p></body>"
    )
}

/// `application/x-www-form-urlencoded` pairs of a query string.
pub(super) fn parse_query(query: &str) -> Vec<(String, String)> {
    query
        .split('&')
        .filter(|pair| !pair.is_empty())
        .map(|pair| {
            let (key, value) = pair.split_once('=').unwrap_or((pair, ""));
            (form_decode(key), form_decode(value))
        })
        .collect()
}

/// Decodes `+` and `%XX`; a malformed escape stays as written.
fn form_decode(text: &str) -> String {
    let bytes = text.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        let hex = bytes.get(i + 1..i + 3).and_then(|pair| {
            std::str::from_utf8(pair)
                .ok()
                .and_then(|s| u8::from_str_radix(s, 16).ok())
        });
        match (bytes[i], hex) {
            (b'%', Some(byte)) => {
                out.push(byte);
                i += 3;
                continue;
            }
            (b'+', _) => out.push(b' '),
            (byte, _) => out.push(byte),
        }
        i += 1;
    }
    String::from_utf8_lossy(&out).into_owned()
}

/// Encodes like `URLSearchParams`: alphanumerics and `*-._` kept, space as `+`.
pub(crate) fn form_encode(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    for byte in text.bytes() {
        match byte {
            b'a'..=b'z' | b'A'..=b'Z' | b'0'..=b'9' | b'*' | b'-' | b'.' | b'_' => {
                out.push(byte as char)
            }
            b' ' => out.push('+'),
            _ => out.push_str(&format!("%{byte:02X}")),
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn form_round_trip() {
        let text = "a b&c=d/é%";
        assert_eq!(form_decode(&form_encode(text)), text);
        assert_eq!(form_decode("%zz+%41"), "%zz A");
    }

    #[test]
    fn judge_needs_matching_state() {
        let params = parse_query("code=abc&state=s1");
        assert_eq!(judge(&params, "s1").2.unwrap(), "abc");
        assert!(matches!(judge(&params, "s2").2, Err(OAuthError::Mismatch)));
        let denied = parse_query("error=access_denied&state=s1");
        assert!(matches!(judge(&denied, "s1").2, Err(OAuthError::Denied(_))));
    }
}
