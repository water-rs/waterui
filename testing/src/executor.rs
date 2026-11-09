//! A `LocalExecutor` for tests that have no event loop.

use core::future::Future;
use core::time::Duration;
use std::cell::Cell;
use std::sync::mpsc::{self, Receiver, RecvTimeoutError, Sender};
use std::sync::{
    Arc,
    atomic::{AtomicUsize, Ordering},
};

use executor_core::LocalExecutor;
use executor_core::async_task::{self, AsyncTask, Runnable};

/// The parked queue for one thread's [`TestLocalExecutor`].
///
/// The sender half is cloned into each task's schedule closure so a waker
/// fires correctly no matter which thread it runs on — a timer or I/O reactor
/// wakes from its own thread, not the one that spawned the task. `pending`
/// counts runnables sent but not yet run; it is atomic for the same reason.
struct ParkedWork {
    sender: Sender<Runnable>,
    receiver: Receiver<Runnable>,
    pending: Arc<AtomicUsize>,
}

thread_local! {
    /// Whether [`install_test_executor`] installed the parked queue as this
    /// thread's local executor. When a runtime installed its own first, the
    /// runtime's executor is the one whose wake edge a wait must observe.
    // Android's emulated TLS expands even a `const { … }` initializer into a
    // non-const init fn, which the lint inspects; native-TLS targets keep it
    // const, so the expectation is Android-only.
    #[cfg_attr(
        target_os = "android",
        expect(
            clippy::missing_const_for_thread_local,
            reason = "the initializer is already const; the lint misfires on std's emulated-TLS expansion (rust-lang/rust-clippy#13422)"
        )
    )]
    static INSTALLED: Cell<bool> = const { Cell::new(false) };
    /// Parks runnables so dropping them — which cancels the task — is deferred to
    /// thread teardown rather than happening inside `schedule`.
    static PARKED_WORK: ParkedWork = {
        let (sender, receiver) = mpsc::channel();
        ParkedWork {
            sender,
            receiver,
            pending: Arc::new(AtomicUsize::new(0)),
        }
    };
}

/// Queues `spawn_local` work without running it.
///
/// `NativeExecutor` cannot fill this slot. Its main-thread half is
/// `NativeMainExecutor`, which only exists where a platform main thread has been
/// established — and a test binary establishes none, since thread assignment
/// belongs to the harness. This mirrors what Apple's dispatch backend does in a
/// test anyway: `spawn_local` hands the future to the main queue, and a unit
/// test runs no main loop, so it is never polled.
///
/// Runnables are deliberately not run inline. Reactive work re-enters the code
/// under test, which deadlocks when polled in the middle of the call that
/// spawned it.
#[derive(Clone, Copy, Debug, Default)]
pub struct TestLocalExecutor;

impl LocalExecutor for TestLocalExecutor {
    type Task<T: 'static> = AsyncTask<T>;

    fn spawn_local<Fut>(&self, fut: Fut) -> Self::Task<Fut::Output>
    where
        Fut: Future + 'static,
    {
        let (sender, pending) =
            PARKED_WORK.with(|parked| (parked.sender.clone(), Arc::clone(&parked.pending)));
        let (runnable, task) = async_task::spawn_local(fut, move |runnable: Runnable| {
            pending.fetch_add(1, Ordering::SeqCst);
            if let Err(unsent) = sender.send(runnable) {
                pending.fetch_sub(1, Ordering::SeqCst);
                // The queue is gone — the owning thread is tearing down. A
                // `spawn_local` runnable dropped off its thread panics, so leak
                // it instead, matching `HeadlessMainThreadExecutor`.
                std::mem::forget(unsent.0);
            }
        });
        runnable.schedule();
        task
    }
}

/// Installs [`TestLocalExecutor`] as the local executor for this thread.
///
/// The executor is wrapped in [`waterui::task::MonitoredLocalExecutor`] so the
/// framework's refresh-rate probes see the headless rate. A session waits on
/// work parked on an external wake through the parked queue itself: a waker
/// sends the runnable, which ends the wait.
///
/// Idempotent, so every test can call it without coordinating with the others.
pub fn install_test_executor() {
    if executor_core::try_init_local_executor(waterui::task::monitored_local_executor(
        TestLocalExecutor,
        waterui::task::RefreshRate::HEADLESS,
    ))
    .is_ok()
    {
        INSTALLED.with(|installed| installed.set(true));
    }
}

/// Blocks until a waker parks a runnable on this thread's queue, or `timeout`
/// elapses, and runs what arrived; returns whether a runnable ran.
///
/// `None` when this thread's local executor is not [`TestLocalExecutor`]:
/// the runtime installed its own, and its wake edge is the one to wait on.
/// Called between frames of a pump loop, where running a runnable cannot
/// re-enter the code under test.
pub fn wait_parked_local_work(timeout: Duration) -> Option<bool> {
    if !INSTALLED.with(Cell::get) {
        return None;
    }
    Some(
        PARKED_WORK.with(|parked| match parked.receiver.recv_timeout(timeout) {
            Ok(runnable) => {
                runnable.run();
                parked.pending.fetch_sub(1, Ordering::SeqCst);
                true
            }
            // The sender lives in this thread-local, so the queue cannot
            // disconnect while it is read.
            Err(RecvTimeoutError::Timeout | RecvTimeoutError::Disconnected) => false,
        }),
    )
}

/// Runs the work `spawn_local` parked, returning how many runnables ran.
///
/// Parking exists so a runnable never polls in the middle of the call that
/// spawned it, which would re-enter the code under test. Draining is therefore
/// only safe from a point that is not inside such a call — between frames of a
/// pump loop is the intended one.
///
/// Each drained runnable may spawn more work; this runs only what was already
/// parked, so a task that reschedules itself cannot spin forever here.
#[must_use]
pub fn drain_parked_local_work() -> usize {
    PARKED_WORK.with(|parked| {
        let budget = parked.pending.load(Ordering::SeqCst);
        let mut ran = 0;
        while ran < budget {
            match parked.receiver.try_recv() {
                Ok(runnable) => {
                    runnable.run();
                    parked.pending.fetch_sub(1, Ordering::SeqCst);
                    ran += 1;
                }
                // A waker on another thread counted its send before this read;
                // the runnable lands for the next drain.
                Err(_) => break,
            }
        }
        ran
    })
}
