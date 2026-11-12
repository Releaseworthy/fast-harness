//! The parent's side of background children (PLAN.md D40): their events go
//! to the parent's sink, their answers join the parent's inbox as steers, a
//! turn that ended `Done` waits for the ones still running, and a cancel
//! stops them all.

use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant};

use super::children::TICK;
use super::{Agent, Inbox};
use crate::events::AgentEvent;

/// The longest wait for cancelled children to end.
const STOP_WAIT: Duration = Duration::from_secs(30);

impl Agent {
    /// Sends the events of the children (and their descendants) to `sink`.
    pub(super) fn forward_events(&self, sink: &mut dyn FnMut(AgentEvent)) {
        if let Some(children) = &self.children {
            for event in children.take_events() {
                sink(event);
            }
        }
    }

    /// The finished notices of background children, if any.
    pub(super) fn take_notices(&self) -> Option<String> {
        self.children.as_ref()?.take_notices()
    }

    /// The text of the next turn after one that ended `Done`, and whether
    /// it is a steer: the inbox first, else a background child's answer,
    /// waiting while children run. `None` on pause, cancel or nothing left.
    pub(super) fn next_input(
        &self,
        cancel: &AtomicBool,
        inbox: &mut Inbox,
        sink: &mut dyn FnMut(AgentEvent),
    ) -> Option<(String, bool)> {
        loop {
            if inbox.take_pause() || cancel.load(Ordering::Relaxed) {
                return None;
            }
            if let Some(text) = inbox.next_turn() {
                return Some((text, false));
            }
            self.forward_events(sink);
            if let Some(text) = self.take_notices() {
                return Some((text, true));
            }
            let children = self.children.as_ref()?;
            if children.running_background() == 0 {
                return None;
            }
            children.wait_change(TICK);
        }
    }

    /// Cancels every background child (each passes it on to its own
    /// children) and waits for them to end, forwarding their last events.
    pub(super) fn stop_children(&self, sink: &mut dyn FnMut(AgentEvent)) {
        let Some(children) = &self.children else {
            return;
        };
        children.cancel_all();
        children.wait_idle(Instant::now() + STOP_WAIT);
        self.forward_events(sink);
    }
}
