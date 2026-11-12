//! Process liveness for session locks and detached workers (PLAN.md D49).

use std::os::windows::process::CommandExt;
use std::process::{Child, Command, Stdio};

use windows_sys::Win32::Foundation::{CloseHandle, FILETIME, HANDLE};
use windows_sys::Win32::System::Threading::{
    GetExitCodeProcess, GetProcessTimes, OpenProcess, CREATE_BREAKAWAY_FROM_JOB,
    CREATE_NEW_PROCESS_GROUP, DETACHED_PROCESS, PROCESS_QUERY_LIMITED_INFORMATION,
};

use crate::platform::PlatformError;

/// `GetExitCodeProcess` value of a process that still runs.
const STILL_ACTIVE: u32 = 259;

/// The creation time of process `pid` while it runs (100 ns units since
/// 1601), so a reused pid is told apart; `None` when no such process runs
/// or it cannot be opened (another user's process).
pub fn process_start(pid: u32) -> Option<u64> {
    // SAFETY: OpenProcess takes plain values and returns a handle we own or
    // null, which is checked before any use.
    let handle = unsafe { OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, 0, pid) };
    if handle.is_null() {
        return None;
    }
    let started = query(handle);
    // SAFETY: `handle` is the valid process handle opened above, closed once.
    unsafe { CloseHandle(handle) };
    started
}

fn query(handle: HANDLE) -> Option<u64> {
    let mut code = 0u32;
    // SAFETY: `handle` is a valid process handle with query rights and
    // `code` a live out pointer for the call.
    if unsafe { GetExitCodeProcess(handle, &mut code) } == 0 || code != STILL_ACTIVE {
        return None;
    }
    let zero = FILETIME {
        dwLowDateTime: 0,
        dwHighDateTime: 0,
    };
    let (mut created, mut exited, mut kernel, mut user) = (zero, zero, zero, zero);
    // SAFETY: `handle` is valid; the four FILETIME out pointers are live locals.
    let ok = unsafe { GetProcessTimes(handle, &mut created, &mut exited, &mut kernel, &mut user) };
    let time = (u64::from(created.dwHighDateTime) << 32) | u64::from(created.dwLowDateTime);
    (ok != 0).then_some(time)
}

/// Starts `cmd` with no console and its own process group, outside the
/// caller's Job Object when the job allows it, with null stdio: the worker
/// outlives the process that started it.
pub fn spawn_detached(cmd: &mut Command) -> Result<Child, PlatformError> {
    cmd.stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null());
    let base = DETACHED_PROCESS | CREATE_NEW_PROCESS_GROUP;
    cmd.creation_flags(base | CREATE_BREAKAWAY_FROM_JOB);
    if let Ok(child) = cmd.spawn() {
        return Ok(child);
    }
    // A job without JOB_OBJECT_LIMIT_BREAKAWAY_OK refuses the breakaway.
    cmd.creation_flags(base);
    cmd.spawn().map_err(|err| PlatformError::Os {
        code: err.raw_os_error().unwrap_or(0),
        message: err.to_string(),
    })
}
