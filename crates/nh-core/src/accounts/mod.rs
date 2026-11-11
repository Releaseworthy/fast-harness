//! Claude accounts: encrypted store, linked import from claude-switch, usage,
//! automatic switching.
//!
//! - `crypto`: base64 and SHA-256 (PKCE, secret encoding).
//! - `model`: the account record.
//! - `store`: `~/.nh/accounts.json`, secrets encrypted with `platform::protect`.
//! - `dsh_import`: read-only access to claude-switch accounts in `~/.dsh/.credentials.yaml`.
//! - `usage`: usage windows from response headers and from the usage endpoint.
//! - `order`: holds, candidate filtering and claude-switch's switch order.

pub mod crypto;
pub mod dsh_import;
pub mod model;
pub mod order;
pub mod store;
pub mod usage;
