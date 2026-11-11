//! `fast-harness models`: the live model lists of every provider with a
//! credential, else the cached or built-in ones, grouped by provider (D24).

use std::process::ExitCode;

use fast_harness_core::app::Runtime;
use fast_harness_core::llm::anthropic::ModelInfo;
use fast_harness_core::llm::catalog::provider_of;
use serde_json::Value;

/// One line: provider, id, display name, effort levels (`-` when none), a
/// deprecation mark.
fn line(model: &ModelInfo) -> String {
    let efforts = if model.efforts.is_empty() {
        "-".to_string()
    } else {
        model.efforts.join(",")
    };
    let note = if model.deprecated {
        "  (deprecated)"
    } else {
        ""
    };
    format!(
        "{:<12} {:<40} {:<24} {efforts}{note}",
        provider_of(&model.id),
        model.id,
        model.display_name
    )
}

/// The `--json` entry of `model`: its fields plus `provider`.
fn entry(model: &ModelInfo) -> Result<Value, String> {
    let mut value = serde_json::to_value(model).map_err(|err| err.to_string())?;
    if let Value::Object(map) = &mut value {
        map.insert("provider".into(), provider_of(&model.id).into());
    }
    Ok(value)
}

pub fn run(json: bool) -> Result<ExitCode, String> {
    let runtime = Runtime::load()?;
    let source = runtime.models();
    let (list, failure) = match source.fetch() {
        Ok(list) => (list, None),
        Err(err) => (source.cached(), Some(err)),
    };
    if json {
        let entries = list.iter().map(entry).collect::<Result<Vec<_>, _>>()?;
        let text = serde_json::to_string_pretty(&entries).map_err(|err| err.to_string())?;
        println!("{text}");
    } else {
        for model in &list {
            println!("{}", line(model));
        }
    }
    match failure {
        Some(err) => {
            eprintln!("fast-harness: live model list failed, showing the last known list: {err}");
            Ok(ExitCode::FAILURE)
        }
        None => Ok(ExitCode::SUCCESS),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use fast_harness_core::llm::anthropic::builtin_models;

    #[test]
    fn line_shows_id_name_and_efforts() {
        let mut model = builtin_models().remove(0);
        let text = line(&model);
        assert!(text.starts_with("anthropic    claude-opus-5-5"));
        assert!(text.ends_with("low,medium,high,xhigh,max"));
        model.efforts.clear();
        model.deprecated = true;
        assert!(line(&model).ends_with("-  (deprecated)"));
        model.id = "antigravity/gemini-cli".into();
        assert!(line(&model).starts_with("antigravity  antigravity/gemini-cli"));
        let json = entry(&model).unwrap();
        assert_eq!(json["provider"], "antigravity");
        assert_eq!(json["id"], "antigravity/gemini-cli");
    }
}
