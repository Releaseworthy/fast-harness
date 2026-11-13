//! `UreqTransport` ends a cancelled call on a real socket (PLAN.md D66):
//! local listeners only, no external network.

use std::io::{Read, Write};
use std::net::{TcpListener, TcpStream};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Mutex;
use std::thread;
use std::time::{Duration, Instant};

use fast_harness_core::http::{
    Flow, HttpError, HttpErrorKind, HttpHead, HttpRequest, Transport, CANCEL_BOUND_MS,
};
use fast_harness_core::net::{live_calls, UreqTransport};

/// The tests count live call threads, so they run one at a time.
static SERIAL: Mutex<()> = Mutex::new(());

struct Sent {
    result: Result<HttpHead, HttpError>,
    body: Vec<u8>,
    /// From the cancel to `send` returning.
    latency: Duration,
}

/// `GET url` with a 5 s limit (without the cancel, the call would end on
/// it), cancelled after `delay`.
fn send_cancelled(url: &str, delay: Duration) -> Sent {
    let transport = UreqTransport::new();
    let cancel = AtomicBool::new(false);
    let req = HttpRequest::new("GET", url, &[], b"")
        .with_timeout_ms(5_000)
        .with_cancel(&cancel);
    let mut body = Vec::new();
    thread::scope(|scope| {
        let setter = scope.spawn(|| {
            thread::sleep(delay);
            cancel.store(true, Ordering::Relaxed);
            Instant::now()
        });
        let result = transport.send(&req, &mut |_, chunk| {
            body.extend_from_slice(chunk);
            Flow::Continue
        });
        let ended = Instant::now();
        let latency = ended.saturating_duration_since(setter.join().unwrap());
        Sent {
            result,
            body,
            latency,
        }
    })
}

fn assert_cancelled(sent: &Sent, state: &str) {
    eprintln!("real socket cancel latency {state}: {:?}", sent.latency);
    let err = sent.result.as_ref().unwrap_err();
    assert_eq!(err.kind, HttpErrorKind::Cancelled, "{state}: {err}");
    assert!(
        sent.latency < Duration::from_millis(CANCEL_BOUND_MS),
        "{state}: {:?}",
        sent.latency
    );
}

/// The server side sees the client's socket closed (read gives 0 or an
/// error) within the bound: the connection was not left open.
fn assert_closed(mut server: TcpStream) {
    server
        .set_read_timeout(Some(Duration::from_millis(CANCEL_BOUND_MS)))
        .unwrap();
    let mut buf = [0u8; 4096];
    loop {
        match server.read(&mut buf) {
            Ok(0) => return,
            Ok(_) => continue,
            Err(err) => {
                let open = matches!(
                    err.kind(),
                    std::io::ErrorKind::WouldBlock | std::io::ErrorKind::TimedOut
                );
                assert!(!open, "the client socket is still open");
                return;
            }
        }
    }
}

/// No call thread outlives its call (beyond the bound).
fn assert_no_live_calls() {
    let start = Instant::now();
    while live_calls() > 0 && start.elapsed() < Duration::from_millis(CANCEL_BOUND_MS) {
        thread::sleep(Duration::from_millis(5));
    }
    assert_eq!(live_calls(), 0, "a call thread outlived its call");
}

/// Accepts one connection, reads the request head, writes `reply` (may be
/// empty) and then stays silent; hands the socket back.
fn silent_server(reply: &'static [u8]) -> (String, thread::JoinHandle<TcpStream>) {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let url = format!("http://{}/v1/messages", listener.local_addr().unwrap());
    let handle = thread::spawn(move || {
        let (mut stream, _) = listener.accept().unwrap();
        let mut seen = Vec::new();
        let mut buf = [0u8; 1024];
        while !seen.windows(4).any(|w| w == b"\r\n\r\n") {
            let n = stream.read(&mut buf).unwrap();
            assert!(n > 0, "client closed before its request");
            seen.extend_from_slice(&buf[..n]);
        }
        stream.write_all(reply).unwrap();
        stream
    });
    (url, handle)
}

#[test]
fn cancel_while_waiting_for_the_response_head() {
    let _serial = SERIAL.lock().unwrap_or_else(|e| e.into_inner());
    let (url, server) = silent_server(b"");
    let sent = send_cancelled(&url, Duration::from_millis(200));
    assert_cancelled(&sent, "waiting for the head");
    assert_closed(server.join().unwrap());
    assert_no_live_calls();
}

#[test]
fn cancel_in_a_silent_body() {
    let _serial = SERIAL.lock().unwrap_or_else(|e| e.into_inner());
    let head =
        b"HTTP/1.1 200 OK\r\ncontent-type: text/event-stream\r\ncontent-length: 1000\r\n\r\nabc";
    let (url, server) = silent_server(head);
    let sent = send_cancelled(&url, Duration::from_millis(300));
    assert_cancelled(&sent, "silent body");
    assert_eq!(sent.body, b"abc");
    assert_closed(server.join().unwrap());
    assert_no_live_calls();
}

#[test]
fn cancel_while_connecting() {
    let _serial = SERIAL.lock().unwrap_or_else(|e| e.into_inner());
    // Nothing listens on this port: Windows retries the connect for about
    // 2 s before it is refused, which is the state under test there.
    let port = TcpListener::bind("127.0.0.1:0")
        .unwrap()
        .local_addr()
        .unwrap()
        .port();
    let url = format!("http://127.0.0.1:{port}/v1/messages");
    let started = Instant::now();
    let sent = send_cancelled(&url, Duration::from_millis(100));
    eprintln!("connect state: send returned after {:?}", started.elapsed());
    match &sent.result {
        // A platform that refuses at once ends before the cancel; Windows
        // must be cancelled.
        Err(err) if err.kind == HttpErrorKind::Connect && !cfg!(windows) => {}
        _ => assert_cancelled(&sent, "connecting"),
    }
    // The connect thread ends when the OS gives up on the connect.
    let start = Instant::now();
    while live_calls() > 0 && start.elapsed() < Duration::from_secs(10) {
        thread::sleep(Duration::from_millis(20));
    }
    eprintln!(
        "connect state: call thread gone after {:?}",
        start.elapsed()
    );
    assert_eq!(live_calls(), 0);
}

#[test]
fn a_call_without_cancel_still_reads_the_whole_body() {
    let _serial = SERIAL.lock().unwrap_or_else(|e| e.into_inner());
    let reply = b"HTTP/1.1 200 OK\r\ncontent-length: 5\r\nconnection: close\r\n\r\nhello";
    let (url, server) = silent_server(reply);
    let cancel = AtomicBool::new(false);
    let req = HttpRequest::new("GET", &url, &[], b"").with_cancel(&cancel);
    let mut body = Vec::new();
    let head = UreqTransport::new()
        .send(&req, &mut |_, chunk| {
            body.extend_from_slice(chunk);
            Flow::Continue
        })
        .unwrap();
    assert_eq!((head.status, body.as_slice()), (200, &b"hello"[..]));
    drop(server.join().unwrap());
    assert_no_live_calls();
}

#[test]
fn calls_with_a_cancel_flag_reuse_one_pooled_connection() {
    let _serial = SERIAL.lock().unwrap_or_else(|e| e.into_inner());
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let url = format!("http://{}/v1/models", listener.local_addr().unwrap());
    let server = thread::spawn(move || {
        let (mut stream, _) = listener.accept().unwrap();
        for _ in 0..2 {
            let mut seen = Vec::new();
            let mut buf = [0u8; 1024];
            while !seen.windows(4).any(|w| w == b"\r\n\r\n") {
                let n = stream.read(&mut buf).unwrap();
                assert!(n > 0, "client closed the pooled connection");
                seen.extend_from_slice(&buf[..n]);
            }
            stream
                .write_all(b"HTTP/1.1 200 OK\r\ncontent-length: 2\r\n\r\nok")
                .unwrap();
        }
        listener.set_nonblocking(true).unwrap();
        listener.accept().is_err()
    });
    let transport = UreqTransport::new();
    for _ in 0..2 {
        let cancel = AtomicBool::new(false);
        let req = HttpRequest::new("GET", &url, &[], b"").with_cancel(&cancel);
        let mut body = Vec::new();
        let head = transport
            .send(&req, &mut |_, chunk| {
                body.extend_from_slice(chunk);
                Flow::Continue
            })
            .unwrap();
        assert_eq!((head.status, body.as_slice()), (200, &b"ok"[..]));
    }
    assert!(server.join().unwrap(), "a second connection was opened");
    assert_no_live_calls();
}
