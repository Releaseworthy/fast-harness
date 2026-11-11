//! `nh models`: the live model list, else the cached or built-in one.

use std::process::ExitCode;

use nh_core::app::Runtime;
use nh_core::llm::anthropic::ModelInfo;

/// One line: id, display name, effort levels (`-` when none), a deprecation mark.
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
        "{:<28} {:<20} {efforts}{note}",
        model.id, model.display_name
    )
}

pub fn run(json: bool) -> Result<ExitCode, String> {
    let runtime = Runtime::load()?;
    let source = runtime.models();
    let (list, failure) = match source.fetch() {
        Ok(list) => (list, None),
        Err(err) => (source.cached(), Some(err)),
    };
    if json {
        let text = serde_json::to_string_pretty(&list).map_err(|err| err.to_string())?;
        println!("{text}");
    } else {
        for model in &list {
            println!("{}", line(model));
        }
    }
    match failure {
        Some(err) => {
            eprintln!("nh: live model list failed, showing the last known list: {err}");
            Ok(ExitCode::FAILURE)
        }
        None => Ok(ExitCode::SUCCESS),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use nh_core::llm::anthropic::builtin_models;

    #[test]
    fn line_shows_id_name_and_efforts() {
        let mut model = builtin_models().remove(0);
        let text = line(&model);
        assert!(text.starts_with("claude-opus-5-5"));
        assert!(text.ends_with("low,medium,high,xhigh,max"));
        model.efforts.clear();
        model.deprecated = true;
        assert!(line(&model).ends_with("-  (deprecated)"));
    }
}
