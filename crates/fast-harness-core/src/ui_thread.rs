//! The UI thread mark (D60, chunk 26B). A front end marks the thread that
//! draws its window or terminal; the reads that grow with history
//! (`session::list`, `session::latest`, `trace::read`, `trace::reader::load`)
//! call [`forbid`], which panics on a marked thread in a debug build, so a
//! change that moves such a read back onto a UI thread fails its tests.
//! Release builds keep the mark (one thread-local bool) and skip the check.

use std::cell::Cell;

thread_local! {
    static MARKED: Cell<bool> = const { Cell::new(false) };
}

/// Marks (`true`) or unmarks the calling thread as a front end's UI thread.
pub fn mark(on: bool) {
    MARKED.with(|m| m.set(on));
}

/// Whether the calling thread is marked.
pub fn is_marked() -> bool {
    MARKED.with(Cell::get)
}

/// A read named `what` is about to run: on a marked thread a debug build
/// panics (an invariant of the front ends, never input from outside).
#[track_caller]
pub fn forbid(what: &str) {
    if cfg!(debug_assertions) && is_marked() {
        panic!("{what} ran on a UI thread (D60): run it on a worker");
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_marked_thread_panics_in_debug_and_others_do_not() {
        forbid("unmarked");
        let marked = std::thread::spawn(|| {
            mark(true);
            forbid("session::list");
        })
        .join();
        assert_eq!(marked.is_err(), cfg!(debug_assertions));
        assert!(!is_marked());
    }
}
