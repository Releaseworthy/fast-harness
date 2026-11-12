//! A panic leaves a trace. The window has no console (`windows_subsystem`),
//! and a panic on a turn thread is caught and shown as one fixed sentence
//! (`turn.rs`), so without this nothing says what went wrong: the panic hook
//! appends the message and its place to `<home>/crash.log`, a file a user can
//! read before attaching it to a report (a panic message can quote the data
//! that caused it).

use std::fs::OpenOptions;
use std::io::Write;
use std::path::{Path, PathBuf};

/// The file name under the app home.
pub const FILE_NAME: &str = "crash.log";

/// `<home>/crash.log`, when there is a home.
pub fn path() -> Option<PathBuf> {
    fast_harness_core::platform::app_home().map(|home| home.join(FILE_NAME))
}

/// Chains a hook that logs every panic of every thread, then runs the
/// previous hook.
pub fn install() {
    let previous = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |info| {
        if let Some(path) = path() {
            let thread = std::thread::current();
            append(&path, thread.name().unwrap_or("unnamed"), &info.to_string());
        }
        previous(info);
    }));
}

/// One entry: when (Unix milliseconds), the version, the thread, the panic.
/// A write that fails is dropped: a panic hook must not panic.
fn append(path: &Path, thread: &str, what: &str) {
    let now = fast_harness_core::trace::now_ms();
    let version = env!("CARGO_PKG_VERSION");
    let entry = format!("{now} fast-harness-gui {version} thread {thread}: {what}\n");
    if let Some(dir) = path.parent() {
        let _ = std::fs::create_dir_all(dir);
    }
    if let Ok(mut file) = OpenOptions::new().create(true).append(true).open(path) {
        let _ = file.write_all(entry.as_bytes());
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn entries_are_appended_with_version_and_thread() {
        let dir = std::env::temp_dir().join(format!("fh-crash-{}", std::process::id()));
        let path = dir.join("deep").join(FILE_NAME);
        append(&path, "fast-harness-turn", "panicked at src/x.rs:1:1:\nboom");
        append(&path, "main", "second");
        let text = std::fs::read_to_string(&path).unwrap();
        let version = env!("CARGO_PKG_VERSION");
        assert!(text.contains(&format!(
            "fast-harness-gui {version} thread fast-harness-turn: panicked"
        )));
        assert!(text.contains("boom\n") && text.trim_end().ends_with("thread main: second"));
        std::fs::remove_dir_all(&dir).unwrap();
    }
}
