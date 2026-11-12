//! `fast-harness reflect [ID] [--apply]` (PLAN.md D50): prints the proposed
//! diff; `--apply` applies it, `"selfImprove": "auto"` applies the memory
//! edits, `"off"` refuses.

use std::path::PathBuf;
use std::process::ExitCode;

use crate::auth_choice::runtime;

pub fn run(id: Option<String>, apply: bool, cwd: Option<PathBuf>) -> Result<ExitCode, String> {
    let rt = runtime()?;
    let cwd = crate::work_dir(cwd)?;
    let args = if apply { "--apply" } else { "" };
    let job = rt.reflect_job(&cwd, id.as_deref(), args)?;
    println!("{}", job()?);
    Ok(ExitCode::SUCCESS)
}
