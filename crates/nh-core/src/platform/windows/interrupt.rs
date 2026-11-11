use std::sync::Mutex;

use windows_sys::core::BOOL;
use windows_sys::Win32::System::Console::{SetConsoleCtrlHandler, CTRL_BREAK_EVENT, CTRL_C_EVENT};

use crate::platform::PlatformError;

static HANDLER: Mutex<Option<fn()>> = Mutex::new(None);

/// Runs on a thread the OS creates for each console control event.
unsafe extern "system" fn on_ctrl(kind: u32) -> BOOL {
    if kind != CTRL_C_EVENT && kind != CTRL_BREAK_EVENT {
        return 0;
    }
    let handler = HANDLER.lock().ok().and_then(|slot| *slot);
    match handler {
        Some(handler) => {
            handler();
            1
        }
        None => 0,
    }
}

/// Calls `handler` on Ctrl+C or Ctrl+Break instead of ending the process.
/// A later call replaces the handler.
pub fn on_interrupt(handler: fn()) -> Result<(), PlatformError> {
    if let Ok(mut slot) = HANDLER.lock() {
        *slot = Some(handler);
    }
    // SAFETY: `on_ctrl` is a plain function with the PHANDLER_ROUTINE
    // signature that lives for the whole process; registering it twice only
    // means it runs twice per event, and it reads the same slot each time.
    let ok = unsafe { SetConsoleCtrlHandler(Some(on_ctrl), 1) };
    if ok == 0 {
        return Err(PlatformError::last_os_error());
    }
    Ok(())
}
