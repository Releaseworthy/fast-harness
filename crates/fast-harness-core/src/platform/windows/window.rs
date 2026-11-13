//! The large window icon (taskbar, Alt-Tab) from the exe's icon group 1,
//! the main window handle and the caret timeout.

use std::time::Duration;

use windows_sys::Win32::Foundation::{HWND, LPARAM};
use windows_sys::Win32::System::Threading::GetCurrentThreadId;
use windows_sys::Win32::UI::WindowsAndMessaging::{
    EnumThreadWindows, GetSystemMetrics, GetWindow, GetWindowLongPtrW, IsWindowVisible, LoadImageW,
    SendMessageW, SystemParametersInfoW, GWLP_HINSTANCE, GW_OWNER, ICON_BIG, IMAGE_ICON,
    LR_DEFAULTCOLOR, SM_CXICON, SM_CYICON, SPI_GETCARETTIMEOUT, WM_SETICON,
};

use crate::platform::PlatformError;

/// How long a text caret blinks after the last input (`SPI_GETCARETTIMEOUT`,
/// 5000 ms by default); `None` when the call fails or gives 0.
pub fn caret_timeout() -> Option<Duration> {
    let mut ms: u32 = 0;
    // SAFETY: SPI_GETCARETTIMEOUT writes one DWORD through `pvparam`, which
    // points at `ms` for the whole call; no profile is written (flags 0).
    let ok =
        unsafe { SystemParametersInfoW(SPI_GETCARETTIMEOUT, 0, (&mut ms as *mut u32).cast(), 0) };
    (ok != 0 && ms != 0).then(|| Duration::from_millis(u64::from(ms)))
}

/// The resource id of the icon group `fast-harness-gui/build.rs` embeds.
const ICON_GROUP: usize = 1;

/// Gives every visible top-level window of the calling thread the big icon
/// (SM_CXICON size) from icon group 1 of the module that owns the window.
/// Call it on the UI thread after the window is shown. Returns how many
/// windows got the icon.
pub fn set_big_window_icon() -> Result<usize, PlatformError> {
    let mut count: usize = 0;
    // SAFETY: the callback only runs during this call, on this thread, and
    // `count` outlives it; the pointer passed as LPARAM is the callback's only
    // way to reach it.
    unsafe {
        EnumThreadWindows(
            GetCurrentThreadId(),
            Some(set_icon_on),
            &mut count as *mut usize as LPARAM,
        );
    }
    if count == 0 {
        return Err(PlatformError::last_os_error());
    }
    Ok(count)
}

/// `EnumThreadWindows` callback: one window. Always continues the walk.
unsafe extern "system" fn set_icon_on(hwnd: HWND, lparam: LPARAM) -> i32 {
    // SAFETY: `hwnd` comes from EnumThreadWindows, so it is a live window of
    // this thread; `lparam` is the `&mut usize` set_big_window_icon passed and
    // keeps alive for the whole enumeration.
    unsafe {
        if IsWindowVisible(hwnd) == 0 {
            return 1;
        }
        let instance = GetWindowLongPtrW(hwnd, GWLP_HINSTANCE) as *mut core::ffi::c_void;
        let (cx, cy) = (GetSystemMetrics(SM_CXICON), GetSystemMetrics(SM_CYICON));
        // MAKEINTRESOURCE: a small integer id in the name pointer.
        let name = ICON_GROUP as *const u16;
        let icon = LoadImageW(instance, name, IMAGE_ICON, cx, cy, LR_DEFAULTCOLOR);
        if !icon.is_null() {
            SendMessageW(hwnd, WM_SETICON, ICON_BIG as usize, icon as LPARAM);
            *(lparam as *mut usize) += 1;
        }
    }
    1
}

/// The calling thread's first visible top-level window without an owner (the main
/// window on the UI thread), as a raw HWND value.
pub fn thread_window() -> Option<isize> {
    let mut found: isize = 0;
    // SAFETY: the callback only runs during this call, on this thread, and `found`
    // outlives it.
    unsafe {
        EnumThreadWindows(
            GetCurrentThreadId(),
            Some(first_visible),
            &mut found as *mut isize as LPARAM,
        );
    }
    (found != 0).then_some(found)
}

/// `EnumThreadWindows` callback: stops at the first visible unowned window.
unsafe extern "system" fn first_visible(hwnd: HWND, lparam: LPARAM) -> i32 {
    // SAFETY: `hwnd` is a live window of this thread from EnumThreadWindows; `lparam`
    // is the `&mut isize` thread_window keeps alive for the walk.
    unsafe {
        if IsWindowVisible(hwnd) == 0 || !GetWindow(hwnd, GW_OWNER).is_null() {
            return 1;
        }
        *(lparam as *mut isize) = hwnd as isize;
    }
    0
}
