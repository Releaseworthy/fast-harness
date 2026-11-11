//! The large window icon (taskbar, Alt-Tab) from the exe's icon group 1.

use windows_sys::Win32::Foundation::{HWND, LPARAM};
use windows_sys::Win32::System::Threading::GetCurrentThreadId;
use windows_sys::Win32::UI::WindowsAndMessaging::{
    EnumThreadWindows, GetSystemMetrics, GetWindowLongPtrW, IsWindowVisible, LoadImageW,
    SendMessageW, GWLP_HINSTANCE, ICON_BIG, IMAGE_ICON, LR_DEFAULTCOLOR, SM_CXICON, SM_CYICON,
    WM_SETICON,
};

use crate::platform::PlatformError;

/// The resource id of the icon group `nh-gui/build.rs` embeds.
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
