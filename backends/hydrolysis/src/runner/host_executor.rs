//! The main-thread executor every windowed host shares across its sessions.
//!
//! The queue is the same channel shape the headless runner drains, but a
//! waker's send also fires the host's [`HostWake`] — a coalescing signal that
//! lands on the UI thread while it idles, without a bridge call per wake.
//! Android's wake is an `eventfd` the load hook registers with the main
//! `ALooper` once per process; the UIKit host's is a coalesced main-queue
//! block (`cocoa_ui::main_queue::MainQueueWake`). The executor outlives every session: dropping
//! a session never unregisters the wake or strands a queued task the way a
//! per-session executor did (water-rs/waterui#2226).
//!
//! The same wake also carries one-shot posts: a thread-safe caller that needs
//! a bare call on the UI thread — the GPU-surface redraw handle asking the
//! host for its next frame — queues it through [`UiThreadPoster`], so its
//! wake coalesces with the executor's on the host's one wake instead of
//! paying a second registration (water-rs/waterui#2287).

use std::fmt::Debug;
#[cfg(any(target_os = "android", all(test, target_os = "linux")))]
use std::os::fd::{AsRawFd, BorrowedFd, OwnedFd};
use std::rc::Rc;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::{Arc, mpsc};

use executor_core::LocalExecutor;
use executor_core::async_task::{AsyncTask, Runnable};

/// A one-shot call a thread-safe wake queues for the UI thread — the
/// GPU-surface redraw handle's frame request. It rides the executor's wake
/// rather than a `Runnable`: a `Runnable` carries an async task, a wake
/// post is a bare call.
type UiPost = Box<dyn FnOnce() + Send>;

/// The host's way to get its UI thread to drain the executor.
///
/// [`wake`](Self::wake) is called from any thread after the work is queued,
/// and must coalesce: however many wakes arrive before the UI thread drains,
/// one drain observes all the queued work.
pub trait HostWake: Debug + Send + Sync + 'static {
    /// Signals the UI thread that the executor has work queued.
    fn wake(&self);
}

/// The `UIKit` host's wake: a coalesced block on the main dispatch queue, run
/// by the main run loop.
#[cfg(hydrolysis_uikit)]
impl HostWake for cocoa_ui::main_queue::MainQueueWake {
    fn wake(&self) {
        Self::wake(self);
    }
}

/// Android's wake: a coalescing `eventfd` the main `ALooper` watches. The
/// load hook registers this same fd, so a wake written while the looper idles
/// is observed on the fd the looper watches.
#[cfg(any(target_os = "android", all(test, target_os = "linux")))]
#[derive(Debug)]
pub struct EventFdWake(OwnedFd);

#[cfg(any(target_os = "android", all(test, target_os = "linux")))]
impl EventFdWake {
    /// Wraps the `eventfd` every wake writes and the looper registration
    /// watches — the registration can never drift onto a different fd.
    pub(crate) const fn new(fd: OwnedFd) -> Self {
        Self(fd)
    }

    /// The fd the load hook hands to `ALooper_addFd`.
    pub(crate) fn fd(&self) -> BorrowedFd<'_> {
        use std::os::fd::AsFd;
        self.0.as_fd()
    }
}

#[cfg(any(target_os = "android", all(test, target_os = "linux")))]
impl HostWake for EventFdWake {
    fn wake(&self) {
        // Coalesced wake: one counter increment regardless of how many
        // runnables are already queued.
        // SAFETY: `self.0` is the executor's live eventfd.
        unsafe {
            libc::eventfd_write(self.0.as_raw_fd(), 1);
        }
    }
}

/// The UI-local executor adapted to a host's main thread: the same channel
/// queue the headless runner drains, but a waker's send also fires the
/// host's [`HostWake`] — work scheduled from any thread lands on the UI
/// thread without a bridge call per wake.
#[derive(Debug)]
pub struct HostMainThreadExecutor<W: HostWake> {
    runnable_tx: mpsc::Sender<Runnable>,
    runnable_rx: Rc<mpsc::Receiver<Runnable>>,
    /// One-shot posts from thread-safe wakes, drained beside the runnables
    /// so both ride the one wake.
    post_tx: mpsc::Sender<UiPost>,
    post_rx: Rc<mpsc::Receiver<UiPost>>,
    pending: Arc<AtomicUsize>,
    /// The only wake the executor has: whatever watches the UI thread
    /// registers this same wake, so a wake fired while the thread idles is
    /// observed where the host listens.
    wake: Arc<W>,
}

impl<W: HostWake> Clone for HostMainThreadExecutor<W> {
    fn clone(&self) -> Self {
        Self {
            runnable_tx: self.runnable_tx.clone(),
            runnable_rx: Rc::clone(&self.runnable_rx),
            post_tx: self.post_tx.clone(),
            post_rx: Rc::clone(&self.post_rx),
            pending: Arc::clone(&self.pending),
            wake: Arc::clone(&self.wake),
        }
    }
}

/// The thread-safe face of the executor's wake, handed to callers that run
/// off the UI thread — the window's GPU-surface redraw handle. A
/// [`post`](Self::post) queues `work` for the next drain and fires the
/// shared wake, so the call lands on the UI thread coalesced with every
/// other wake the host already counted.
#[derive(Debug)]
pub struct UiThreadPoster<W: HostWake> {
    tx: mpsc::Sender<UiPost>,
    wake: Arc<W>,
}

impl<W: HostWake> Clone for UiThreadPoster<W> {
    fn clone(&self) -> Self {
        Self {
            tx: self.tx.clone(),
            wake: Arc::clone(&self.wake),
        }
    }
}

impl<W: HostWake> UiThreadPoster<W> {
    /// Queues `work` for the next drain and fires the wake. The send
    /// precedes the wake so a drain that observes it always finds the
    /// work; a failed send means the executor is gone and the wake is a
    /// no-op, the same shape a dead task channel gives a scheduled task.
    pub(crate) fn post(&self, work: impl FnOnce() + Send + 'static) {
        if self.tx.send(Box::new(work)).is_ok() {
            self.wake.wake();
        }
    }
}

/// The cross-thread `request_redraw` of a host window: a producer's wake (a
/// decoder, an external frame, an engine completion) lands on any thread, but
/// the frame scheduler is UI-thread state. The returned wake latches the
/// redraw the next frame renders and queues one post through `poster`; the
/// post runs `wake` on the UI thread, where it asks for the frame through
/// the transaction gate. Wakes raised before the queued post runs coalesce
/// into it.
pub fn redraw_wake<W: HostWake>(
    poster: UiThreadPoster<W>,
    latch: Arc<AtomicBool>,
    wake: impl Fn() + Send + Sync + 'static,
) -> impl Fn() + Send + Sync + 'static {
    let wake = Arc::new(wake);
    let queued = Arc::new(AtomicBool::new(false));
    move || {
        latch.store(true, Ordering::Release);
        if queued.swap(true, Ordering::AcqRel) {
            return;
        }
        let wake = Arc::clone(&wake);
        let queued = Arc::clone(&queued);
        poster.post(move || {
            queued.store(false, Ordering::Release);
            wake();
        });
    }
}

impl<W: HostWake> HostMainThreadExecutor<W> {
    /// `wake` is the signal the host's UI-thread registration observes and
    /// every waker fires.
    pub(crate) fn new(wake: W) -> Self {
        let (runnable_tx, runnable_rx) = mpsc::channel();
        let (post_tx, post_rx) = mpsc::channel();
        Self {
            runnable_tx,
            runnable_rx: Rc::new(runnable_rx),
            post_tx,
            post_rx: Rc::new(post_rx),
            pending: Arc::new(AtomicUsize::new(0)),
            wake: Arc::new(wake),
        }
    }

    /// The wake the host's registration observes — the same one every waker
    /// fires.
    #[cfg(any(android_platform, all(test, target_os = "linux")))]
    pub(crate) const fn wake(&self) -> &Arc<W> {
        &self.wake
    }

    /// The handle a thread-safe caller posts through — the window's
    /// GPU-surface redraw handle ends its wake in one.
    pub(crate) fn poster(&self) -> UiThreadPoster<W> {
        UiThreadPoster {
            tx: self.post_tx.clone(),
            wake: Arc::clone(&self.wake),
        }
    }

    /// Runs every runnable and post currently queued, returning whether any
    /// ran.
    pub(crate) fn drain(&self) -> bool {
        let mut ran = false;
        while let Ok(runnable) = self.runnable_rx.try_recv() {
            ran = true;
            runnable.run();
            self.pending.fetch_sub(1, Ordering::SeqCst);
        }
        while let Ok(post) = self.post_rx.try_recv() {
            ran = true;
            post();
        }
        ran
    }
}

impl<W: HostWake> LocalExecutor for HostMainThreadExecutor<W> {
    type Task<T: 'static> = AsyncTask<T>;

    fn spawn_local<Fut>(&self, fut: Fut) -> Self::Task<Fut::Output>
    where
        Fut: std::future::Future + 'static,
    {
        let runnable_tx = self.runnable_tx.clone();
        let pending = Arc::clone(&self.pending);
        let wake = Arc::clone(&self.wake);
        let (runnable, task) = executor_core::async_task::spawn_local(fut, move |runnable| {
            pending.fetch_add(1, Ordering::SeqCst);
            match runnable_tx.send(runnable) {
                Ok(()) => wake.wake(),
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

#[cfg(all(test, target_os = "linux"))]
mod tests {
    use std::cell::Cell;
    use std::os::fd::FromRawFd;

    use super::*;

    fn executor() -> HostMainThreadExecutor<EventFdWake> {
        // SAFETY: NONBLOCK + CLOEXEC keep a saturated counter from stalling
        // the looper and the fd out of child processes.
        let fd = unsafe { libc::eventfd(0, libc::EFD_NONBLOCK | libc::EFD_CLOEXEC) };
        assert!(
            fd >= 0,
            "eventfd failed: {}",
            std::io::Error::last_os_error()
        );
        // SAFETY: fd >= 0 checked above.
        HostMainThreadExecutor::new(EventFdWake::new(unsafe { OwnedFd::from_raw_fd(fd) }))
    }

    /// A wake written while the looper is idle must land on the fd the
    /// `ALooper` watches. `ExecutorWake` cannot run on a host machine — it
    /// needs the Android `ALooper` — but it registers `wake().fd()`, the only
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
            fd: executor.wake().fd().as_raw_fd(),
            events: libc::POLLIN,
            revents: 0,
        };
        // SAFETY: `pollfd` points at a live pollfd entry for the call.
        let ready = unsafe { libc::poll(&raw mut pollfd, 1, 0) };
        assert_eq!(
            ready, 1,
            "the wake the executor wrote is not observed on the fd the looper watches"
        );
        let mut count: u64 = 0;
        // SAFETY: `wake_fd` is the live eventfd the spawn wrote; `count`
        // receives the coalesced counter exactly as the looper callback
        // reads it.
        unsafe {
            libc::eventfd_read(executor.wake().fd().as_raw_fd(), &raw mut count);
        }
        assert_eq!(count, 1);

        // The fd callback then drains the queue on the UI thread.
        assert!(executor.drain());
        assert!(ran.get());
        assert!(!executor.drain(), "a drained queue must go quiet again");
    }

    /// The producer wake's transport (water-rs/waterui#2287): a
    /// `UiThreadPoster::post` from another thread — a GPU producer's thread,
    /// not the UI thread — must be observed on the same fd `ExecutorWake`
    /// registers with the `ALooper`, coalesced with every other wake the fd
    /// counted, and run by `drain` where the Choreographer request can post.
    /// Before the poster existed the window's redraw handle had nothing to
    /// call, and a cross-thread wake never reached the looper at all.
    #[test]
    fn a_posted_wake_is_observed_on_the_registered_fd_and_runs_in_drain() {
        let executor = executor();
        let poster = executor.poster();
        let post_ran = Arc::new(AtomicUsize::new(0));

        // A coalesced batch: two posts before any drain still write the same
        // fd the looper watches, exactly as a runnable wake does.
        let flag = Arc::clone(&post_ran);
        std::thread::spawn(move || {
            poster.post(move || {
                flag.fetch_add(1, Ordering::SeqCst);
            });
            poster.post(|| {});
        })
        .join()
        .expect("the producer thread's posts must complete");

        // What `ALooper_pollOnce` would report while idle: the registered
        // fd readable, the wakes already there.
        let mut pollfd = libc::pollfd {
            fd: executor.wake().fd().as_raw_fd(),
            events: libc::POLLIN,
            revents: 0,
        };
        // SAFETY: `pollfd` points at a live pollfd entry for the call.
        let ready = unsafe { libc::poll(&raw mut pollfd, 1, 0) };
        assert_eq!(
            ready, 1,
            "the post is not observed on the fd the looper watches"
        );

        // The fd callback then drains both queues on the UI thread.
        assert!(executor.drain());
        assert_eq!(post_ran.load(Ordering::SeqCst), 1);
        assert!(!executor.drain(), "a drained queue must go quiet again");
    }
}
