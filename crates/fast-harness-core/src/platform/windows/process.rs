//! Process trees killed as one unit (a Job Object).
//!
//! The child starts suspended, joins a Job Object created with
//! `JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE`, and only then runs, so every process
//! it starts is in the job too. `kill` terminates the job; dropping the group
//! closes the job handle, which also kills whatever is still running in it.

use std::os::windows::io::AsRawHandle;
use std::os::windows::process::CommandExt;
use std::process::{Child, Command};

use windows_sys::Win32::Foundation::{CloseHandle, HANDLE, INVALID_HANDLE_VALUE};
use windows_sys::Win32::System::Diagnostics::ToolHelp::{
    CreateToolhelp32Snapshot, Thread32First, Thread32Next, TH32CS_SNAPTHREAD, THREADENTRY32,
};
use windows_sys::Win32::System::JobObjects::{
    AssignProcessToJobObject, CreateJobObjectW, JobObjectExtendedLimitInformation,
    SetInformationJobObject, TerminateJobObject, JOBOBJECT_EXTENDED_LIMIT_INFORMATION,
    JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE,
};
use windows_sys::Win32::System::Threading::{
    OpenThread, ResumeThread, CREATE_NO_WINDOW, CREATE_SUSPENDED, THREAD_SUSPEND_RESUME,
};

use crate::platform::PlatformError;

/// Exit code given to processes killed through [`ProcessGroup::kill`].
const KILLED_EXIT_CODE: u32 = 1;

/// A spawned process and every process it starts.
pub struct ProcessGroup {
    /// The Job Object handle; owned, closed exactly once in `Drop`.
    job: HANDLE,
}

// SAFETY: a Job Object handle is a kernel handle usable from any thread; the
// group only passes it to thread-safe Win32 calls and closes it once in Drop.
unsafe impl Send for ProcessGroup {}
// SAFETY: see `Send`; `kill` takes `&self` and TerminateJobObject is thread-safe.
unsafe impl Sync for ProcessGroup {}

impl ProcessGroup {
    /// Spawns `cmd` inside a new group; processes it starts join the group.
    /// Sets the creation flags of `cmd` (suspended, no console window).
    pub fn spawn(cmd: &mut Command) -> Result<(ProcessGroup, Child), PlatformError> {
        let group = ProcessGroup::new_job()?;
        cmd.creation_flags(CREATE_SUSPENDED | CREATE_NO_WINDOW);
        let mut child = cmd.spawn().map_err(io_error)?;
        let process = child.as_raw_handle() as HANDLE;
        // SAFETY: `group.job` is a valid job handle we own and `process` is the
        // live process handle owned by `child`, which outlives this call.
        let assigned = unsafe { AssignProcessToJobObject(group.job, process) };
        if assigned == 0 {
            let err = PlatformError::last_os_error();
            abandon(&mut child);
            return Err(err);
        }
        if let Err(err) = resume_threads(child.id()) {
            let _ = group.kill();
            let _ = child.wait();
            return Err(err);
        }
        Ok((group, child))
    }

    /// Ends the group after the child is done: closing the job kills the
    /// processes the child left running, so the pipes they hold close.
    pub fn close(self) {
        drop(self);
    }

    /// Kills what is left of the group after the child exited (a background
    /// job's leftovers), so the pipes they hold close.
    pub fn kill_rest(&self) {
        let _ = self.kill();
    }

    /// Kills every process in the group.
    pub fn kill(&self) -> Result<(), PlatformError> {
        // SAFETY: `self.job` is a valid job handle owned by `self`.
        let done = unsafe { TerminateJobObject(self.job, KILLED_EXIT_CODE) };
        if done == 0 {
            return Err(PlatformError::last_os_error());
        }
        Ok(())
    }

    /// Creates an anonymous job that kills its processes when its last
    /// handle closes.
    fn new_job() -> Result<ProcessGroup, PlatformError> {
        // SAFETY: null attributes and a null name are documented as valid.
        let job = unsafe { CreateJobObjectW(std::ptr::null(), std::ptr::null()) };
        if job.is_null() {
            return Err(PlatformError::last_os_error());
        }
        let group = ProcessGroup { job };
        // SAFETY: the struct is plain data; all-zero is a valid value.
        let mut info: JOBOBJECT_EXTENDED_LIMIT_INFORMATION = unsafe { std::mem::zeroed() };
        info.BasicLimitInformation.LimitFlags = JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE;
        let size = std::mem::size_of::<JOBOBJECT_EXTENDED_LIMIT_INFORMATION>() as u32;
        // SAFETY: `info` lives for the call and `size` is its exact size,
        // matching the JobObjectExtendedLimitInformation class.
        let set = unsafe {
            SetInformationJobObject(
                group.job,
                JobObjectExtendedLimitInformation,
                (&info as *const JOBOBJECT_EXTENDED_LIMIT_INFORMATION).cast(),
                size,
            )
        };
        if set == 0 {
            return Err(PlatformError::last_os_error());
        }
        Ok(group)
    }
}

impl Drop for ProcessGroup {
    fn drop(&mut self) {
        // SAFETY: `self.job` is a valid handle owned by `self`, closed only here.
        unsafe {
            CloseHandle(self.job);
        }
    }
}

/// Kills and reaps a suspended child that never joined the job.
fn abandon(child: &mut Child) {
    let _ = child.kill();
    let _ = child.wait();
}

fn io_error(err: std::io::Error) -> PlatformError {
    PlatformError::Os {
        code: err.raw_os_error().unwrap_or(0),
        message: err.to_string(),
    }
}

/// Owned handle closed on drop.
struct Owned(HANDLE);

impl Drop for Owned {
    fn drop(&mut self) {
        // SAFETY: the handle was returned valid by the OS and is owned here.
        unsafe {
            CloseHandle(self.0);
        }
    }
}

/// Resumes every thread of process `pid` (a CREATE_SUSPENDED child has one).
fn resume_threads(pid: u32) -> Result<(), PlatformError> {
    // SAFETY: plain call; the result is checked before use.
    let snapshot = unsafe { CreateToolhelp32Snapshot(TH32CS_SNAPTHREAD, 0) };
    if snapshot == INVALID_HANDLE_VALUE || snapshot.is_null() {
        return Err(PlatformError::last_os_error());
    }
    let snapshot = Owned(snapshot);
    // SAFETY: the struct is plain data; all-zero is valid before dwSize is set.
    let mut entry: THREADENTRY32 = unsafe { std::mem::zeroed() };
    entry.dwSize = std::mem::size_of::<THREADENTRY32>() as u32;
    let mut resumed = 0usize;
    // SAFETY: `snapshot.0` is a valid snapshot handle and `entry` has dwSize set.
    let mut more = unsafe { Thread32First(snapshot.0, &mut entry) } != 0;
    while more {
        if entry.th32OwnerProcessID == pid {
            resume_thread(entry.th32ThreadID)?;
            resumed += 1;
        }
        // SAFETY: as for Thread32First.
        more = unsafe { Thread32Next(snapshot.0, &mut entry) } != 0;
    }
    if resumed == 0 {
        return Err(PlatformError::Os {
            code: 0,
            message: format!("no thread found to resume in process {pid}"),
        });
    }
    Ok(())
}

fn resume_thread(tid: u32) -> Result<(), PlatformError> {
    // SAFETY: plain call; the result is checked before use.
    let thread = unsafe { OpenThread(THREAD_SUSPEND_RESUME, 0, tid) };
    if thread.is_null() {
        return Err(PlatformError::last_os_error());
    }
    let thread = Owned(thread);
    // SAFETY: `thread.0` is a valid handle opened with THREAD_SUSPEND_RESUME.
    let previous = unsafe { ResumeThread(thread.0) };
    if previous == u32::MAX {
        return Err(PlatformError::last_os_error());
    }
    Ok(())
}
