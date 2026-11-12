//! Where credentials come from. Before each model call the agent takes a
//! [`Lease`] from an [`AuthSource`]; after the call it reports the response
//! headers (live usage) or the failure (failover decision). The account pool
//! (`accounts`) is the main implementation; [`StaticAuth`] serves a fixed key.

use std::fmt;

use crate::http::{HttpHead, Transport};
use crate::llm::{Auth, LlmError};

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Lease {
    /// Stable account id written to the trace. Never a secret.
    pub account: String,
    /// Display label for UIs (an email or a key name).
    pub label: String,
    pub auth: Auth,
}

/// What the agent does after a call failed with an auth or limit error.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Failover {
    /// Another credential is ready: take a new lease and retry now.
    Retry,
    /// No credential is usable for this many milliseconds; wait, then take a new lease.
    Wait(u64),
    /// The source cannot fix this error.
    Fail,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AuthError {
    pub message: String,
}

impl fmt::Display for AuthError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.message)
    }
}

impl std::error::Error for AuthError {}

pub trait AuthSource: Send + Sync {
    /// Credential for the next call. May refresh a token over `http` first.
    fn lease(&self, http: &dyn Transport) -> Result<Lease, AuthError>;

    /// Response headers of a call made with `lease` (live usage headers).
    fn observe(&self, lease: &Lease, head: &HttpHead);

    /// A call made with `lease` failed with `ErrorKind::Auth` or `ErrorKind::RateLimit`.
    fn on_failure(
        &self,
        lease: &Lease,
        err: &LlmError,
        model: &str,
        http: &dyn Transport,
    ) -> Failover;

    /// A problem the user should see that does not stop the call, such as an
    /// accounts file that could not be written. Never holds a secret.
    fn warning(&self) -> Option<String> {
        None
    }
}

/// A fixed credential, for example an API key from the environment.
pub struct StaticAuth {
    lease: Lease,
}

impl StaticAuth {
    pub fn new(account: &str, label: &str, auth: Auth) -> StaticAuth {
        StaticAuth {
            lease: Lease {
                account: account.into(),
                label: label.into(),
                auth,
            },
        }
    }
}

impl AuthSource for StaticAuth {
    fn lease(&self, _http: &dyn Transport) -> Result<Lease, AuthError> {
        Ok(self.lease.clone())
    }

    fn observe(&self, _lease: &Lease, _head: &HttpHead) {}

    fn on_failure(
        &self,
        _lease: &Lease,
        err: &LlmError,
        _model: &str,
        _http: &dyn Transport,
    ) -> Failover {
        match err.retry_after_ms() {
            Some(ms) if err.kind == crate::llm::ErrorKind::RateLimit => Failover::Wait(ms),
            _ => Failover::Fail,
        }
    }
}
