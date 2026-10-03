//! The pump-driven local executor every runtime without a platform event
//! loop shares.
//!
//! The headless, semantic, and one-shot `run` render paths own their frame
//! cadence — a pump drains queued runnables where an event loop would. The
//! queue is a plain channel: wakers send `Runnable`s from arbitrary threads
//! and the next drain runs them on the runtime's thread. Compiles on every
//! target, wasm32 included — a semantic runtime in the browser has the same
//! pump-driven shape, just no renderer behind it.

use std::cell::Cell;
use std::rc::Rc;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::{Arc, mpsc};
#[cfg(test)]
use std::sync::{Condvar, Mutex};

use executor_core::LocalExecutor;
use executor_core::async_task::{AsyncTask, Runnable};

#[derive(Clone, Debug)]
pub struct HeadlessMainThreadExecutor {
    runnable_tx: mpsc::Sender<Runnable>,
    runnable_rx: Rc<mpsc::Receiver<Runnable>>,
    /// Queued-but-not-yet-run runnable count. Incremented before send and
    /// decremented after each run, so `has_pending` over-reports around the
    /// hand-off instant — the safe direction for a settledness probe. Atomic
    /// because wakers clone the sender onto arbitrary threads.
    pending: Arc<AtomicUsize>,
    /// Open while a pump-driven runtime on this thread owns the queue.
    ///
    /// The channel's receiver is never disconnected — `mpsc` offers no close —
    /// so this flag is the teardown boundary instead: the last owning runtime
    /// clears it, and every schedule from then on leaks its runnable instead
    /// of queueing it. Without the gate a waker on another thread could land a
    /// runnable after the runtime's final drain, and that runnable would sit
    /// until thread-local teardown dropped it — where dropping a task whose
    /// destructor touches an already-destroyed thread-local aborts the
    /// process (water-rs/hydrolysis#332).
    accepting: Arc<AtomicBool>,
    /// Pump-driven runtimes currently owning the queue. The last one's
    /// teardown closes `accepting` and drains to quiescence; earlier drops
    /// leave the queue open for the survivors. `Cell` because ownership
    /// changes hands only on this thread, unlike the waker-visible atomics.
    owners: Rc<Cell<usize>>,
    /// Count of runnables ever delivered to the channel, bumped under the
    /// mutex and signaled on the condvar by the wake path. Tests wait on this
    /// edge — a real timer re-queueing its task — instead of guessing the
    /// reactor thread's latency with a fixed sleep. The count trails `pending`
    /// deliberately: it moves only once the runnable is in the channel, so a
    /// waiter that wakes on it can drain immediately.
    #[cfg(test)]
    queued: Arc<(Mutex<usize>, Condvar)>,
}

thread_local! {
    /// One executor per thread, shared by every pump-driven runtime on it.
    ///
    /// `try_init_local_executor` installs a single executor per thread and the
    /// first install wins, so per-runtime executors would strand every task a
    /// second runtime spawns on the same thread (perf repetitions,
    /// multi-measure benches, any test mounting twice) in the first runtime's
    /// queue — never drained, and dropped only during thread-local teardown,
    /// where dropping a future whose destructor touches other thread-locals
    /// aborts the process.
    static THREAD_EXECUTOR: HeadlessMainThreadExecutor = HeadlessMainThreadExecutor::new();
}

impl HeadlessMainThreadExecutor {
    fn new() -> Self {
        let (runnable_tx, runnable_rx) = mpsc::channel();
        Self {
            runnable_tx,
            runnable_rx: Rc::new(runnable_rx),
            pending: Arc::new(AtomicUsize::new(0)),
            accepting: Arc::new(AtomicBool::new(true)),
            owners: Rc::new(Cell::new(0)),
            #[cfg(test)]
            queued: Arc::new((Mutex::new(0), Condvar::new())),
        }
    }

    /// The executor shared by every pump-driven runtime on this thread.
    pub(crate) fn thread_shared() -> Self {
        THREAD_EXECUTOR.with(Clone::clone)
    }

    /// Runs every runnable currently queued, returning whether any ran.
    ///
    /// The offscreen runner must call this while rendering: a `GpuView`'s
    /// `setup` is an async future spawned onto this executor, so a frame
    /// rendered without draining would run `render` against a renderer that has
    /// not built its pipelines yet and would emit nothing.
    pub(super) fn drain(&self) -> bool {
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

    /// Whether any spawned work is queued and waiting for the next drain.
    pub(super) fn has_pending(&self) -> bool {
        self.pending.load(Ordering::SeqCst) > 0
    }

    /// Blocks until a waker delivers a runnable to the queue, returning
    /// whether one arrived within `timeout`.
    ///
    /// Tests that arm a real timer — `debounce`'s `async_io::Timer` — must
    /// wait on this wake edge rather than a fixed sleep: a loaded runner can
    /// take arbitrarily long to fire the wake, while an idle one answers in
    /// microseconds. False means the timer never re-queued its task.
    #[cfg(test)]
    pub(super) fn wait_queued(&self, timeout: core::time::Duration) -> bool {
        let (lock, queued) = &*self.queued;
        let deadline = std::time::Instant::now() + timeout;
        // The counter — not `pending` — is the arrival edge: `pending` is
        // incremented before the send, so it can report a runnable the
        // channel does not yet hold. Bump and notify share one lock hold, so
        // a waiter holding the lock across its check and `wait_timeout` can
        // never miss a signal.
        // Each loop turn holds the lock for exactly its own check-then-wait:
        // `wait_timeout` consumes the guard, and the returned post-wait guard
        // is dropped before the next turn re-locks. A producer that signals
        // between turns bumps `queued_count` under its own hold, which the
        // next turn's check still observes — the counter is the edge.
        let baseline = *lock.lock().unwrap();
        loop {
            let (guard, result) = {
                let queued_count = lock.lock().unwrap();
                if *queued_count > baseline || self.has_pending() {
                    return true;
                }
                let Some(remaining) = deadline.checked_duration_since(std::time::Instant::now())
                else {
                    return false;
                };
                // `wait_timeout` consumes the guard and hands back the
                // post-wait hold, so the pre-wait lock is held for exactly the
                // check-then-enqueue window.
                queued.wait_timeout(queued_count, remaining).unwrap()
            };
            if result.timed_out() {
                return self.has_pending();
            }
            drop(guard);
        }
    }
}

impl LocalExecutor for HeadlessMainThreadExecutor {
    type Task<T: 'static> = AsyncTask<T>;

    fn spawn_local<Fut>(&self, fut: Fut) -> Self::Task<Fut::Output>
    where
        Fut: std::future::Future + 'static,
    {
        let runnable_tx = self.runnable_tx.clone();
        let pending = Arc::clone(&self.pending);
        let accepting = Arc::clone(&self.accepting);
        #[cfg(test)]
        let queued = Arc::clone(&self.queued);
        let (runnable, task) = executor_core::async_task::spawn_local(fut, move |runnable| {
            pending.fetch_add(1, Ordering::SeqCst);
            let delivery = if accepting.load(Ordering::SeqCst) {
                runnable_tx.send(runnable).map_err(|unsent| unsent.0)
            } else {
                Err(runnable)
            };
            match delivery {
                Ok(()) => {
                    #[cfg(test)]
                    {
                        let (lock, queued) = &*queued;
                        *lock.lock().unwrap() += 1;
                        queued.notify_all();
                    }
                }
                Err(unsent) => {
                    pending.fetch_sub(1, Ordering::SeqCst);
                    // Teardown race: a waker held by another thread (decoder,
                    // audio, dispatch callback) fired after the last owning
                    // runtime closed the queue. The task can never run again,
                    // and dropping a `spawn_local` runnable off its spawning
                    // thread panics by design (async-task's thread check), so
                    // leak it instead — bounded to shutdown, reclaimed at
                    // process exit.
                    std::mem::forget(unsent);
                }
            }
        });
        runnable.schedule();
        task
    }
}

/// Owns the thread-shared executor on behalf of one pump-driven runtime.
///
/// The executor itself lives in a lazily destroyed thread-local, but its
/// queue must not: a runnable still queued at thread exit is dropped inside
/// the thread-local destructor, where a task future whose drop touches an
/// already-destroyed thread-local aborts the process. The last owning
/// runtime's drop therefore closes the queue and drains it to quiescence
/// while this thread's locals are still alive.
#[derive(Debug)]
pub(super) struct DrainExecutorOnDrop(HeadlessMainThreadExecutor);

impl DrainExecutorOnDrop {
    pub(super) fn new(executor: HeadlessMainThreadExecutor) -> Self {
        executor.owners.set(executor.owners.get() + 1);
        // Reopen the queue a previous runtime's teardown closed: this runtime
        // drains it from now on, so work queued through the shared executor
        // runs again.
        executor.accepting.store(true, Ordering::SeqCst);
        Self(executor)
    }
}

impl Drop for DrainExecutorOnDrop {
    fn drop(&mut self) {
        let remaining = self.0.owners.get() - 1;
        self.0.owners.set(remaining);
        if remaining > 0 {
            // A later-mounted runtime on this thread still pumps the shared
            // queue; run what is already queued so this teardown strands
            // nothing, then leave the queue open.
            let _ = self.0.drain();
            return;
        }
        // Last owner on the thread: close the queue first so a waker on
        // another thread can no longer land a runnable in it, then drain until
        // nothing is queued and no send is in flight (`pending` counts both —
        // it is incremented before the schedule closure reaches its send).
        // Anything that still loses the race is leaked by the send path, so
        // the channel is guaranteed empty for thread-local teardown (#332).
        self.0.accepting.store(false, Ordering::SeqCst);
        while self.0.has_pending() {
            self.0.drain();
            std::thread::yield_now();
        }
    }
}
