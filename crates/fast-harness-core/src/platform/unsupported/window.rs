use crate::platform::PlatformError;

/// No window icon to set outside Windows: a no-op.
pub fn set_big_window_icon() -> Result<usize, PlatformError> {
    Ok(0)
}

/// No caret timeout setting outside Windows.
pub fn caret_timeout() -> Option<std::time::Duration> {
    None
}

/// No native window handle outside Windows.
pub fn thread_window() -> Option<isize> {
    None
}
