use crate::platform::PlatformError;

/// No window icon to set outside Windows: a no-op.
pub fn set_big_window_icon() -> Result<usize, PlatformError> {
    Ok(0)
}
