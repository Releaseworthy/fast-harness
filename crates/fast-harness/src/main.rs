//! `fast-harness`: command line front end of Fast Harness.

mod accounts_cmd;
mod args;
mod auth_choice;
mod config_cmd;
mod info;
mod models_cmd;
mod print;
mod stats_cmd;
mod tui;

use std::io::IsTerminal;
use std::path::PathBuf;
use std::process::ExitCode;

use args::Command;

fn main() -> ExitCode {
    let argv: Vec<String> = std::env::args().skip(1).collect();
    let terminal = std::io::stdin().is_terminal() && std::io::stdout().is_terminal();
    let parsed = if argv.is_empty() && terminal {
        Ok(Command::Tui(args::TuiArgs::default()))
    } else {
        args::parse(argv)
    };
    let command = match parsed {
        Ok(command) => command,
        Err(message) => {
            eprintln!("fast-harness: {message}");
            return ExitCode::from(2);
        }
    };
    let result = match command {
        Command::Version => {
            println!("Fast Harness {}", env!("CARGO_PKG_VERSION"));
            Ok(ExitCode::SUCCESS)
        }
        Command::Help => {
            println!("{}", args::USAGE);
            Ok(ExitCode::SUCCESS)
        }
        Command::Sessions { cwd } => info::sessions(cwd),
        Command::Prompt { cwd } => info::prompt(cwd),
        Command::Trace {
            id,
            steps,
            json,
            cwd,
        } => info::trace(id, steps, json, cwd),
        Command::Stats { id, all, json, cwd } => stats_cmd::run(id, all, json, cwd),
        Command::Accounts(action) => accounts_cmd::run(action),
        Command::Models { json } => models_cmd::run(json),
        Command::Config { action, cwd } => config_cmd::config(action, cwd),
        Command::Trust { path } => config_cmd::trust(path),
        Command::Doctor { cwd } => config_cmd::doctor(cwd),
        Command::Print(print_args) => print::run(print_args),
        Command::Tui(tui_args) => tui::run(tui_args),
    };
    result.unwrap_or_else(|message| {
        eprintln!("fast-harness: {message}");
        ExitCode::FAILURE
    })
}

/// `~/.fast-harness` or `FAST_HARNESS_HOME`.
pub fn home() -> Result<PathBuf, String> {
    fast_harness_core::platform::app_home()
        .ok_or_else(|| "cannot find the home folder; set FAST_HARNESS_HOME".into())
}

/// `--cwd` made absolute, or the current directory.
pub fn work_dir(cwd: Option<PathBuf>) -> Result<PathBuf, String> {
    let current = std::env::current_dir().map_err(|err| format!("current directory: {err}"))?;
    let dir = match cwd {
        Some(dir) if dir.is_absolute() => dir,
        Some(dir) => current.join(dir),
        None => current,
    };
    if !dir.is_dir() {
        return Err(format!("{} is not a directory", dir.display()));
    }
    Ok(dir)
}
