//! Raw wire capture of model calls (PLAN.md D7: off by default, on with `Config::wire`). Each call writes
//! `<name>.request.json`, `<name>.head.json` and `<name>.response.txt` into
//! the session's `.wire` folder. Credentials never reach the files.

use crate::http::{Flow, HttpError, HttpHead, HttpRequest, Transport};
use serde_json::{json, Value};
use std::fs::{self, File, OpenOptions};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU32, Ordering};

/// Header names whose values are replaced by `<redacted>` (compared case-insensitively).
const SECRET_HEADERS: [&str; 4] = ["authorization", "x-api-key", "cookie", "set-cookie"];
pub const REDACTED: &str = "<redacted>";

/// A [`Transport`] that records each call it forwards. A failure to write a
/// wire file never fails the call: it is counted in [`WireCapture::failures`].
pub struct WireCapture<'a> {
    inner: &'a dyn Transport,
    dir: PathBuf,
    name: String,
    failures: AtomicU32,
}

impl<'a> WireCapture<'a> {
    /// `name` is the call name, such as `t1-s2-a1`.
    pub fn new(inner: &'a dyn Transport, dir: &Path, name: &str) -> WireCapture<'a> {
        WireCapture {
            inner,
            dir: dir.to_path_buf(),
            name: name.to_string(),
            failures: AtomicU32::new(0),
        }
    }

    /// Wire file writes that failed so far.
    pub fn failures(&self) -> u32 {
        self.failures.load(Ordering::Relaxed)
    }

    /// Path of `<dir>/<name>.<suffix>`, for example `request.json`.
    pub fn path(&self, suffix: &str) -> PathBuf {
        self.dir.join(format!("{}.{suffix}", self.name))
    }

    fn fail(&self) {
        self.failures.fetch_add(1, Ordering::Relaxed);
    }

    fn write_json(&self, suffix: &str, value: &Value) {
        let written = serde_json::to_vec_pretty(value)
            .map_err(std::io::Error::from)
            .and_then(|bytes| fs::write(self.path(suffix), bytes));
        if written.is_err() {
            self.fail();
        }
    }

    fn write_head(&self, head: &HttpHead) {
        let headers = redact_headers(head.headers.iter().map(|(k, v)| (k.as_str(), v.as_str())));
        let value = json!({ "status": head.status, "headers": headers });
        self.write_json("head.json", &value);
    }

    fn open_response(&self) -> Option<File> {
        let opened = OpenOptions::new()
            .create(true)
            .write(true)
            .truncate(true)
            .open(self.path("response.txt"));
        opened.map_err(|_| self.fail()).ok()
    }
}

impl Transport for WireCapture<'_> {
    fn send(
        &self,
        req: &HttpRequest<'_>,
        on_chunk: &mut dyn FnMut(&HttpHead, &[u8]) -> Flow,
    ) -> Result<HttpHead, HttpError> {
        if fs::create_dir_all(&self.dir).is_err() {
            self.fail();
        }
        self.write_json("request.json", &request_json(req));
        let mut response = self.open_response();
        let mut head_written = false;
        let result = self.inner.send(req, &mut |head, chunk| {
            if !head_written {
                self.write_head(head);
                head_written = true;
            }
            if let Some(file) = response.as_mut() {
                if file.write_all(chunk).is_err() {
                    self.fail();
                    response = None;
                }
            }
            on_chunk(head, chunk)
        });
        if let (Ok(head), false) = (&result, head_written) {
            self.write_head(head);
        }
        if let Some(mut file) = response {
            if file.flush().is_err() {
                self.fail();
            }
        }
        result
    }
}

/// `{method, url, headers, body}`; `body` is the parsed JSON when the bytes
/// are JSON, else the text (invalid UTF-8 replaced).
pub fn request_json(req: &HttpRequest<'_>) -> Value {
    let body = serde_json::from_slice::<Value>(req.body)
        .unwrap_or_else(|_| Value::String(String::from_utf8_lossy(req.body).into_owned()));
    json!({
        "method": req.method,
        "url": req.url,
        "headers": redact_headers(req.headers.iter().copied()),
        "body": body,
    })
}

/// `[[name, value], ...]` in order, secret values replaced by `<redacted>`.
pub fn redact_headers<'h>(headers: impl Iterator<Item = (&'h str, &'h str)>) -> Value {
    let pairs: Vec<Value> = headers
        .map(|(name, value)| {
            let secret = SECRET_HEADERS
                .iter()
                .any(|s| s.eq_ignore_ascii_case(name.trim()));
            json!([name, if secret { REDACTED } else { value }])
        })
        .collect();
    Value::Array(pairs)
}
