//! The shared rules of every extension file (phase 29): the name rule,
//! `version` (absent reads as 1; a newer one is skipped with its reason)
//! and `enabled` (`false` turns one file off without deleting it). Each
//! kind keeps its own parsing and calls [`json_header`] or [`md_header`]
//! first; the walk of the layer folders is [`super::layers::kind_files`].

use std::collections::BTreeMap;
use std::path::Path;

use serde_json::Value;

/// The newest extension file version this build reads.
pub const VERSION: u64 = 1;
/// The skip reason of a file turned off with `"enabled": false`.
pub const DISABLED: &str = "disabled (`enabled: false`)";

/// The shared keys of one file.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Header {
    pub version: u64,
    pub enabled: bool,
}

/// `version` of `value`: absent is 1, a newer or non-integer one an error.
pub fn check_version(value: Option<&Value>) -> Result<u64, String> {
    match value {
        None => Ok(1),
        Some(Value::Number(n)) => match n.as_u64() {
            Some(v @ 1..=VERSION) => Ok(v),
            Some(v) if v > VERSION => Err(format!(
                "version {v} is newer than this build reads ({VERSION}); update Fast Harness"
            )),
            _ => Err(format!("bad version {n}: use a whole number from 1")),
        },
        Some(other) => Err(format!("bad version {other}: use a whole number from 1")),
    }
}

/// `version` and `enabled` of a JSON file. `"enabled": false` or, for tool
/// files, the older `"disabled": true` turn it off: off wins when both are set.
pub fn json_header(value: &Value) -> Result<Header, String> {
    let object = value.as_object().ok_or("not a JSON object")?;
    let version = check_version(object.get("version"))?;
    let enabled = match object.get("enabled") {
        None => true,
        Some(Value::Bool(on)) => *on,
        Some(other) => return Err(format!("bad `enabled` {other}: use true or false")),
    };
    let disabled = object.get("disabled").and_then(Value::as_bool) == Some(true);
    Ok(Header {
        version,
        enabled: enabled && !disabled,
    })
}

/// [`json_header`] of a Markdown file's front matter (`version: 2`,
/// `enabled: false`).
pub fn md_header(meta: &BTreeMap<String, String>) -> Result<Header, String> {
    let version = match meta.get("version").map(|v| v.trim()) {
        None => 1,
        Some(text) => match text.parse::<u64>() {
            Ok(number) => check_version(Some(&Value::from(number)))?,
            Err(_) => return Err(format!("bad version `{text}`: use a whole number from 1")),
        },
    };
    let enabled = match meta.get("enabled").map(|v| v.trim()) {
        None | Some("true") => true,
        Some("false") => false,
        Some(other) => return Err(format!("bad `enabled` `{other}`: use true or false")),
    };
    Ok(Header { version, enabled })
}

/// `Err(DISABLED)` for a file turned off, the header otherwise.
pub fn on(header: Header) -> Result<Header, String> {
    match header.enabled {
        true => Ok(header),
        false => Err(DISABLED.into()),
    }
}

/// The `version` a file declares (1 when absent or unreadable), for listings.
pub fn version_of(path: &Path) -> u64 {
    let Ok(text) = std::fs::read_to_string(path) else {
        return 1;
    };
    let is_md = path
        .extension()
        .is_some_and(|e| e.eq_ignore_ascii_case("md"));
    let found = match is_md {
        true => super::index::front_matter(&text).0.get("version").cloned(),
        false => serde_json::from_str::<Value>(&text)
            .ok()
            .and_then(|v| v.get("version").map(Value::to_string)),
    };
    found.and_then(|v| v.trim().parse().ok()).unwrap_or(1)
}

/// A tool, agent, command, skill, panel or MCP server name: 1 to 64
/// letters, digits, `_` or `-` (the one name rule).
pub fn check_name(name: &str) -> Result<(), String> {
    let ok = (1..=64).contains(&name.len())
        && name
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '-');
    match ok {
        true => Ok(()),
        false => Err(format!(
            "bad name `{name}`: use 1 to 64 letters, digits, `_` or `-`"
        )),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn version_and_enabled_rules() {
        assert_eq!(check_version(None), Ok(1));
        assert_eq!(check_version(Some(&json!(1))), Ok(1));
        assert!(check_version(Some(&json!(2)))
            .unwrap_err()
            .contains("newer"));
        assert!(check_version(Some(&json!("1"))).is_err());
        let off = json_header(&json!({"enabled": false})).unwrap();
        assert!(!off.enabled);
        let alias = json_header(&json!({"disabled": true, "enabled": true})).unwrap();
        assert!(!alias.enabled, "off wins");
        let meta = BTreeMap::from([("enabled".to_string(), "false".to_string())]);
        assert_eq!(on(md_header(&meta).unwrap()), Err(DISABLED.to_string()));
        assert!(check_name("a b").is_err() && check_name("ok_1").is_ok());
    }
}
