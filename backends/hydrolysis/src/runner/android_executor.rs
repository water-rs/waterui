//! The UI-thread executor the Android host shares with every session.
//!
//! The queue is the same channel shape the headless runner drains, but a
//! waker's send also writes a coalescing `eventfd`. The load hook registers
//! that fd with the main `ALooper` once per process — work scheduled from
//! any thread lands on the UI thread while it idles, without a JNI call per
//! wake. The executor outlives every session: dropping a session never
//! unregisters the fd or strands a queued task the way a per-session
//! executor did (water-rs/waterui#2226).

use std::os::fd::{AsRawFd, OwnedFd};
use std::rc::Rc;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, mpsc};

use executor_core::LocalExecutor;
use executor_core::async_task::{AsyncTask, Runnable};

/// The UI-local executor adapted to the main `ALooper`: the same channel
/// queue the headless runner drains, but a waker's send also writes a
/// coalescing `eventfd` the looper's fd callback watches — work scheduled
/// from any thread lands on the UI thread without a JNI call per wake.
#[derive(Clone, Debug)]
pub struct AndroidMainThreadExecutor {
    runnable_tx: mpsc::Sender<Runnable>,
    runnable_rx: Rc<mpsc::Receiver<Runnable>>,
    pending: Arc<AtomicUsize>,
    /// The `eventfd` every schedule edge writes and the only wake fd the
    /// executor has: `ExecutorWake` registers this same fd with the main
    /// `ALooper`, so a wake written while the looper idles is observed on
    /// the fd the looper watches.
    wake_fd: Arc<OwnedFd>,
}

impl AndroidMainThreadExecutor {
    /// `wake_fd` is the fd the looper registration watches and the fd every
    /// waker writes — the registration can never drift onto a different fd.
    pub(crate) fn new(wake_fd: OwnedFd) -> Self {
        let (runnable_tx, runnable_rx) = mpsc::channel();
        Self {
            runnable_tx,
            runnable_rx: Rc::new(runnable_rx),
            pending: Arc::new(AtomicUsize::new(0)),
            wake_fd: Arc::new(wake_fd),
        }
    }

    /// The fd the load hook hands to `ALooper_addFd` — the same one the
    /// waker writes.
    pub(crate) const fn wake_fd(&self) -> &Arc<OwnedFd> {
        &self.wake_fd
    }

    /// Runs every runnable currently queued, returning whether any ran.
    pub(crate) fn drain(&self) -> bool {
        let mut ran = false;
        loop {
            let Ok(runnable) = self.runnable_rx.try_recv() else {
                return ran;
            };
            ran = true;
            runnable.run();
            self.pending.fetch_sub(1, Ordering::SeqCst);
        }
    }
}

impl LocalExecutor for AndroidMainThreadExecutor {
    type Task<T: 'static> = AsyncTask<T>;

    fn spawn_local<Fut>(&self, fut: Fut) -> Self::Task<Fut::Output>
    where
        Fut: std::future::Future + 'static,
    {
        let runnable_tx = self.runnable_tx.clone();
        let pending = Arc::clone(&self.pending);
        let wake_fd = self.wake_fd.as_raw_fd();
        let (runnable, task) = executor_core::async_task::spawn_local(fut, move |runnable| {
            pending.fetch_add(1, Ordering::SeqCst);
            match runnable_tx.send(runnable) {
                Ok(()) => {
                    // Coalesced wake: one counter increment regardless of how
                    // many runnables are already queued.
                    // SAFETY: `wake_fd` is the executor's live eventfd.
                    unsafe {
                        libc::eventfd_write(wake_fd, 1);
                    }
                }
                Err(unsent) => {
                    pending.fetch_sub(1, Ordering::SeqCst);
                    // Same teardown race as the headless executor: dropping a
                    // spawn_local runnable off-thread panics — leak it.
                    std::mem::forget(unsent);
                }
            }
        });
        runnable.schedule();
        task
    }
}

#[cfg(all(test, not(target_arch = "wasm32")))]
mod tests {
    use std::cell::Cell;
    use std::os::fd::{AsRawFd, FromRawFd};

    use super::*;

    fn executor() -> AndroidMainThreadExecutor {
        // SAFETY: NONBLOCK + CLOEXEC keep a saturated counter from stalling
        // the looper and the fd out of child processes.
        let fd = unsafe { libc::eventfd(0, libc::EFD_NONBLOCK | libc::EFD_CLOEXEC) };
        assert!(
            fd >= 0,
            "eventfd failed: {}",
            std::io::Error::last_os_error()
        );
        // SAFETY: fd >= 0 checked above.
        AndroidMainThreadExecutor::new(unsafe { OwnedFd::from_raw_fd(fd) })
    }

    /// A wake written while the looper is idle must land on the fd the
    /// `ALooper` watches. `ExecutorWake` cannot run on a host machine — it
    /// needs the Android `ALooper` — but it registers `wake_fd()`, the only
    /// fd the executor has, so polling it here exercises exactly what the
    /// looper sees on device. The regression this guards against wrote
    /// wakes to one eventfd while a different, never-written fd sat in the
    /// looper.
    #[test]
    fn a_wake_written_while_idle_is_observed_on_the_registered_fd() {
        let executor = executor();
        let ran = Rc::new(Cell::new(false));
        let flag = Rc::clone(&ran);
        // `AsyncTask` cancels on drop — hold it as callers must.
        let _task = executor.spawn_local(async move {
            flag.set(true);
        });

        // What `ALooper_pollOnce` would report while idle: the registered
        // fd readable, the wake already there.
        let mut pollfd = libc::pollfd {
            fd: executor.wake_fd().as_raw_fd(),
            events: libc::POLLIN,
            revents: 0,
        };
        // SAFETY: `pollfd` points at a live pollfd entry for the call.
        assert_eq!(
            unsafe { libc::poll(&raw mut pollfd, 1, 0) },
            1,
            "the wake the executor wrote is not observed on the fd the looper watches"
        );
        let mut count: u64 = 0;
        // SAFETY: `wake_fd` is the live eventfd the spawn wrote; `count`
        // receives the coalesced counter exactly as the looper callback
        // reads it.
        unsafe {
            libc::eventfd_read(executor.wake_fd().as_raw_fd(), &raw mut count);
        }
        assert_eq!(count, 1);

        // The fd callback then drains the queue on the UI thread.
        assert!(executor.drain());
        assert!(ran.get());
        assert!(!executor.drain(), "a drained queue must go quiet again");
    }
}
