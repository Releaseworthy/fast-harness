//! Native plugins (PLAN.md D52, docs/plugins.md): Rust crates compiled
//! into a custom build by `fast-harness-plugins` (`register_all`, generated
//! from `plugins.toml`). A plugin crate exposes
//!
//! ```
//! use fast_harness_core::ext::v1::Ext;
//! fast_harness_core::ext_api!(1);
//!
//! pub fn register(ext: &mut Ext) {
//!     let _ = ext;
//! }
//! ```
//!
//! and adds tools, providers, in-process hooks and panels through [`v1::Ext`].
//!
//! Versioning: the API's major version is part of the path (`ext::v1`).
//! A breaking change adds `ext::v2` and removes `v1`, so a plugin written
//! for another major version names a module that does not exist and fails
//! to compile; [`ext_api!`] states the version the plugin targets and turns
//! a mismatch into a compile error with a readable message:
//!
//! ```compile_fail
//! fast_harness_core::ext_api!(2);
//! ```
//!
//! ```compile_fail
//! pub fn register(ext: &mut fast_harness_core::ext::v2::Ext) {}
//! ```
//!
//! The official binaries are built with an empty plugin list: nothing is
//! installed and [`installed`] answers an empty [`v1::Ext`].

pub mod v1;

use std::sync::{Arc, OnceLock};

pub use v1::Ext;

/// The major version of the plugin API (the `v1` module).
pub const API_VERSION: u32 = 1;

/// Fails to compile unless `$major` is [`API_VERSION`].
#[macro_export]
macro_rules! ext_api {
    ($major:literal) => {
        const _: () = assert!(
            $major == $crate::ext::API_VERSION,
            "this plugin targets another major version of the Fast Harness plugin API (fast_harness_core::ext::API_VERSION)"
        );
    };
}

static INSTALLED: OnceLock<Arc<Ext>> = OnceLock::new();

/// Installs the plugins' registrations for this process (the binaries call
/// it once at start through `fast_harness_plugins::install`). A second call
/// is ignored and returns false.
pub fn install(ext: Ext) -> bool {
    INSTALLED.set(Arc::new(ext)).is_ok()
}

/// The installed registrations; empty when nothing was installed.
pub fn installed() -> Arc<Ext> {
    INSTALLED.get().cloned().unwrap_or_default()
}
