//! Hand-written argument parsing (no extra crates, PLAN.md D1).

use std::path::PathBuf;

pub const USAGE: &str = "\
nh: a coding agent harness

Usage:
  nh -p <prompt> [--model M] [--effort E] [-c|--continue] [--resume ID] [--cwd DIR]
      Run one turn in print mode: the answer on stdout, notes on stderr.
      -c continues the newest session of the directory, --resume a given one.
      Exit code: 0 done, 1 failed or step limit, 130 cancelled (Ctrl+C).
  nh sessions [--cwd DIR]          List the sessions of the directory, newest first.
  nh trace [ID|latest] [--json] [--cwd DIR]
      One row per step of a session (default latest); --json prints the raw events.
  nh --version                     Print the version.
  nh help                          Print this text.

Credentials: ANTHROPIC_API_KEY (API key) or NH_OAUTH_TOKEN (Claude subscription token).
Settings: <NH_HOME or ~/.nh>/config.json; NH_MODEL and NH_EFFORT override it.";

#[derive(Debug, Default, PartialEq)]
pub struct PrintArgs {
    pub prompt: String,
    pub model: Option<String>,
    pub effort: Option<String>,
    pub continue_latest: bool,
    pub resume: Option<String>,
    pub cwd: Option<PathBuf>,
}

#[derive(Debug, PartialEq)]
pub enum Command {
    Print(PrintArgs),
    Sessions {
        cwd: Option<PathBuf>,
    },
    Trace {
        id: Option<String>,
        json: bool,
        cwd: Option<PathBuf>,
    },
    Version,
    Help,
}

#[derive(Default)]
struct Opts {
    print: PrintArgs,
    prompt: Option<String>,
    json: bool,
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
            "-h" | "--help" => opts.help = true,
            "-V" | "--version" => opts.version = true,
            flag if flag.starts_with('-') && flag.len() > 1 => {
                return Err(format!("unknown option `{flag}` (see `nh help`)"))
            }
            _ => opts.positional.push(arg),
        }
    }
    Ok(opts)
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
        Some("trace") if opts.positional.len() <= 2 => {
            let id = opts.positional.get(1).cloned();
            let json = opts.json;
            return Ok(Command::Trace { id, json, cwd });
        }
        Some(other) => return Err(format!("unexpected argument `{other}` (see `nh help`)")),
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
                json: true,
                cwd: None
            }
        );
        assert!(parse_str("-p").is_err());
        assert!(parse_str("--bogus").is_err());
        assert!(parse_str("frob").is_err());
        assert!(parse_str("-p x -c --resume y").is_err());
    }
}
