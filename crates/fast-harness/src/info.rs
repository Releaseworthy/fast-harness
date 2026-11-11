//! `fast-harness sessions` and `fast-harness trace`.

use std::path::{Path, PathBuf};
use std::process::ExitCode;

use fast_harness_core::session;
use fast_harness_core::trace::{self, detail, report};

pub fn sessions(cwd: Option<PathBuf>) -> Result<ExitCode, String> {
    let (home, cwd) = (crate::home()?, crate::work_dir(cwd)?);
    let list = session::list(&home, &cwd).map_err(|err| format!("sessions: {err}"))?;
    if list.is_empty() {
        eprintln!("no sessions for {}", cwd.display());
    }
    for item in list {
        let created = session::utc_stamp(item.created_at / 1000);
        println!(
            "{}  {created}Z  {:>3} turns  {}",
            item.id, item.turns, item.title
        );
    }
    Ok(ExitCode::SUCCESS)
}

/// The id itself, or the newest session for `None` and `latest`.
fn resolve(home: &Path, cwd: &Path, id: Option<String>) -> Result<String, String> {
    match id.filter(|id| id != "latest") {
        Some(id) => Ok(id),
        None => session::latest(home, cwd)
            .map_err(|err| format!("sessions: {err}"))?
            .map(|summary| summary.id)
            .ok_or_else(|| format!("no sessions for {}", cwd.display())),
    }
}

/// The timeline and the step table; `steps` prints only the table, `json`
/// the raw header and events.
pub fn trace(
    id: Option<String>,
    steps: bool,
    json: bool,
    cwd: Option<PathBuf>,
) -> Result<ExitCode, String> {
    let (home, cwd) = (crate::home()?, crate::work_dir(cwd)?);
    let id = resolve(&home, &cwd, id)?;
    let path = session::log_path(&home, &cwd, &id);
    if !path.is_file() {
        return Err(format!("no session {id} for {}", cwd.display()));
    }
    let (header, events) =
        trace::read(&path).map_err(|err| format!("{}: {err}", path.display()))?;
    if json {
        let line = serde_json::to_string(&header).map_err(|err| err.to_string())?;
        println!("{line}");
        for event in &events {
            let line = serde_json::to_string(event).map_err(|err| err.to_string())?;
            println!("{line}");
        }
    } else {
        println!("session {id}");
        if !steps {
            print!("{}", detail::format_text(&detail::entries(&events)));
            println!();
        }
        print!("{}", report::format_table(&report::step_rows(&events)));
    }
    Ok(ExitCode::SUCCESS)
}
