//! Hand-written argument parsing (no extra crates, PLAN.md D1).

use std::path::PathBuf;

use crate::args_sessions::{self, SessionAction};

pub const USAGE: &str = "\
Fast Harness: a coding agent harness

Usage:
  fast-harness -p <prompt> [--json] [--model M] [--effort E] [-c|--continue] [--resume ID] [--cwd DIR]
      Run one turn in print mode: the answer on stdout, notes on stderr.
      -c continues the newest session of the directory, --resume a given one.
      --json writes every event as a JSON line, the result last (docs/service.md).
      Exit code: 0 done, 1 failed or step limit, 130 cancelled (Ctrl+C).
  fast-harness serve --stdio [--model M] [--effort E]
      JSON-RPC 2.0 on stdin/stdout: Agent Client Protocol v2 with _fh/ extensions
      (docs/service.md); several sessions at once. Logs go to stderr.
  fast-harness [tui] [-c|--resume ID] [--cwd DIR]
      The full-screen chat (`fast-harness` alone on a terminal). F2 accounts, F3 trace, F4 model.
  fast-harness sessions [--cwd DIR]          List the sessions of the directory, newest first.
  fast-harness prompt [--cwd DIR]
      The system prompt file that applies to the directory (prompts/<slug>.md, else
      system-prompt.md, else the built-in default) and the rendered prompt.
  fast-harness trace [ID|latest] [--steps] [--json] [--cwd DIR]
      A session (default latest): what the user sent, each request, the model's thinking,
      answers and tool calls, then one row per step. --steps prints only the step table,
      --json the raw events.
  fast-harness reflect [ID|latest] [--apply] [--cwd DIR]
      A subagent on the cheapest configured model (reflect.model) reads the stats and a
      digest of the session's trace and prints a diff of memory, AGENTS.md, skill and tool
      edits; --apply applies it (selfImprove auto applies the memory edits, off refuses).
  fast-harness stats [ID|latest|--all] [--json] [--cwd DIR]
      Context and cache numbers of a session (default latest) or of every session of the
      directory: prompt tokens, cache hit rate, cache rewrites after an idle gap, tool result
      bytes per tool, the largest and clipped results, find_tools steps, transformations.
  fast-harness compact [ID|latest] [--cwd DIR]
      Summarize a session (default latest) into a fresh history; the log keeps the full
      transcript. /compact [instructions] does the same in the TUI and the GUI.
  fast-harness models [--json]
      The Claude models with their effort levels, read live from the API (else the last
      known list); --json prints the full entries.
  fast-harness accounts [list]               Accounts with their 5-hour and 7-day usage (* = active);
      Google accounts with their plan and project.
  fast-harness accounts login [--no-browser] Sign in with a Claude account in the browser.
  fast-harness accounts login --google [--no-browser]
      Sign in with a Google Antigravity account in the browser.
  fast-harness accounts google-client --id ID
      Save the Google OAuth client (the secret is read from stdin, stored encrypted).
  fast-harness accounts use ID|EMAIL         Make an account the active one.
  fast-harness accounts remove ID|EMAIL      Remove an account from fast-harness.
  fast-harness accounts add-key [LABEL]      Add an Anthropic API key (read from stdin).
  fast-harness accounts usage                Read the usage of every account now.
  fast-harness config path|get KEY|set KEY VALUE [--project] [--cwd DIR]
      The settings files; a key's value and the layer it came from (default, user, project,
      env, command line); set writes the user config.json (--project: the project's). KEY may
      be dotted (compaction.trigger); VALUE is JSON, else a string.
  fast-harness trust [PATH]                  Trust the project of PATH (default the current
      directory): its .fast-harness folder (config.json, providers/) loads from then on.
  fast-harness doctor [--cwd DIR]            What loaded, from where, and what was skipped and why.
  fast-harness keys [--json] [--cwd DIR]     Every TUI and GUI action with its keys and the layer
      that set them (keys.json in the user and trusted project folders; null unbinds).
  fast-harness mcp list|refresh [SERVER] [--cwd DIR]
      The MCP servers of mcp.json with their cached tool lists; refresh starts each enabled
      server (or SERVER), saves its tool list in mcp-cache/ and stops it.
  fast-harness send <ID|latest> <text> [--steer|--queue] [--wait] [--cwd DIR]
      Text for a session: a running turn (any front end, any process) reads it at its next
      step (--steer, the default) or after it ends (--queue); --wait waits for the turn end
      and prints the last answer. A session nobody runs runs the turn here (the answer on stdout).
  fast-harness ps [--json]                   Running sessions: id, state, turn, step, pid, directory.
  fast-harness stop|pause <ID|latest>        End a running turn at its next step (pause: after the step's tools).
  fast-harness wait <ID|latest>              Wait until the session's turn ends.
  fast-harness tail <ID|latest> [--json]     The session's log as text lines (--json: raw events),
      followed while a turn runs.
  fast-harness start [--model M] [--cwd DIR] <prompt>
      A new session whose turn runs in a detached worker process; prints the session id.
  fast-harness --version                     Print the version.
  fast-harness help                          Print this text.

Credentials: ANTHROPIC_API_KEY or FAST_HARNESS_OAUTH_TOKEN when set, else the accounts of `fast-harness accounts`
(switched automatically on rate limits).
Settings: <FAST_HARNESS_HOME or ~/.fast-harness>/config.json (model, effort, theme, icons, wire); icons is
nerd (Nerd Font, the default) or plain. FAST_HARNESS_MODEL, FAST_HARNESS_EFFORT, FAST_HARNESS_THEME, FAST_HARNESS_ICONS and FAST_HARNESS_WIRE
override it. Raw wire capture (<session>.wire/) is off unless wire is true.";

#[derive(Debug, PartialEq)]
pub enum AccountsAction {
    List,
    Login { browser: bool },
    LoginGoogle { browser: bool },
    GoogleClient { id: String },
    Use(String),
    Remove(String),
    AddKey { label: Option<String> },
    Usage,
}

const ACCOUNTS_USAGE: &str =
    "usage: fast-harness accounts [list | login [--google] [--no-browser] | google-client --id ID \
| use ID | remove ID | add-key [LABEL] | usage]";

fn accounts_action(rest: &[String], opts: &Opts) -> Result<AccountsAction, String> {
    let first = rest.first().map(String::as_str);
    if rest.len() > 2 || (opts.google && first != Some("login")) {
        return Err(ACCOUNTS_USAGE.into());
    }
    if opts.id.is_some() != (first == Some("google-client")) {
        return Err(ACCOUNTS_USAGE.into());
    }
    let browser = !opts.no_browser;
    let arg = rest.get(1).cloned();
    Ok(match (first, arg) {
        (None | Some("list"), None) => AccountsAction::List,
        (Some("login"), None) if opts.google => AccountsAction::LoginGoogle { browser },
        (Some("login"), None) => AccountsAction::Login { browser },
        (Some("google-client"), None) => AccountsAction::GoogleClient {
            id: opts.id.clone().unwrap_or_default(),
        },
        (Some("use"), Some(id)) => AccountsAction::Use(id),
        (Some("remove"), Some(id)) => AccountsAction::Remove(id),
        (Some("add-key"), label) => AccountsAction::AddKey { label },
        (Some("usage"), None) => AccountsAction::Usage,
        _ => return Err(ACCOUNTS_USAGE.into()),
    })
}

#[derive(Debug, Default, PartialEq)]
pub struct PrintArgs {
    pub prompt: String,
    pub model: Option<String>,
    pub effort: Option<String>,
    pub continue_latest: bool,
    pub resume: Option<String>,
    pub cwd: Option<PathBuf>,
    /// `--json`: the events as JSON lines (PLAN.md D48).
    pub json: bool,
}

#[derive(Debug, Default, PartialEq)]
pub struct TuiArgs {
    pub continue_latest: bool,
    pub resume: Option<String>,
    pub cwd: Option<PathBuf>,
}

#[derive(Debug, PartialEq)]
pub enum Command {
    Print(PrintArgs),
    Tui(TuiArgs),
    Sessions {
        cwd: Option<PathBuf>,
    },
    Prompt {
        cwd: Option<PathBuf>,
    },
    Trace {
        id: Option<String>,
        steps: bool,
        json: bool,
        cwd: Option<PathBuf>,
    },
    Stats {
        id: Option<String>,
        all: bool,
        json: bool,
        cwd: Option<PathBuf>,
    },
    /// `reflect [ID] [--apply]` (PLAN.md D50).
    Reflect {
        id: Option<String>,
        apply: bool,
        cwd: Option<PathBuf>,
    },
    Accounts(AccountsAction),
    Models {
        json: bool,
    },
    Config {
        action: ConfigAction,
        cwd: Option<PathBuf>,
    },
    Trust {
        path: Option<PathBuf>,
    },
    Doctor {
        cwd: Option<PathBuf>,
    },
    Keys {
        json: bool,
        cwd: Option<PathBuf>,
    },
    /// `mcp list` or `mcp refresh [SERVER]` (PLAN.md D47).
    Mcp {
        refresh: bool,
        server: Option<String>,
        cwd: Option<PathBuf>,
    },
    /// `serve --stdio` (PLAN.md D48).
    Serve {
        model: Option<String>,
        effort: Option<String>,
    },
    /// `send`, `ps`, `stop`, `pause`, `wait`, `tail`, `start` (PLAN.md D49).
    Session {
        action: SessionAction,
        cwd: Option<PathBuf>,
    },
    Version,
    Help,
}

/// `fast-harness config` actions (PLAN.md D41).
#[derive(Debug, PartialEq, Eq)]
pub enum ConfigAction {
    Path,
    Get(String),
    Set {
        key: String,
        value: String,
        project: bool,
    },
}

const CONFIG_USAGE: &str = "usage: fast-harness config path|get KEY|set KEY VALUE [--project]";

fn config_action(args: &[String], project: bool) -> Result<ConfigAction, String> {
    match args {
        [verb] if verb == "path" => Ok(ConfigAction::Path),
        [verb, key] if verb == "get" => Ok(ConfigAction::Get(key.clone())),
        [verb, key, value] if verb == "set" => Ok(ConfigAction::Set {
            key: key.clone(),
            value: value.clone(),
            project,
        }),
        _ => Err(CONFIG_USAGE.into()),
    }
}

#[derive(Default)]
struct Opts {
    print: PrintArgs,
    prompt: Option<String>,
    json: bool,
    steps: bool,
    all: bool,
    apply: bool,
    no_browser: bool,
    google: bool,
    project: bool,
    stdio: bool,
    steer: bool,
    queue: bool,
    wait: bool,
    id: Option<String>,
    help: bool,
    version: bool,
    positional: Vec<String>,
}

fn value(args: &mut impl Iterator<Item = String>, flag: &str) -> Result<String, String> {
    args.next().ok_or_else(|| format!("{flag} needs a value"))
}

fn read_opts(args: Vec<String>) -> Result<Opts, String> {
    let mut opts = Opts::default();
    let mut args = args.into_iter();
    while let Some(arg) = args.next() {
        match arg.as_str() {
            "-p" | "--prompt" => opts.prompt = Some(value(&mut args, &arg)?),
            "--model" => opts.print.model = Some(value(&mut args, &arg)?),
            "--effort" => opts.print.effort = Some(value(&mut args, &arg)?),
            "--resume" => opts.print.resume = Some(value(&mut args, &arg)?),
            "--cwd" => opts.print.cwd = Some(PathBuf::from(value(&mut args, &arg)?)),
            "-c" | "--continue" => opts.print.continue_latest = true,
            "--json" => opts.json = true,
            "--steps" => opts.steps = true,
            "--all" => opts.all = true,
            "--apply" => opts.apply = true,
            "--no-browser" => opts.no_browser = true,
            "--google" => opts.google = true,
            "--project" => opts.project = true,
            "--stdio" => opts.stdio = true,
            "--steer" => opts.steer = true,
            "--queue" => opts.queue = true,
            "--wait" => opts.wait = true,
            "--id" => opts.id = Some(value(&mut args, &arg)?),
            "-h" | "--help" => opts.help = true,
            "-V" | "--version" => opts.version = true,
            flag if flag.starts_with('-') && flag.len() > 1 => {
                return Err(format!("unknown option `{flag}` (see `fast-harness help`)"))
            }
            _ => opts.positional.push(arg),
        }
    }
    Ok(opts)
}

fn tui_args(opts: Opts) -> Result<Command, String> {
    if opts.prompt.is_some() {
        return Err("fast-harness tui takes no prompt; use fast-harness -p for print mode".into());
    }
    if opts.print.continue_latest && opts.print.resume.is_some() {
        return Err("use either --continue or --resume, not both".into());
    }
    Ok(Command::Tui(TuiArgs {
        continue_latest: opts.print.continue_latest,
        resume: opts.print.resume,
        cwd: opts.print.cwd,
    }))
}

/// `mcp list` or `mcp refresh [SERVER]` (PLAN.md D47).
fn mcp_command(args: &[String], cwd: Option<PathBuf>) -> Result<Command, String> {
    let (refresh, server) = match args {
        [verb] if verb == "list" => (false, None),
        [verb] if verb == "refresh" => (true, None),
        [verb, name] if verb == "refresh" => (true, Some(name.clone())),
        _ => return Err("usage: fast-harness mcp list|refresh [SERVER]".into()),
    };
    Ok(Command::Mcp {
        refresh,
        server,
        cwd,
    })
}

/// `send`, `ps`, `stop`, `pause`, `wait`, `tail`, `start` (PLAN.md D49).
fn session_command(opts: Opts) -> Result<Command, String> {
    let flags = args_sessions::Flags {
        steer: opts.steer,
        queue: opts.queue,
        wait: opts.wait,
        json: opts.json,
        model: opts.print.model,
    };
    let action = args_sessions::parse(&opts.positional[0], &opts.positional[1..], flags)?;
    let cwd = opts.print.cwd;
    Ok(Command::Session { action, cwd })
}

/// Parses the arguments after the program name.
pub fn parse(args: Vec<String>) -> Result<Command, String> {
    let mut opts = read_opts(args)?;
    if opts.version {
        return Ok(Command::Version);
    }
    let first = opts.positional.first().map(String::as_str);
    if opts.help || first == Some("help") {
        return Ok(Command::Help);
    }
    let cwd = opts.print.cwd.clone();
    match first {
        Some("sessions") if opts.positional.len() == 1 => return Ok(Command::Sessions { cwd }),
        Some("prompt") if opts.positional.len() == 1 => return Ok(Command::Prompt { cwd }),
        Some("tui") if opts.positional.len() == 1 => return tui_args(opts),
        Some("config") => {
            let action = config_action(&opts.positional[1..], opts.project)?;
            return Ok(Command::Config { action, cwd });
        }
        Some("trust") if opts.positional.len() <= 2 => {
            let path = opts.positional.get(1).map(PathBuf::from);
            return Ok(Command::Trust { path });
        }
        Some("doctor") if opts.positional.len() == 1 => return Ok(Command::Doctor { cwd }),
        Some("mcp") => return mcp_command(&opts.positional[1..], cwd),
        Some(verb) if args_sessions::is_verb(verb) => return session_command(opts),
        Some("serve") if opts.positional.len() == 1 => {
            if !opts.stdio {
                return Err("usage: fast-harness serve --stdio (the only transport)".into());
            }
            let (model, effort) = (opts.print.model, opts.print.effort);
            return Ok(Command::Serve { model, effort });
        }
        Some("keys") if opts.positional.len() == 1 => {
            return Ok(Command::Keys {
                json: opts.json,
                cwd,
            })
        }
        Some("accounts") => {
            let action = accounts_action(&opts.positional[1..], &opts)?;
            return Ok(Command::Accounts(action));
        }
        Some("trace") if opts.positional.len() <= 2 => {
            let id = opts.positional.get(1).cloned();
            let (steps, json) = (opts.steps, opts.json);
            return Ok(Command::Trace {
                id,
                steps,
                json,
                cwd,
            });
        }
        Some("stats") if opts.positional.len() <= 2 => {
            let id = opts.positional.get(1).cloned();
            if opts.all && id.is_some() {
                return Err("use either a session id or --all, not both".into());
            }
            let (all, json) = (opts.all, opts.json);
            return Ok(Command::Stats { id, all, json, cwd });
        }
        Some("reflect") if opts.positional.len() <= 2 => {
            let id = opts.positional.get(1).filter(|id| *id != "latest").cloned();
            let apply = opts.apply;
            return Ok(Command::Reflect { id, apply, cwd });
        }
        // `compact [ID|latest]`: a print-mode turn running `/compact` (D38).
        Some("compact") if opts.positional.len() <= 2 => {
            let id = opts.positional.get(1).filter(|id| *id != "latest").cloned();
            return Ok(Command::Print(PrintArgs {
                prompt: "/compact".into(),
                continue_latest: id.is_none(),
                resume: id,
                ..opts.print
            }));
        }
        Some("models") if opts.positional.len() == 1 => {
            return Ok(Command::Models { json: opts.json })
        }
        Some(other) => {
            return Err(format!(
                "unexpected argument `{other}` (see `fast-harness help`)"
            ))
        }
        None => {}
    }
    match opts.prompt.take() {
        Some(prompt) if !prompt.trim().is_empty() => {
            if opts.print.continue_latest && opts.print.resume.is_some() {
                return Err("use either --continue or --resume, not both".into());
            }
            opts.print.prompt = prompt;
            opts.print.json = opts.json;
            Ok(Command::Print(opts.print))
        }
        Some(_) => Err("the prompt is empty".into()),
        None => Ok(Command::Help),
    }
}

#[cfg(test)]
#[path = "args_tests.rs"]
mod tests;
