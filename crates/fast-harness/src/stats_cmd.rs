//! `fast-harness stats`: context and cache numbers of session logs (PLAN.md D35).

use std::path::{Path, PathBuf};
use std::process::ExitCode;

use fast_harness_core::session;
use fast_harness_core::stats::{self, Stats};
use fast_harness_core::trace;

fn of_session(home: &Path, cwd: &Path, id: &str) -> Result<Stats, String> {
    let path = session::log_path(home, cwd, id);
    if !path.is_file() {
        return Err(format!("no session {id} for {}", cwd.display()));
    }
    let (_, events) = trace::read(&path).map_err(|err| format!("{}: {err}", path.display()))?;
    Ok(stats::compute(id, &events))
}

/// Every session of `cwd` merged; an unreadable log is skipped with a note.
fn of_all(home: &Path, cwd: &Path) -> Result<Stats, String> {
    let list = session::list(home, cwd).map_err(|err| format!("sessions: {err}"))?;
    let mut total = Stats::default();
    for item in list {
        match of_session(home, cwd, &item.id) {
            Ok(one) => total.merge(one),
            Err(err) => eprintln!("[note] skipped: {err}"),
        }
    }
    Ok(total)
}

pub fn run(
    id: Option<String>,
    all: bool,
    json: bool,
    cwd: Option<PathBuf>,
) -> Result<ExitCode, String> {
    let (home, cwd) = (crate::home()?, crate::work_dir(cwd)?);
    let (title, numbers) = if all {
        (
            format!("all sessions of {}", cwd.display()),
            of_all(&home, &cwd)?,
        )
    } else {
        let id = crate::info::resolve(&home, &cwd, id)?;
        (format!("session {id}"), of_session(&home, &cwd, &id)?)
    };
    if json {
        let text = serde_json::to_string_pretty(&numbers).map_err(|err| err.to_string())?;
        println!("{text}");
    } else {
        println!("{title}");
        print!("{}", stats::format_text(&numbers));
    }
    Ok(ExitCode::SUCCESS)
}
