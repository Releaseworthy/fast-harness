//! Claude and Google accounts: encrypted store, usage, automatic switching.
//!
//! - `crypto`: base64 and SHA-256 (PKCE, secret encoding).
//! - `model`: the account record.
//! - `store`: `~/.fast-harness/accounts.json`, secrets encrypted with `platform::protect`.
//! - `usage`: usage windows from response headers and from the usage endpoint.
//! - `order`: holds, candidate filtering and claude-switch's switch order.
//! - `oauth`: browser sign-in (PKCE, loopback callback), code exchange, refresh, profile.
//! - `remote`: usage of idle accounts from the usage endpoint.
//! - `pool`: the Anthropic accounts as the agent's `AuthSource`, with failover.
//! - `google`: Google Antigravity sign-in, project, plan and quota.

pub mod crypto;
pub mod google;
pub mod model;
pub mod oauth;
pub mod order;
pub mod pool;
pub mod remote;
pub mod store;
pub mod usage;
