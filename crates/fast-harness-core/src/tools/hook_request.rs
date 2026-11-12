//! What a `PreModelRequest` hook may change in a model request (PLAN.md
//! D63): the effort (a thinking level, never `off`) and a lower
//! `max_tokens`. The system prompt, the tool list, the model and every
//! message are frozen (D36): an answer naming one of them is refused whole.

use serde_json::Value;

/// The effort words a hook may ask for; the provider fits them to the model.
pub const EFFORTS: [&str; 6] = ["minimal", "low", "medium", "high", "xhigh", "max"];

/// The request keys a hook may not change (named in the refusal).
const FROZEN: [&str; 5] = ["system", "tools", "messages", "model", "headers"];

/// The change of one `PreModelRequest` answer (`{"request": {...}}`).
#[derive(Clone, Debug, Default, PartialEq, Eq)]
#[non_exhaustive]
pub struct RequestChange {
    /// One of [`EFFORTS`].
    pub effort: Option<String>,
    /// At most the request's own `max_tokens` (a hook may only lower it).
    pub max_tokens: Option<u32>,
}

impl RequestChange {
    pub fn with_effort(mut self, effort: impl Into<String>) -> RequestChange {
        self.effort = Some(effort.into());
        self
    }

    pub fn with_max_tokens(mut self, max_tokens: u32) -> RequestChange {
        self.max_tokens = Some(max_tokens);
        self
    }

    /// Reads the `request` object of an answer; any key besides `effort`
    /// and `maxTokens` makes the whole answer an error.
    pub fn parse(value: &Value) -> Result<RequestChange, String> {
        let Value::Object(map) = value else {
            return Err("`request` is not a JSON object".into());
        };
        let mut change = RequestChange::default();
        for (key, item) in map {
            match key.as_str() {
                "effort" => {
                    let word = item.as_str().ok_or("`request.effort` is not a string")?;
                    change.effort = Some(word.to_string());
                }
                "maxTokens" => {
                    let n = item.as_u64().and_then(|n| u32::try_from(n).ok());
                    change.max_tokens = Some(n.ok_or("`request.maxTokens` is not a count")?);
                }
                key if FROZEN.contains(&key) => {
                    return Err(format!(
                        "`request.{key}` is frozen for the session and cannot change"
                    ))
                }
                key => return Err(format!("`request.{key}` is not a key a hook may change")),
            }
        }
        Ok(change)
    }

    /// `Err` when the change breaks a rule against the request's own
    /// `max_tokens` (`cap`).
    pub fn check(&self, cap: u32) -> Result<(), String> {
        if let Some(effort) = self.effort.as_deref() {
            if !EFFORTS.contains(&effort) {
                let all = EFFORTS.join(", ");
                return Err(format!("effort `{effort}` is not one of {all}"));
            }
        }
        match self.max_tokens {
            Some(n) if n == 0 || n > cap => Err(format!("maxTokens {n} is not in 1..={cap}")),
            _ => Ok(()),
        }
    }

    /// The trace words of the keys that are set.
    pub fn words(&self) -> Vec<&'static str> {
        let parts = [
            (self.effort.is_some(), "effort"),
            (self.max_tokens.is_some(), "maxTokens"),
        ];
        parts.iter().filter(|(on, _)| *on).map(|p| p.1).collect()
    }

    /// `later` over `self`, key by key.
    pub fn merge(&mut self, later: RequestChange) {
        if later.effort.is_some() {
            self.effort = later.effort;
        }
        if later.max_tokens.is_some() {
            self.max_tokens = later.max_tokens;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn only_effort_and_max_tokens_may_change() {
        let change = RequestChange::parse(&json!({"effort": "low", "maxTokens": 99})).unwrap();
        assert_eq!(
            change,
            RequestChange::default()
                .with_effort("low")
                .with_max_tokens(99)
        );
        assert_eq!(change.words(), ["effort", "maxTokens"]);
        assert!(change.check(100).is_ok() && change.check(50).is_err());
        for key in [
            "system",
            "tools",
            "messages",
            "model",
            "headers",
            "temperature",
        ] {
            assert!(RequestChange::parse(&json!({ key: "x" })).is_err(), "{key}");
        }
        let off = RequestChange::default().with_effort("off");
        assert!(off.check(100).is_err(), "thinking stays on");
        assert!(RequestChange::default()
            .with_max_tokens(0)
            .check(9)
            .is_err());
    }
}
