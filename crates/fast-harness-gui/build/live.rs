//! The UI code of `fast-harness-gui-live` (cargo feature `ui-files`, PLAN.md D51).
//!
//! Slint's live preview keeps the typed Rust API (`MainWindow::new`, `set_*`,
//! `on_*`) but builds the window with the interpreter from a `.slint` file at
//! run time. Its generator writes the build machine's path of `ui/main.slint`
//! into the code; this module points it at `crate::live::main_file()` instead,
//! the folder the binary staged from `~/.fast-harness/ui` at start.

use std::path::{Path, PathBuf};

/// The call the generator emits, followed by the main file's string literal.
const NEEDLE: &str = "LiveReloadingComponent";

/// Writes `<OUT_DIR>/live_ui.rs` when the feature is on; nothing otherwise.
pub fn generate(main: &str) {
    if std::env::var_os("CARGO_FEATURE_UI_FILES").is_none() {
        return;
    }
    let out = PathBuf::from(std::env::var("OUT_DIR").expect("cargo sets OUT_DIR"));
    let target = out.join("live_ui.rs");
    let main = std::path::absolute(main).expect("the UI path must resolve");
    // The generator reads this variable; the build script is single threaded.
    std::env::set_var("SLINT_LIVE_PREVIEW", "1");
    let config = slint_build::CompilerConfiguration::new();
    let result = slint_build::compile_with_output_path(&main, &target, config);
    std::env::remove_var("SLINT_LIVE_PREVIEW");
    result.expect("Slint UI must compile (live)");
    retarget(&target);
}

fn retarget(path: &Path) {
    let code = std::fs::read_to_string(path).expect("the generated UI code must be readable");
    let patched = replace_main_file(&code);
    assert!(
        patched != code,
        "the live-preview code no longer has the main file literal"
    );
    std::fs::write(path, patched).expect("OUT_DIR must be writable");
}

/// Replaces the string literal after each `LiveReloadingComponent ... new(compiler,`
/// with the run-time path.
fn replace_main_file(code: &str) -> String {
    let mut out = String::with_capacity(code.len());
    let mut rest = code;
    while let Some(at) = rest.find(NEEDLE) {
        let (head, tail) = rest.split_at(at + NEEDLE.len());
        out.push_str(head);
        rest = tail;
        let Some(open) = literal_after_new(rest) else {
            continue;
        };
        let close = literal_end(&rest[open + 1..]) + open + 1;
        out.push_str(&rest[..open]);
        out.push_str("crate::live::main_file()");
        rest = &rest[close + 1..];
    }
    out.push_str(rest);
    out
}

/// The offset of the opening quote when `rest` is `::new(compiler, "...`.
fn literal_after_new(rest: &str) -> Option<usize> {
    let call = rest.find('(')?;
    let compact: String = rest[..call]
        .chars()
        .filter(|c| !c.is_whitespace())
        .collect();
    if compact != "::new" {
        return None;
    }
    let quote = rest.find('"')?;
    let between: String = rest[call + 1..quote]
        .chars()
        .filter(|c| !c.is_whitespace())
        .collect();
    (between == "compiler,").then_some(quote)
}

/// The offset of the closing quote of a literal whose body starts at `body`.
fn literal_end(body: &str) -> usize {
    let mut escaped = false;
    for (i, c) in body.char_indices() {
        match c {
            '\\' if !escaped => escaped = true,
            '"' if !escaped => return i,
            _ => escaped = false,
        }
    }
    body.len()
}
