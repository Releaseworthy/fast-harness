//! Windows backend (windows-sys). Each feature lives in its own file.

mod interrupt;
mod open;
mod process;
mod random;
mod secret;

pub use interrupt::on_interrupt;
pub use open::open_url;
pub use process::ProcessGroup;
pub use random::random_fill;
pub use secret::{protect, unprotect};
