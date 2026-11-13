//! Guard rails of PLAN.md phase 30: the layering of `fast-harness-core`, the
//! size limits of `AGENTS.md` and the document sizes, read as text. No parser
//! crate is used, and every file of the tree is read once, then shared.
//!
//! A line is one line of the file: a function counts from its `fn` line to the
//! line of its closing brace, inclusive, with nothing subtracted. Comments,
//! string literals, char literals and raw strings are blanked before a brace,
//! an identifier or an import is read, so a `{` inside them moves no count.
//!
//! Test code is exempt from the function rule (2026-11-13: 17 of the 30
//! functions over 50 lines sat in `crates/*/tests`, in a `*_tests.rs` file or
//! in a `#[cfg(test)]` module; a test that builds one long fixture is fine,
//! while this table is the list the owner must shorten). The file rule still
//! covers every file.

// A file of this test target is the crate root, so its helpers need the path:
// the size rule holds every file under 500 lines and this file grew past it.
#[path = "guard_rails/scan.rs"]
mod scan;
#[path = "guard_rails/text.rs"]
mod text;

use std::collections::BTreeSet;
use std::fs;

use scan::{
    declared_modules, file_violations, function_violations, layer_of, lib_layer_names,
    module_exists, paste_table, read, table_problems, trace_key_reads, upward_imports,
    workspace_root,
};
use text::functions;

/// The layers of `crates/fast-harness-core/src/lib.rs`, lowest first: a module
/// never imports a module of a higher group. The order inside a group is the
/// order of the line of `lib.rs`, which `the_layer_line_of_lib_names_the_same_modules`
/// holds `lib.rs` to.
const LAYERS: &[&[&str]] = &[
    &["types", "theme", "ui_thread"],
    &["http", "sse", "platform"],
    &["store", "net"],
    &["llm", "tools", "auth", "config"],
    &["trace", "session", "events", "keys"],
    &["workspaces", "accounts"],
    &["agent"],
    &["view", "panels", "stats"],
    &["reflect"],
    &["service"],
    &["ext"],
    &["app"],
];

/// Upward imports that exist today: (from, to, file). This table may only
/// shrink: the test fails when an entry matches no import any more, so a fix
/// removes its line.
const ALLOWED: &[(&str, &str, &str)] = &[];

/// Over the size limits today: (path, function name or `<file>`, lines today).
/// This table may only shrink: the test fails when an item grew, when a new
/// violation is not listed, and when a listed item is gone or within the limit.
/// Shortening these functions is the next phase, not this one.
const EXCEPTIONS: &[(&str, &str, usize)] = &[
    ("crates/fast-harness/src/args.rs", "parse", 98),
    (
        "crates/fast-harness-core/src/view/mod.rs",
        "apply_control",
        67,
    ),
    ("crates/fast-harness-gui/src/wire.rs", "connect", 63),
    ("crates/fast-harness/src/main.rs", "main", 57),
    (
        "crates/fast-harness-core/src/tools/hook_run.rs",
        "execute",
        53,
    ),
    ("crates/fast-harness/src/args_sessions.rs", "parse", 52),
    (
        "crates/fast-harness-core/examples/step_bench.rs",
        "main",
        51,
    ),
    (
        "crates/fast-harness-core/src/llm/openai/mod.rs",
        "stream",
        51,
    ),
    ("crates/fast-harness-core/src/tools/find.rs", "run_with", 51),
];

const FILE_LIMIT: usize = 500;
const FUNCTION_LIMIT: usize = 50;
const ARCHITECTURE_BYTES: usize = 12_000;
const PLAN_BYTES: usize = 12_000;

#[test]
fn every_declared_module_sits_in_one_layer() {
    let lib = read("crates/fast-harness-core/src/lib.rs");
    let declared = declared_modules(&lib);
    let mut problems = Vec::new();
    for name in &declared {
        if layer_of(name).is_none() {
            problems.push(format!("{name}: declared in lib.rs, in no group of LAYERS"));
        }
        if !module_exists(name) {
            problems.push(format!("{name}: no src/{name}.rs and no src/{name}/mod.rs"));
        }
    }
    for group in LAYERS {
        for name in *group {
            if !declared.iter().any(|found| found == name) {
                problems.push(format!("{name}: in LAYERS, not declared in lib.rs"));
            } else if !module_exists(name) {
                problems.push(format!(
                    "{name}: in LAYERS, no src/{name}.rs and no src/{name}/mod.rs"
                ));
            }
        }
    }
    assert!(
        problems.is_empty(),
        "the layers of lib.rs and of this test disagree:\n{}",
        problems.join("\n")
    );
}

#[test]
fn the_layer_line_of_lib_names_the_same_modules() {
    let lib = read("crates/fast-harness-core/src/lib.rs");
    let named = lib_layer_names(&lib);
    let expected: Vec<String> = LAYERS
        .iter()
        .flat_map(|group| group.iter().map(|name| name.to_string()))
        .collect();
    assert_eq!(
        named, expected,
        "the layer line of lib.rs and LAYERS must name the same modules in the same order"
    );
}

#[test]
fn no_module_imports_a_higher_layer() {
    let found = upward_imports();
    let allowed: BTreeSet<(String, String, String)> = ALLOWED
        .iter()
        .map(|(from, to, file)| (from.to_string(), to.to_string(), file.to_string()))
        .collect();
    let mut problems = Vec::new();
    for ((from, to, file), lines) in &found {
        if allowed.contains(&(from.clone(), to.clone(), file.clone())) {
            continue;
        }
        problems.push(format!(
            "{from} imports {to}, a higher layer, in {file}, line {lines:?}"
        ));
    }
    for (from, to, file) in &allowed {
        if !found.contains_key(&(from.clone(), to.clone(), file.clone())) {
            problems.push(format!(
                "{from} -> {to} in {file}: fixed or stale, remove the ALLOWED line"
            ));
        }
    }
    assert!(
        problems.is_empty(),
        "the layering (a module never imports a higher one) is broken:\n{}\n\
         an upward import that cannot move goes into ALLOWED, which may only shrink",
        problems.join("\n")
    );
}

#[test]
fn no_file_reaches_500_lines() {
    let violations = file_violations();
    let problems = table_problems(&violations, true);
    assert!(
        problems.is_empty(),
        "files of {FILE_LIMIT} lines or more:\n{}\n{}",
        problems.join("\n"),
        paste_table(&violations)
    );
}

#[test]
fn no_function_runs_over_50_lines() {
    let violations = function_violations();
    let problems = table_problems(&violations, false);
    assert!(
        problems.is_empty(),
        "functions of more than {FUNCTION_LIMIT} lines (test code not counted):\n{}\n{}",
        problems.join("\n"),
        paste_table(&violations)
    );
}

#[test]
fn no_source_outside_trace_reads_event_data_by_key() {
    let reads = trace_key_reads();
    assert!(
        reads.is_empty(),
        "read a trace event through `TraceEvent::record()` (PLAN.md D61), not by string key:\n{}",
        reads.join("\n")
    );
}

#[test]
fn the_documents_stay_small() {
    let architecture = read("ARCHITECTURE.md").len();
    assert!(
        architecture <= ARCHITECTURE_BYTES,
        "ARCHITECTURE.md is {architecture} bytes; the limit is {ARCHITECTURE_BYTES}"
    );
    let plan = read("PLAN.md").len();
    assert!(
        plan < PLAN_BYTES,
        "PLAN.md is {plan} bytes; the limit is under {PLAN_BYTES}"
    );
}

#[test]
fn the_plan_index_names_every_decision() {
    let plan = read("PLAN.md");
    let linked: BTreeSet<String> = scan::decision_links(&plan);
    let folder = workspace_root().join("docs/decisions");
    let mut files: BTreeSet<String> = BTreeSet::new();
    for entry in fs::read_dir(&folder).expect("docs/decisions folder") {
        let path = entry.expect("decision file").path();
        if path.extension() == Some(std::ffi::OsStr::new("md")) {
            let name = path.file_name().unwrap_or_default().to_string_lossy();
            files.insert(name.into_owned());
        }
    }
    let mut problems = Vec::new();
    for name in &linked {
        if !files.contains(name) {
            problems.push(format!(
                "PLAN.md links docs/decisions/{name}, which does not exist"
            ));
        }
    }
    for name in &files {
        if !linked.contains(name) {
            problems.push(format!(
                "docs/decisions/{name} is in no line of the PLAN.md index"
            ));
        }
    }
    assert!(
        problems.is_empty(),
        "the decision index is out of date:\n{}",
        problems.join("\n")
    );
}

#[test]
fn the_counter_counts_what_a_reader_measures() {
    // Five lines from the `fn` line to the closing brace line, although two
    // braces and a comment hide inside strings and a raw string.
    let source = "fn five() {\n  let a = \"{\";\n  let b = '{';\n  let c = r#\"}\"#;\n}\n\
                  fn one() { let _ = 0; }\n\
                  fn pointer(f: fn(u8) -> u8) -> u8 { f(0) }\n\
                  trait T { fn declared(&self) -> u8; }\n";
    let found: Vec<(String, usize)> = functions("x.rs", source)
        .into_iter()
        .map(|func| (func.name, func.lines))
        .collect();
    assert_eq!(
        found,
        vec![
            ("five".to_string(), 5),
            ("one".to_string(), 1),
            ("pointer".to_string(), 1)
        ],
        "one line counts from the fn line to the closing brace line, inclusive"
    );
}
