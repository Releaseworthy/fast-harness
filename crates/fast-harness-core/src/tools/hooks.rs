//! `hooks.json` (PLAN.md D44): external commands run at seven points of
//! the agent loop. The user file and the trusted project file both load
//! (user entries first); no file costs one failed read per layer folder
//! and nothing is ever spawned. Running a hook is [`run`] (`hook_run.rs`).

use std::path::{Path, PathBuf};

use serde::Deserialize;
use serde_json::Value;

use crate::config::layers::{LayerFile, Layers, Skipped, Source};

pub use super::hook_run::{run, HookRun};

/// The file name in each layer folder.
pub const HOOKS_FILE: &str = "hooks.json";
/// Timeout of an entry without `timeoutMs`.
pub const DEFAULT_TIMEOUT_MS: u64 = 60_000;

/// The points of the loop a hook can run at.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum HookEvent {
    SessionStart,
    UserPromptSubmit,
    PreToolUse,
    PostToolUse,
    PreCompact,
    Stop,
    SubagentStop,
}

const EVENTS: [HookEvent; 7] = [
    HookEvent::SessionStart,
    HookEvent::UserPromptSubmit,
    HookEvent::PreToolUse,
    HookEvent::PostToolUse,
    HookEvent::PreCompact,
    HookEvent::Stop,
    HookEvent::SubagentStop,
];

impl HookEvent {
    pub fn name(self) -> &'static str {
        match self {
            HookEvent::SessionStart => "SessionStart",
            HookEvent::UserPromptSubmit => "UserPromptSubmit",
            HookEvent::PreToolUse => "PreToolUse",
            HookEvent::PostToolUse => "PostToolUse",
            HookEvent::PreCompact => "PreCompact",
            HookEvent::Stop => "Stop",
            HookEvent::SubagentStop => "SubagentStop",
        }
    }

    pub fn parse(name: &str) -> Option<HookEvent> {
        EVENTS.into_iter().find(|event| event.name() == name)
    }

    /// True for the events whose `match` tests a tool name.
    pub fn has_tool(self) -> bool {
        matches!(self, HookEvent::PreToolUse | HookEvent::PostToolUse)
    }
}

/// An in-process hook of a native plugin (PLAN.md D52): the seven events
/// of [`HookEvent`] with the same input object a `hooks.json` command reads
/// on stdin and the same [`Answer`]. Runs after the file entries of its
/// event; an `Err` is traced as `hook/error` and ignored (fail open).
pub trait NativeHook: Send + Sync {
    /// Shown in traces as `native:<name>`.
    fn name(&self) -> &str;
    /// Whether the hook runs for `event` (and, for tool events, `tool`).
    fn matches(&self, event: HookEvent, tool: Option<&str>) -> bool {
        let _ = (event, tool);
        true
    }
    fn run(&self, event: HookEvent, input: &Value) -> Result<Answer, String>;
}

/// One entry of a hooks file.
#[derive(Clone, Debug, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct HookEntry {
    /// Tool name pattern (`read|mcp__*`); absent matches every tool.
    #[serde(default, rename = "match")]
    pub matcher: Option<String>,
    pub command: String,
    #[serde(default)]
    pub args: Vec<String>,
    #[serde(default)]
    pub timeout_ms: Option<u64>,
    /// The folder of the hooks file (relative commands resolve there).
    #[serde(skip)]
    pub dir: PathBuf,
}

impl HookEntry {
    pub fn timeout_ms(&self) -> u64 {
        self.timeout_ms.unwrap_or(DEFAULT_TIMEOUT_MS).max(1)
    }
}

/// Every hook of the settings, in run order (user file first).
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Hooks {
    entries: Vec<(HookEvent, HookEntry)>,
    /// The files that loaded.
    pub files: Vec<LayerFile>,
}

impl Hooks {
    /// Reads `hooks.json` of every layer folder. A missing file costs one
    /// failed read; a bad file is skipped with its reason.
    pub fn load(layers: &Layers) -> (Hooks, Vec<Skipped>) {
        let mut hooks = Hooks::default();
        let mut skipped = Vec::new();
        for (dir, source) in layers.dirs() {
            let path = dir.join(HOOKS_FILE);
            let Ok(bytes) = std::fs::read(&path) else {
                continue;
            };
            match parse(&bytes, &dir) {
                Ok(entries) => {
                    hooks.entries.extend(entries);
                    hooks.files.push(LayerFile { path, source });
                }
                Err(reason) => skipped.push(Skipped { path, reason }),
            }
        }
        (hooks, skipped)
    }

    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    /// The entries of `event`; for tool events only those whose pattern
    /// matches `tool`.
    pub fn matching<'a>(
        &'a self,
        event: HookEvent,
        tool: Option<&'a str>,
    ) -> impl Iterator<Item = &'a HookEntry> + 'a {
        (self.entries.iter())
            .filter(move |(kind, _)| *kind == event)
            .map(|(_, entry)| entry)
            .filter(move |entry| match (event.has_tool(), &entry.matcher) {
                (true, Some(pattern)) => tool.is_some_and(|name| matches(pattern, name)),
                _ => true,
            })
    }

    /// `(event, entry)` in run order (for `doctor`).
    pub fn entries(&self) -> &[(HookEvent, HookEntry)] {
        &self.entries
    }

    /// The layer of an entry's file (for `doctor`).
    pub fn source_of(&self, entry: &HookEntry) -> Source {
        let file = (self.files.iter()).find(|file| file.path.parent() == Some(&entry.dir));
        file.map_or(Source::User, |file| file.source)
    }
}

/// `{"<Event>": [entries]}`; an unknown event name makes the file bad.
fn parse(bytes: &[u8], dir: &Path) -> Result<Vec<(HookEvent, HookEntry)>, String> {
    let value: Value = serde_json::from_slice(bytes).map_err(|err| err.to_string())?;
    let Value::Object(map) = value else {
        return Err("a hooks file is a JSON object keyed by event name".into());
    };
    let mut out = Vec::new();
    for (name, list) in map {
        if name == "version" || name.starts_with('$') {
            continue;
        }
        let event =
            HookEvent::parse(&name).ok_or_else(|| format!("unknown hook event `{name}`"))?;
        let entries: Vec<HookEntry> =
            serde_json::from_value(list).map_err(|err| format!("{name}: {err}"))?;
        for mut entry in entries {
            if entry.command.trim().is_empty() {
                return Err(format!("{name}: an entry has an empty `command`"));
            }
            entry.dir = dir.to_path_buf();
            out.push((event, entry));
        }
    }
    // A JSON object has no defined order: run in the fixed event order,
    // entries of one event in file order.
    out.sort_by_key(|(event, _)| EVENTS.iter().position(|e| e == event));
    Ok(out)
}

/// `pattern` is alternatives split by `|`, each a glob where `*` is any run
/// of characters and `?` one character; empty or `*` matches everything.
pub fn matches(pattern: &str, name: &str) -> bool {
    let pattern = pattern.trim();
    if pattern.is_empty() {
        return true;
    }
    (pattern.split('|')).any(|alt| glob(alt.trim().as_bytes(), name.as_bytes()))
}

fn glob(pattern: &[u8], name: &[u8]) -> bool {
    match pattern.split_first() {
        None => name.is_empty(),
        Some((b'*', rest)) => (0..=name.len()).any(|skip| glob(rest, &name[skip..])),
        Some((b'?', rest)) => !name.is_empty() && glob(rest, &name[1..]),
        Some((c, rest)) => name.first() == Some(c) && glob(rest, &name[1..]),
    }
}

/// What one hook answered on stdout (empty: all `None`).
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Answer {
    /// `{"decision": "block", "reason"}`: the reason (never empty).
    pub block: Option<String>,
    /// PreToolUse: the arguments the tool runs with instead.
    pub arguments: Option<Value>,
    /// PostToolUse: the output that replaces the tool's.
    pub output: Option<String>,
    /// A text added as a context update at the tail.
    pub context: Option<String>,
    /// Stop: the turn goes on with this user text.
    pub cont: Option<String>,
}

impl Answer {
    /// Parses stdout: empty is a no-op, else a JSON object.
    pub fn parse(stdout: &str) -> Result<Answer, String> {
        let text = stdout.trim();
        if text.is_empty() {
            return Ok(Answer::default());
        }
        let value: Value =
            serde_json::from_str(text).map_err(|err| format!("the answer is not JSON: {err}"))?;
        let Value::Object(map) = value else {
            return Err("the answer is not a JSON object".into());
        };
        let text = |key: &str| map.get(key).and_then(Value::as_str).map(str::to_string);
        let block = match map.get("decision").and_then(Value::as_str) {
            Some("block") => Some(text("reason").unwrap_or_else(|| "blocked by a hook".into())),
            _ => None,
        };
        Ok(Answer {
            block,
            arguments: map.get("arguments").filter(|v| v.is_object()).cloned(),
            output: text("output"),
            context: text("context").filter(|t| !t.trim().is_empty()),
            cont: text("continue").filter(|t| !t.trim().is_empty()),
        })
    }

    /// The trace word: `none`, or the keys that acted joined by `,`.
    pub fn decision(&self) -> String {
        let parts = [
            (self.block.is_some(), "block"),
            (self.arguments.is_some(), "arguments"),
            (self.output.is_some(), "output"),
            (self.context.is_some(), "context"),
            (self.cont.is_some(), "continue"),
        ];
        let words: Vec<&str> = parts.iter().filter(|(on, _)| *on).map(|p| p.1).collect();
        match words.is_empty() {
            true => "none".into(),
            false => words.join(","),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn patterns_and_answers() {
        assert!(matches("read|mcp__*", "mcp__github__issue"));
        assert!(matches("read|mcp__*", "read"));
        assert!(!matches("read|mcp__*", "shell"));
        assert!(matches("", "x") && matches("*", "x") && matches("re?d", "read"));
        assert_eq!(Answer::parse("  \n").unwrap(), Answer::default());
        let answer = Answer::parse(r#"{"decision":"block","reason":"no"}"#).unwrap();
        assert_eq!(answer.block.as_deref(), Some("no"));
        assert_eq!(answer.decision(), "block");
        assert!(Answer::parse("nope").is_err() && Answer::parse("[1]").is_err());
    }

    #[test]
    fn bad_event_names_fail_the_file() {
        let dir = Path::new(".");
        assert!(parse(br#"{"PreTool": []}"#, dir).is_err());
        let ok = parse(
            br#"{"Stop":[{"command":"a"}],"PreToolUse":[{"command":"b"}]}"#,
            dir,
        );
        let events: Vec<HookEvent> = ok.unwrap().into_iter().map(|e| e.0).collect();
        assert_eq!(events, [HookEvent::PreToolUse, HookEvent::Stop]);
    }
}
