//! The connections one call uses, so a cancel can shut them down (PLAN.md
//! D66). ureq 3.5 lets a connector register a [`ShutdownHandle`] for each
//! socket it opens (`ConnectionDetails::register_shutdown`); `shutdown` on
//! it makes a read or write blocked on that socket in another thread return
//! at once. `Agent::shutdown` would shut down every call of the agent, so
//! [`WatchedConnector`] keeps the handles per connection instead, and
//! [`Watched`] hands them to the call that is using the connection (pooled
//! connections move between calls) through a thread local: each cancellable
//! call runs on its own thread (`net::call`).

use std::cell::RefCell;
use std::fmt;
use std::io;
use std::mem;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, Weak};

use ureq::unversioned::transport::{
    Buffers, ConnectionDetails, Connector, DefaultConnector, NextTimeout, ShutdownHandle, Transport,
};
use ureq::Error;

type Handles = Vec<Weak<dyn ShutdownHandle>>;

/// One cancellable call: the sockets it uses and whether it was stopped.
#[derive(Default)]
pub(super) struct CallCtx {
    stopped: AtomicBool,
    handles: Mutex<Handles>,
}

impl CallCtx {
    /// Ends the call: every socket it uses is shut down now, and one it
    /// registers later is shut down at once ([`CallCtx::watch`]).
    pub(super) fn stop(&self) {
        self.stopped.store(true, Ordering::SeqCst);
        let handles = match self.handles.lock() {
            Ok(mut list) => mem::take(&mut *list),
            Err(poisoned) => mem::take(&mut *poisoned.into_inner()),
        };
        for handle in handles.iter().filter_map(Weak::upgrade) {
            handle.shutdown();
        }
    }

    /// Adds `handles` to the call; fails (and shuts them down) once the
    /// call is stopped. The push happens before the check and `stop` sets
    /// the flag before it takes the list, so no handle escapes a stop.
    fn watch(&self, handles: &[Weak<dyn ShutdownHandle>]) -> Result<(), Error> {
        if let Ok(mut list) = self.handles.lock() {
            for handle in handles {
                if !list.iter().any(|known| Weak::ptr_eq(known, handle)) {
                    list.push(handle.clone());
                }
            }
        }
        if self.stopped.load(Ordering::SeqCst) {
            self.stop();
            return Err(cancelled());
        }
        Ok(())
    }
}

fn cancelled() -> Error {
    Error::Io(io::Error::other("cancelled"))
}

thread_local! {
    static CURRENT: RefCell<Option<Arc<CallCtx>>> = const { RefCell::new(None) };
}

/// Marks this thread as running the call `ctx` (the call's own thread).
pub(super) fn enter(ctx: Arc<CallCtx>) {
    CURRENT.with(|current| *current.borrow_mut() = Some(ctx));
}

/// Runs `f` on the call of this thread; a thread without one (a call
/// without a cancel flag) is never watched.
fn with_current(f: impl FnOnce(&CallCtx) -> Result<(), Error>) -> Result<(), Error> {
    let ctx = CURRENT.with(|current| current.borrow().clone());
    match ctx {
        Some(ctx) => f(&ctx),
        None => Ok(()),
    }
}

/// ureq's default connector chain whose transports remember the shutdown
/// handles registered while they were made.
#[derive(Debug, Default)]
pub(super) struct WatchedConnector {
    inner: DefaultConnector,
}

impl Connector<()> for WatchedConnector {
    type Out = Watched;

    fn connect(
        &self,
        details: &ConnectionDetails,
        chained: Option<()>,
    ) -> Result<Option<Watched>, Error> {
        with_current(|ctx| ctx.watch(&[]))?;
        let seen: Arc<Mutex<Handles>> = Arc::default();
        let outer = details.register_shutdown.clone();
        let kept = seen.clone();
        let register = Arc::new(move |handle: Weak<dyn ShutdownHandle>| {
            if let Ok(mut list) = kept.lock() {
                list.push(handle.clone());
            }
            with_current(|ctx| ctx.watch(std::slice::from_ref(&handle)))?;
            outer(handle)
        });
        let details = ConnectionDetails {
            uri: details.uri,
            addrs: details.addrs.clone(),
            config: details.config,
            request_level: details.request_level,
            resolver: details.resolver,
            now: details.now,
            timeout: details.timeout,
            current_time: details.current_time.clone(),
            run_connector: details.run_connector.clone(),
            register_shutdown: register,
        };
        let made = self.inner.connect(&details, chained)?;
        let handles = seen.lock().map(|list| list.clone()).unwrap_or_default();
        Ok(made.map(|inner| Watched { inner, handles }))
    }
}

/// A connection that joins the current call before each wait on it.
pub(super) struct Watched {
    inner: Box<dyn Transport>,
    handles: Handles,
}

impl fmt::Debug for Watched {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Watched")
            .field("inner", &self.inner)
            .field("handles", &self.handles.len())
            .finish()
    }
}

impl Transport for Watched {
    fn buffers(&mut self) -> &mut dyn Buffers {
        self.inner.buffers()
    }

    fn transmit_output(&mut self, amount: usize, timeout: NextTimeout) -> Result<(), Error> {
        with_current(|ctx| ctx.watch(&self.handles))?;
        self.inner.transmit_output(amount, timeout)
    }

    fn await_input(&mut self, timeout: NextTimeout) -> Result<bool, Error> {
        with_current(|ctx| ctx.watch(&self.handles))?;
        self.inner.await_input(timeout)
    }

    fn is_open(&mut self) -> bool {
        self.inner.is_open()
    }

    fn is_tls(&self) -> bool {
        self.inner.is_tls()
    }
}
