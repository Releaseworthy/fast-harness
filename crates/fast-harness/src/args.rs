//! Hand-written argument parsing (no extra crates, PLAN.md D1).

use std::path::PathBuf;

pub const USAGE: &str = "\
Fast Harness: a coding agent harness

Usage:
  fast-harness -p <prompt> [--model M] [--effort E] [-c|--continue] [--resume ID] [--cwd DIR]
      Run one turn in print mode: the answer on stdout, notes on stderr.
      -c continues the newest session of the directory, --resume a given one.
      Exit code: 0 done, 1 failed or step limit, 130 cancelled (Ctrl+C).
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
    Accounts(AccountsAction),
    Models {
        json: bool,
    },
    Version,
    Help,
}

#[derive(Default)]
struct Opts {
    print: PrintArgs,
    prompt: Option<String>,
    json: bool,
    steps: bool,
    no_browser: bool,
    google: bool,
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
            "--no-browser" => opts.no_browser = true,
            "--google" => opts.google = true,
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
            Ok(Command::Print(opts.print))
        }
        Some(_) => Err("the prompt is empty".into()),
        None => Ok(Command::Help),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn parse_str(line: &str) -> Result<Command, String> {
        parse(line.split_whitespace().map(String::from).collect())
    }

    #[test]
    fn parses_commands() {
        let Command::Print(print) = parse_str("-p hi --model m --effort low -c --cwd x").unwrap()
        else {
            panic!("not print mode");
        };
        assert_eq!(print.prompt, "hi");
        assert_eq!(print.model.as_deref(), Some("m"));
        assert_eq!(print.effort.as_deref(), Some("low"));
        assert!(print.continue_latest);
        assert_eq!(print.cwd, Some(PathBuf::from("x")));
        assert_eq!(parse_str("--version").unwrap(), Command::Version);
        assert_eq!(parse_str("help").unwrap(), Command::Help);
        assert_eq!(parse_str("").unwrap(), Command::Help);
        assert_eq!(
            parse_str("sessions").unwrap(),
            Command::Sessions { cwd: None }
        );
        let trace = parse_str("trace latest --json").unwrap();
        let id = Some("latest".to_string());
        assert_eq!(
            trace,
            Command::Trace {
                id,
                steps: false,
                json: true,
                cwd: None
            }
        );
        assert_eq!(
            parse_str("trace --steps --cwd d").unwrap(),
            Command::Trace {
                id: None,
                steps: true,
                json: false,
                cwd: Some(PathBuf::from("d"))
            }
        );
        assert!(parse_str("trace a b").is_err());
        assert!(parse_str("-p").is_err());
        assert!(parse_str("--bogus").is_err());
        assert!(parse_str("frob").is_err());
        assert!(parse_str("-p x -c --resume y").is_err());
        let tui = parse_str("tui -c --cwd d").unwrap();
        let expected = TuiArgs {
            continue_latest: true,
            resume: None,
            cwd: Some(PathBuf::from("d")),
        };
        assert_eq!(tui, Command::Tui(expected));
        assert!(parse_str("tui -c --resume x").is_err());
        assert!(parse_str("tui -p hi").is_err());
        assert!(parse_str("tui extra").is_err());
    }

    #[test]
    fn parses_accounts() {
        let accounts = |line: &str| match parse_str(line) {
            Ok(Command::Accounts(action)) => Ok(action),
            Ok(other) => panic!("not accounts: {other:?}"),
            Err(err) => Err(err),
        };
        assert_eq!(accounts("accounts").unwrap(), AccountsAction::List);
        assert_eq!(accounts("accounts list").unwrap(), AccountsAction::List);
        let login = accounts("accounts login --no-browser").unwrap();
        assert_eq!(login, AccountsAction::Login { browser: false });
        assert_eq!(
            accounts("accounts use u-1").unwrap(),
            AccountsAction::Use("u-1".into())
        );
        let remove = accounts("accounts remove a@b.c").unwrap();
        assert_eq!(remove, AccountsAction::Remove("a@b.c".into()));
        let key = accounts("accounts add-key work").unwrap();
        assert_eq!(
            key,
            AccountsAction::AddKey {
                label: Some("work".into())
            }
        );
        assert_eq!(accounts("accounts usage").unwrap(), AccountsAction::Usage);
        let google = accounts("accounts login --google --no-browser").unwrap();
        assert_eq!(google, AccountsAction::LoginGoogle { browser: false });
        let google = accounts("accounts login --google").unwrap();
        assert_eq!(google, AccountsAction::LoginGoogle { browser: true });
        let client = accounts("accounts google-client --id test-client.apps.example").unwrap();
        let id = "test-client.apps.example".to_string();
        assert_eq!(client, AccountsAction::GoogleClient { id });
        assert!(accounts("accounts google-client").is_err());
        assert!(accounts("accounts use u-1 --google").is_err());
        assert!(accounts("accounts list --id x").is_err());
        assert!(accounts("accounts use").is_err());
        assert!(accounts("accounts frob").is_err());
        assert!(accounts("accounts use a b").is_err());
    }

    #[test]
    fn parses_models() {
        assert_eq!(
            parse_str("models").unwrap(),
            Command::Models { json: false }
        );
        let json = parse_str("models --json").unwrap();
        assert_eq!(json, Command::Models { json: true });
        assert!(parse_str("models extra").is_err());
    }

    #[test]
    fn parses_prompt() {
        let cwd = Some(PathBuf::from("d"));
        assert_eq!(parse_str("prompt").unwrap(), Command::Prompt { cwd: None });
        assert_eq!(
            parse_str("prompt --cwd d").unwrap(),
            Command::Prompt { cwd }
        );
        assert!(parse_str("prompt extra").is_err());
        assert!(USAGE.contains("fast-harness prompt [--cwd DIR]"));
    }
}
