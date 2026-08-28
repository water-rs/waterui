//! A `LocalExecutor` for tests that have no event loop.

use core::cell::RefCell;
use core::future::Future;

use executor_core::LocalExecutor;
use executor_core::async_task::{self, AsyncTask, Runnable};

thread_local! {
    /// Parks runnables so dropping them — which cancels the task — is deferred to
    /// thread teardown rather than happening inside `schedule`.
    static PARKED_RUNNABLES: RefCell<Vec<Runnable>> = const { RefCell::new(Vec::new()) };
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
        let (runnable, task) = async_task::spawn_local(fut, |runnable: Runnable| {
            PARKED_RUNNABLES.with(|parked| parked.borrow_mut().push(runnable));
        });
        runnable.schedule();
        task
    }
}

/// Installs [`TestLocalExecutor`] as the process-wide local executor.
///
/// Idempotent, so every test can call it without coordinating with the others.
pub fn install_test_executor() {
    let _ = executor_core::try_init_local_executor(TestLocalExecutor);
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
    let ready = PARKED_RUNNABLES.with(|parked| core::mem::take(&mut *parked.borrow_mut()));
    let count = ready.len();
    for runnable in ready {
        runnable.run();
    }
    count
}
