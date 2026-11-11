//! `nh`: command line front end of the native harness.

mod args;
mod auth_choice;
mod info;
mod print;

use std::path::PathBuf;
use std::process::ExitCode;

use args::Command;

fn main() -> ExitCode {
    let command = match args::parse(std::env::args().skip(1).collect()) {
        Ok(command) => command,
        Err(message) => {
            eprintln!("nh: {message}");
            return ExitCode::from(2);
        }
    };
    let result = match command {
        Command::Version => {
            println!("nh {}", env!("CARGO_PKG_VERSION"));
            Ok(ExitCode::SUCCESS)
        }
        Command::Help => {
            println!("{}", args::USAGE);
            Ok(ExitCode::SUCCESS)
        }
        Command::Sessions { cwd } => info::sessions(cwd),
        Command::Trace { id, json, cwd } => info::trace(id, json, cwd),
        Command::Print(print_args) => print::run(print_args),
    };
    result.unwrap_or_else(|message| {
        eprintln!("nh: {message}");
        ExitCode::FAILURE
    })
}

/// `~/.nh` or `NH_HOME`.
pub fn home() -> Result<PathBuf, String> {
    nh_core::platform::nh_home().ok_or_else(|| "cannot find the home folder; set NH_HOME".into())
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
