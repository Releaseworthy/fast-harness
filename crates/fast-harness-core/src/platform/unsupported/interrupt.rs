use crate::platform::PlatformError;

pub fn on_interrupt(_handler: fn()) -> Result<(), PlatformError> {
    Err(PlatformError::Unsupported)
}
