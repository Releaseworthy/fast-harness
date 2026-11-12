//! `/reflect [apply]` in the TUI (PLAN.md D50): the job is prepared here and
//! runs on its own thread; the text comes back as `Msg::Reflected`.

use std::thread;

use fast_harness_core::view::NoteKind;

use super::app::{Msg, Tui};

impl Tui {
    pub(super) fn reflect(&mut self, args: &str) {
        let made = self.rt().reflect_job(&self.cwd, Some(&self.session), args);
        let job = match made {
            Ok(job) => job,
            Err(err) => return self.note(NoteKind::Error, err),
        };
        let tx = self.tx.clone();
        let spawned = thread::Builder::new()
            .name("fast-harness-reflect".into())
            .spawn(move || {
                let _ = tx.send(Msg::Reflected(job()));
            });
        match spawned {
            Ok(_) => self.note(NoteKind::Info, "reflect is running"),
            Err(err) => self.note(NoteKind::Error, format!("reflect: {err}")),
        }
    }
}
