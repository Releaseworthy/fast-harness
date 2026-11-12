//! Arguments of the session coordination commands (PLAN.md D49): `send`,
//! `ps`, `stop`, `pause`, `wait`, `tail` and `start`.

/// One session coordination command.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum SessionAction {
    Send {
        id: String,
        text: String,
        queue: bool,
        wait: bool,
    },
    Ps {
        json: bool,
    },
    Stop {
        id: String,
    },
    Pause {
        id: String,
    },
    Wait {
        id: String,
    },
    Tail {
        id: String,
        json: bool,
    },
    Start {
        prompt: String,
        model: Option<String>,
    },
}

/// The flags these commands read.
#[derive(Default)]
pub struct Flags {
    pub steer: bool,
    pub queue: bool,
    pub wait: bool,
    pub json: bool,
    pub model: Option<String>,
}

/// `verb` and the positional arguments after it.
pub fn parse(verb: &str, rest: &[String], flags: Flags) -> Result<SessionAction, String> {
    let id = || {
        rest.first()
            .cloned()
            .ok_or_else(|| format!("usage: fast-harness {verb} <ID|latest>"))
    };
    let one = |action: SessionAction| match rest.len() {
        1 => Ok(action),
        _ => Err(format!("usage: fast-harness {verb} <ID|latest>")),
    };
    match verb {
        "send" => {
            if flags.steer && flags.queue {
                return Err("use either --steer or --queue, not both".into());
            }
            let text = rest.get(1..).unwrap_or_default().join(" ");
            if text.trim().is_empty() {
                return Err(
                    "usage: fast-harness send <ID|latest> <text> [--steer|--queue] [--wait]".into(),
                );
            }
            let (queue, wait) = (flags.queue, flags.wait);
            Ok(SessionAction::Send {
                id: id()?,
                text,
                queue,
                wait,
            })
        }
        "ps" if rest.is_empty() => Ok(SessionAction::Ps { json: flags.json }),
        "stop" => one(SessionAction::Stop { id: id()? }),
        "pause" => one(SessionAction::Pause { id: id()? }),
        "wait" => one(SessionAction::Wait { id: id()? }),
        "tail" => one(SessionAction::Tail {
            id: id()?,
            json: flags.json,
        }),
        "start" => {
            let prompt = rest.join(" ");
            if prompt.trim().is_empty() {
                return Err("usage: fast-harness start [--model M] [--cwd DIR] <prompt>".into());
            }
            Ok(SessionAction::Start {
                prompt,
                model: flags.model,
            })
        }
        _ => Err(format!(
            "unexpected arguments after `{verb}` (see `fast-harness help`)"
        )),
    }
}

/// The verbs [`parse`] handles.
pub fn is_verb(word: &str) -> bool {
    matches!(
        word,
        "send" | "ps" | "stop" | "pause" | "wait" | "tail" | "start"
    )
}
