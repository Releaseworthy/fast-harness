//! The native plugins of this build (PLAN.md D52, docs/plugins.md).
//! `build.rs` generates [`register_all`] and [`PLUGINS`] from
//! `plugins.toml` at the workspace root; the binaries call [`install`]
//! once at start. The core never depends on this crate (layering): only
//! the binaries do.

include!(concat!(env!("OUT_DIR"), "/register_all.rs"));

/// Registers every plugin and installs the result for this process
/// (`fast_harness_core::ext::install`); false when there is nothing to
/// install (the official builds) or something was installed already.
pub fn install() -> bool {
    let mut ext = fast_harness_core::ext::v1::Ext::new();
    register_all(&mut ext);
    !ext.is_empty() && fast_harness_core::ext::install(ext)
}

#[cfg(test)]
#[allow(dead_code)]
mod gen;

#[cfg(test)]
mod tests;
