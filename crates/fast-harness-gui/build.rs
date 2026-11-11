#[path = "build/res.rs"]
mod res;
#[path = "build/version.rs"]
mod version;

use std::path::PathBuf;

const ICON: &str = "assets/fast-harness.ico";

fn main() {
    slint_build::compile("ui/main.slint").expect("Slint UI must compile");
    println!("cargo:rerun-if-changed={ICON}");
    println!("cargo:rerun-if-changed=build/res.rs");
    println!("cargo:rerun-if-changed=build/version.rs");
    let windows = std::env::var("CARGO_CFG_TARGET_OS").as_deref() == Ok("windows");
    let msvc = std::env::var("CARGO_CFG_TARGET_ENV").as_deref() == Ok("msvc");
    if windows && msvc {
        link_resources();
    }
}

/// Writes `<OUT_DIR>/fast-harness-gui.res` (icon and version info) and links it into
/// the binaries; link.exe takes `.res` files as inputs.
fn link_resources() {
    let ico = std::fs::read(ICON).expect("the app icon must be readable");
    let version = std::env::var("CARGO_PKG_VERSION").unwrap_or_default();
    let bytes =
        res::build(&ico, &version, "Fast Harness").expect("the app icon must be a valid ICO");
    let out = PathBuf::from(std::env::var("OUT_DIR").expect("cargo sets OUT_DIR"));
    let path = out.join("fast-harness-gui.res");
    std::fs::write(&path, bytes).expect("OUT_DIR must be writable");
    println!("cargo:rustc-link-arg-bins={}", path.display());
}
