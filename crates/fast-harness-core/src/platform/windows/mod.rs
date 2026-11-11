//! Windows backend (windows-sys). Each feature lives in its own file.

mod folder;
mod interrupt;
mod open;
mod process;
mod random;
mod secret;
mod window;

pub use folder::pick_folder;
pub use interrupt::on_interrupt;
pub use open::{open_folder, open_url};
pub use process::ProcessGroup;
pub use random::random_fill;
pub use secret::{protect, unprotect};
pub use window::{set_big_window_icon, thread_window};
