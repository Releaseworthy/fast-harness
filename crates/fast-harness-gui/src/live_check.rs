//! Checks a staged UI folder before the live preview gets it (PLAN.md D51):
//! it must compile, export `MainWindow` and keep every entry of the UI
//! contract (`docs/ui-contract.md`) with its kind and type.

use std::future::Future;
use std::path::Path;

use slint_interpreter::{ComponentDefinition, DiagnosticLevel, ValueType};

use crate::ui_contract::{self, Entry, Kind};

/// At most this many errors go to the banner.
const SHOWN: usize = 12;

/// The errors of the UI whose main file is `main` (empty when it passes).
pub fn check(main: &Path) -> Vec<String> {
    let compiler = slint_interpreter::Compiler::default();
    let mut future = std::pin::pin!(compiler.build_from_path(main));
    let mut cx = std::task::Context::from_waker(std::task::Waker::noop());
    // The compiler does no I/O that waits: its future is ready at once.
    let std::task::Poll::Ready(result) = future.as_mut().poll(&mut cx) else {
        return vec!["the Slint compiler did not finish".into()];
    };
    let errors: Vec<String> = result
        .diagnostics()
        .filter(|d| d.level() == DiagnosticLevel::Error)
        .map(|d| {
            let file = d.source_file().and_then(Path::file_name);
            let file = file.map(|f| f.to_string_lossy().to_string());
            let (line, col) = d.line_column();
            format!("{}:{line}:{col}: {}", file.unwrap_or_default(), d.message())
        })
        .collect();
    if !errors.is_empty() {
        return errors;
    }
    match result.component("MainWindow") {
        Some(def) => contract_errors(&def, &ui_contract::parse(ui_contract::DOC)),
        None => vec!["main.slint does not export MainWindow".into()],
    }
}

/// The contract entries `def` lacks or declares with another kind or type.
pub fn contract_errors(def: &ComponentDefinition, entries: &[Entry]) -> Vec<String> {
    entries
        .iter()
        .filter_map(|e| entry_error(def, e).map(|why| format!("{}.{}: {why}", e.owner, e.name)))
        .collect()
}

fn entry_error(def: &ComponentDefinition, e: &Entry) -> Option<String> {
    let main = e.owner == "MainWindow";
    if !main && def.global_properties(&e.owner).is_none() {
        return Some("missing global".into());
    }
    let same = |n: &String| n.replace('_', "-") == e.name;
    match e.kind {
        Kind::Property => {
            let props: Vec<(String, ValueType)> = if main {
                def.properties().collect()
            } else {
                def.global_properties(&e.owner)?.collect()
            };
            let Some((_, ty)) = props.iter().find(|(n, _)| same(n)) else {
                return Some(format!("missing property <{}>", e.ty));
            };
            let want = value_type(&e.ty)?;
            (*ty != want).then(|| format!("is {ty:?}, the contract needs <{}>", e.ty))
        }
        Kind::Callback | Kind::Function => {
            let names: Vec<String> = match (main, e.kind) {
                (true, Kind::Callback) => def.callbacks().collect(),
                (true, _) => def.functions().collect(),
                (false, Kind::Callback) => def.global_callbacks(&e.owner)?.collect(),
                (false, _) => def.global_functions(&e.owner)?.collect(),
            };
            let what = if e.kind == Kind::Callback {
                "callback"
            } else {
                "public function"
            };
            (!names.iter().any(same)).then(|| format!("missing {what}"))
        }
    }
}

/// The interpreter's type of a contract type, when it has a plain one.
fn value_type(ty: &str) -> Option<ValueType> {
    Some(match ty {
        "string" => ValueType::String,
        "bool" => ValueType::Bool,
        "int" | "float" | "length" | "physical-length" | "duration" | "angle" | "percent"
        | "relative-font-size" => ValueType::Number,
        "color" | "brush" => ValueType::Brush,
        "image" => ValueType::Image,
        _ if ty.starts_with('[') => ValueType::Model,
        _ => return None,
    })
}

/// The banner text of a failed check.
pub fn report(what: &str, dir: &Path, errors: &[String]) -> String {
    let mut text = format!("{what}: {} has {} error(s).", dir.display(), errors.len());
    for error in errors.iter().take(SHOWN) {
        text.push_str("\n- ");
        text.push_str(error);
    }
    if errors.len() > SHOWN {
        text.push_str(&format!("\n- and {} more", errors.len() - SHOWN));
    }
    text
}
