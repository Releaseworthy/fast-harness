//! Opening a URL in the default browser and a folder in Explorer
//! (ShellExecuteW).

use std::path::Path;

use crate::platform::PlatformError;
use windows_sys::Win32::UI::Shell::ShellExecuteW;
use windows_sys::Win32::UI::WindowsAndMessaging::SW_SHOWNORMAL;

/// `ERROR_INVALID_PARAMETER`, for a URL this function refuses to open.
const INVALID_PARAMETER: i32 = 87;

/// Opens an `https://` or `http://` URL in the user's default browser. Any
/// other scheme (a file path, `javascript:`, a custom protocol) is refused, so
/// the shell never runs a program for it.
pub fn open_url(url: &str) -> Result<(), PlatformError> {
    if !is_web_url(url) {
        return Err(PlatformError::Os {
            code: INVALID_PARAMETER,
            message: "only http:// and https:// URLs are opened".into(),
        });
    }
    shell_execute("open", url)
}

/// Opens an existing folder in Explorer. Anything that is not a directory
/// is refused, so the shell never runs a program for it.
pub fn open_folder(path: &Path) -> Result<(), PlatformError> {
    if !path.is_dir() {
        return Err(PlatformError::Os {
            code: INVALID_PARAMETER,
            message: format!("{} is not a folder", path.display()),
        });
    }
    shell_execute("explore", &path.to_string_lossy())
}

fn shell_execute(verb: &str, target: &str) -> Result<(), PlatformError> {
    let verb = wide(verb);
    let file = wide(target);
    // SAFETY: `verb` and `file` are NUL-terminated UTF-16 buffers that live
    // until the call returns; a null window, null parameters and a null
    // directory are documented as allowed for ShellExecuteW.
    let result = unsafe {
        ShellExecuteW(
            std::ptr::null_mut(),
            verb.as_ptr(),
            file.as_ptr(),
            std::ptr::null(),
            std::ptr::null(),
            SW_SHOWNORMAL,
        )
    };
    // ShellExecuteW returns a value above 32 on success, an error code otherwise.
    let code = result as isize;
    if code > 32 {
        Ok(())
    } else {
        Err(PlatformError::Os {
            code: i32::try_from(code).unwrap_or(-1),
            message: format!("ShellExecuteW failed ({code})"),
        })
    }
}

/// `http://` or `https://` (any case), a host part, and no white space or control characters.
fn is_web_url(url: &str) -> bool {
    let lower = url.to_ascii_lowercase();
    let rest = lower
        .strip_prefix("https://")
        .or_else(|| lower.strip_prefix("http://"));
    rest.is_some_and(|rest| !rest.is_empty())
        && !url.chars().any(|c| c.is_control() || c.is_whitespace())
}

fn wide(text: &str) -> Vec<u16> {
    text.encode_utf16().chain(std::iter::once(0)).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_web_urls_pass() {
        assert!(is_web_url("https://claude.ai/oauth/authorize?a=b"));
        assert!(is_web_url("HTTP://localhost:1/x"));
        assert!(!is_web_url("https://"));
        assert!(!is_web_url("file:///C:/Windows/notepad.exe"));
        assert!(!is_web_url("C:\\Windows\\notepad.exe"));
        assert!(!is_web_url("https://a b"));
        assert!(!is_web_url("https://a\0b"));
    }

    #[test]
    fn other_schemes_are_refused_before_the_shell() {
        let err = open_url("javascript:alert(1)").unwrap_err();
        assert!(matches!(err, PlatformError::Os { code: 87, .. }));
    }

    #[test]
    fn a_missing_folder_is_refused_before_the_shell() {
        let missing = std::env::temp_dir().join("fast-harness-no-such-folder-7f3a");
        let err = open_folder(&missing).unwrap_err();
        assert!(matches!(err, PlatformError::Os { code: 87, .. }));
    }
}
