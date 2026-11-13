//! Fallback for an OS without a backend yet: every item reports `Unsupported`,
//! except `random_fill`, which reads `/dev/urandom` on Unix. `protect` and `unprotect` stay
//! `Unsupported` on purpose: secrets are never stored in plaintext (PLAN.md D9).

mod file;
mod folder;
mod interrupt;
mod liveness;
mod open;
mod process;
mod random;
mod secret;
mod window;

pub use file::copy_permissions;
pub use folder::{pick_file, pick_folder};
pub use interrupt::on_interrupt;
pub use liveness::{process_start, spawn_detached};
pub use open::{open_folder, open_url};
pub use process::{ended_by_kill, ProcessGroup};
pub use random::random_fill;
pub use secret::{protect, unprotect};
pub use window::{caret_timeout, set_big_window_icon, thread_window};
