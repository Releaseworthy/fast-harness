//! Generates `register_all` from `plugins.toml` at the workspace root
//! (PLAN.md D52, docs/plugins.md). No file or an empty list: an empty
//! `register_all`, so the official binaries carry no plugin code.

#[path = "src/gen.rs"]
mod gen;

use std::path::PathBuf;

fn main() {
    let manifest = PathBuf::from(std::env::var("CARGO_MANIFEST_DIR").expect("set by cargo"));
    let list = manifest.join("../../plugins.toml");
    let cargo_toml = manifest.join("Cargo.toml");
    println!("cargo:rerun-if-changed={}", list.display());
    println!("cargo:rerun-if-changed={}", cargo_toml.display());
    println!("cargo:rerun-if-changed=src/gen.rs");
    let text = std::fs::read_to_string(&list).unwrap_or_default();
    let plugins = gen::parse(&text).unwrap_or_else(|err| fail(&err));
    let toml = std::fs::read_to_string(&cargo_toml).unwrap_or_else(|err| fail(&err.to_string()));
    if let Err(err) = gen::check_deps(&toml, &plugins) {
        fail(&err);
    }
    let out = PathBuf::from(std::env::var("OUT_DIR").expect("set by cargo"));
    let source = gen::generate(&plugins);
    if let Err(err) = std::fs::write(out.join("register_all.rs"), source) {
        fail(&err.to_string());
    }
}

/// A build script reports a bad input by failing the build.
fn fail(why: &str) -> ! {
    eprintln!("fast-harness-plugins: {why}");
    std::process::exit(1);
}
