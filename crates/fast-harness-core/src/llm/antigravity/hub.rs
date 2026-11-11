//! The Antigravity Hub identity and Cloud Code control calls, as DSH's
//! antigravity-switch does them (PLAN.md D23): the Hub release read from its
//! update manifest (cached 6 h, fallback 2.29.0), the Hub User-Agent, and
//! `POST <host>/v1internal:<method>` on the daily host, then the sandbox host
//! only on 404, 408, 5xx or a network error.

use crate::http::{Flow, HttpError, HttpHead, HttpRequest, Transport};
use serde_json::Value;
use std::cmp::Ordering;
use std::sync::{Mutex, PoisonError};
use std::time::{SystemTime, UNIX_EPOCH};

/// The electron-builder manifest of the arm64 macOS Hub.
pub const MANIFEST_URL: &str = "https://antigravity-hub-auto-updater-974169037036.us-central1.run.app/manifest/latest-arm64-mac.yml";
/// First Cloud Code host.
pub const DAILY_HOST: &str = "https://daily-cloudcode-pa.googleapis.com";
/// Second host, tried only after [`next_host_on`] or a network error.
pub const SANDBOX_HOST: &str = "https://daily-cloudcode-pa.sandbox.googleapis.com";
/// Hub release used when the manifest is unreachable or too old.
pub const VERSION_FALLBACK: &str = "2.29.0";
/// The oldest release Cloud Code still serves.
pub const VERSION_MINIMUM: &str = "2.23.0";
/// How long a manifest read (or its fallback) is reused.
pub const VERSION_TTL_MS: u64 = 6 * 60 * 60_000;
/// Bound on a control call.
pub const CONTROL_TIMEOUT_MS: u32 = 30_000;
/// Known build changelists; another release omits `cl=`.
const CHANGELISTS: [(&str, &str); 4] = [
    ("2.23.0", "998057262"),
    ("2.23.1", "998881661"),
    ("2.24.0", "1000335682"),
    ("2.29.0", "1013060670"),
];
/// The manifest is the arm64 macOS one, so the platform matches it.
const PLATFORM: &str = "os_type=darwin; arch=arm64";
const MANIFEST_TIMEOUT_MS: u32 = 5_000;
const MAX_MANIFEST: usize = 4096;
const MAX_REPLY: usize = 16 << 20;

/// One control-call answer: status and body (at most 16 MiB).
#[derive(Clone, Debug, PartialEq)]
pub struct ControlReply {
    pub status: u16,
    pub body: Vec<u8>,
}

impl ControlReply {
    /// The body as JSON, `None` when it is not JSON.
    pub fn json(&self) -> Option<Value> {
        serde_json::from_slice(&self.body).ok()
    }

    pub fn ok(&self) -> bool {
        (200..300).contains(&self.status)
    }
}

/// The manifest URL, the host list and the cached Hub release.
#[derive(Debug)]
pub struct Hub {
    manifest_url: String,
    hosts: Vec<String>,
    /// `(version, read at unix ms)`.
    cache: Mutex<Option<(String, u64)>>,
}

impl Default for Hub {
    fn default() -> Self {
        Hub::new(MANIFEST_URL, &[DAILY_HOST, SANDBOX_HOST])
    }
}

impl Hub {
    /// A Hub with other endpoints (tests point them at a scripted transport).
    pub fn new(manifest_url: &str, hosts: &[&str]) -> Hub {
        Hub {
            manifest_url: manifest_url.to_string(),
            hosts: hosts
                .iter()
                .map(|h| h.trim_end_matches('/').to_string())
                .collect(),
            cache: Mutex::new(None),
        }
    }

    /// The hosts in the order they are tried.
    pub fn hosts(&self) -> &[String] {
        &self.hosts
    }

    /// The Hub release: the manifest's when it is at least [`VERSION_MINIMUM`],
    /// else [`VERSION_FALLBACK`]; read at most once per [`VERSION_TTL_MS`].
    pub fn version(&self, http: &dyn Transport) -> String {
        let now = now_ms();
        let mut cache = self.cache.lock().unwrap_or_else(PoisonError::into_inner);
        if let Some((version, at)) = cache.as_ref() {
            if now.saturating_sub(*at) < VERSION_TTL_MS {
                return version.clone();
            }
        }
        let version = self
            .read_manifest(http)
            .filter(|v| compare_versions(v, VERSION_MINIMUM) != Ordering::Less)
            .unwrap_or_else(|| VERSION_FALLBACK.to_string());
        *cache = Some((version.clone(), now));
        version
    }

    fn read_manifest(&self, http: &dyn Transport) -> Option<String> {
        let headers = [
            ("user-agent", "electron-builder"),
            ("cache-control", "no-cache"),
        ];
        let req = HttpRequest {
            method: "GET",
            url: &self.manifest_url,
            headers: &headers,
            body: b"",
            timeout_ms: MANIFEST_TIMEOUT_MS,
        };
        let (head, body) = collect(http, &req, MAX_MANIFEST).ok()?;
        (head.status == 200)
            .then(|| parse_manifest_version(&String::from_utf8_lossy(&body)))
            .flatten()
    }

    /// The Hub User-Agent of the current release.
    pub fn user_agent(&self, http: &dyn Transport) -> String {
        hub_user_agent(&self.version(http))
    }

    /// `POST <host>/v1internal:<method>` with a bearer token, the Hub
    /// User-Agent and a JSON body; `extra` headers replace the defaults of the
    /// same name. The next host only on [`next_host_on`] or a network error.
    pub fn control_call(
        &self,
        http: &dyn Transport,
        access: &str,
        method: &str,
        body: &Value,
        extra: &[(&str, &str)],
    ) -> Result<ControlReply, HttpError> {
        let bearer = format!("Bearer {access}");
        let agent = self.user_agent(http);
        let mut headers: Vec<(&str, &str)> = vec![
            ("authorization", bearer.as_str()),
            ("content-type", "application/json"),
            ("user-agent", agent.as_str()),
        ];
        for (name, value) in extra {
            headers.retain(|(have, _)| !have.eq_ignore_ascii_case(name));
            headers.push((name, value));
        }
        let bytes = body.to_string().into_bytes();
        self.post_with_fallback(http, &format!("/v1internal:{method}"), &headers, &bytes)
    }

    fn post_with_fallback(
        &self,
        http: &dyn Transport,
        path: &str,
        headers: &[(&str, &str)],
        body: &[u8],
    ) -> Result<ControlReply, HttpError> {
        let mut last = Err(no_host());
        for (index, host) in self.hosts.iter().enumerate() {
            let url = format!("{host}{path}");
            let req = HttpRequest {
                method: "POST",
                url: &url,
                headers,
                body,
                timeout_ms: CONTROL_TIMEOUT_MS,
            };
            let reply = collect(http, &req, MAX_REPLY).map(|(head, body)| ControlReply {
                status: head.status,
                body,
            });
            let final_host = index + 1 == self.hosts.len();
            match &reply {
                Ok(answer) if final_host || !next_host_on(answer.status) => return reply,
                Err(_) if final_host => return reply,
                _ => last = reply,
            }
        }
        last
    }
}

fn no_host() -> HttpError {
    HttpError {
        kind: crate::http::HttpErrorKind::InvalidRequest,
        message: "no Cloud Code host".into(),
    }
}

/// Whether a failed answer moves on to the next host: 404, 408 or 5xx.
pub fn next_host_on(status: u16) -> bool {
    status == 404 || status == 408 || status >= 500
}

/// `antigravity/hub/<version> (aidev_client; os_type=darwin; arch=arm64[; cl=<n>])`.
pub fn hub_user_agent(version: &str) -> String {
    let changelist = CHANGELISTS
        .iter()
        .find(|(known, _)| *known == version)
        .map(|(_, cl)| format!("; cl={cl}"))
        .unwrap_or_default();
    format!("antigravity/hub/{version} (aidev_client; {PLATFORM}{changelist})")
}

/// The top-level `version:` of an electron-builder manifest (`x.y.z`, quotes
/// and a trailing comment allowed), or `None`.
pub fn parse_manifest_version(body: &str) -> Option<String> {
    let line = body.lines().find(|l| l.starts_with("version:"))?;
    let mut value = line["version:".len()..].trim();
    if let Some(at) = value.find(" #").or_else(|| value.find("\t#")) {
        value = value[..at].trim();
    }
    for quote in ['\'', '"'] {
        if value.len() >= 2 && value.starts_with(quote) && value.ends_with(quote) {
            value = &value[1..value.len() - 1];
        }
    }
    let parts: Vec<&str> = value.split('.').collect();
    let valid = parts.len() == 3
        && parts
            .iter()
            .all(|p| (1..=6).contains(&p.len()) && p.bytes().all(|b| b.is_ascii_digit()));
    valid.then(|| value.to_string())
}

/// `major.minor.patch` order; a missing or non-numeric part counts as 0.
pub fn compare_versions(left: &str, right: &str) -> Ordering {
    let parts = |v: &str| -> Vec<u64> {
        let mut out: Vec<u64> = v.split('.').map(|p| p.parse().unwrap_or(0)).collect();
        out.resize(3, 0);
        out
    };
    parts(left)[..3].cmp(&parts(right)[..3])
}

/// Sends `req` and keeps up to `max` bytes of the answer.
fn collect(
    http: &dyn Transport,
    req: &HttpRequest<'_>,
    max: usize,
) -> Result<(HttpHead, Vec<u8>), HttpError> {
    let mut body = Vec::new();
    let head = http.send(req, &mut |_, chunk| {
        let room = max.saturating_sub(body.len());
        body.extend_from_slice(&chunk[..chunk.len().min(room)]);
        if body.len() >= max {
            Flow::Stop
        } else {
            Flow::Continue
        }
    })?;
    Ok((head, body))
}

fn now_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| u64::try_from(d.as_millis()).unwrap_or(u64::MAX))
        .unwrap_or(0)
}
