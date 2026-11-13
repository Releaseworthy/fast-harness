//! Claude accounts: encrypted store, usage, automatic switching.
//!
//! - `crypto`: re-export of `crate::crypto` (base64 and SHA-256, PKCE, secret encoding).
//! - `model`: the account record.
//! - `store`: `~/.fast-harness/accounts.json`, secrets encrypted with `platform::protect`.
//! - `usage`: usage windows from response headers and from the usage endpoint.
//! - `order`: holds, candidate filtering and claude-switch's switch order.
//! - `oauth`: browser sign-in (PKCE, loopback callback), code exchange, refresh, profile.
//! - `remote`: usage of idle accounts from the usage endpoint.
//! - `pool`: the Anthropic accounts as the agent's `AuthSource`, with failover.
//! - `providers`: the per-provider summaries the Accounts views open on.

pub use crate::crypto;
pub mod model;
pub mod oauth;
pub mod order;
pub mod pool;
pub mod providers;
pub mod remote;
pub mod store;
pub mod usage;
