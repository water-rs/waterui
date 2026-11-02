//! A `spawn_local` executor for the runtime's unit tests that parks runnables
//! until [`drain`] runs them, so a hook future's progress is observable step
//! by step without waiting on any clock.
//!
//! The local executor is process-global and the first install wins, so every
//! test module that needs one shares this one: two modules installing two
//! different parked executors would leave the loser's [`drain`] reading a
//! queue nothing parks into.

use core::future::Future;
use std::sync::mpsc;

use executor_core::LocalExecutor;
use executor_core::async_task::{self, AsyncTask, Runnable};

struct ParkedExecutor;

thread_local! {
    static PARKED: (mpsc::Sender<Runnable>, mpsc::Receiver<Runnable>) = mpsc::channel();
}

impl LocalExecutor for ParkedExecutor {
    type Task<T: 'static> = AsyncTask<T>;

    fn spawn_local<Fut>(&self, fut: Fut) -> Self::Task<Fut::Output>
    where
        Fut: Future + 'static,
    {
        let (runnable, task) = async_task::spawn_local(fut, |runnable| {
            PARKED.with(|(sender, _)| {
                if let Err(unsent) = sender.send(runnable) {
                    // The queue is gone at thread teardown; dropping a
                    // `spawn_local` runnable off its thread panics.
                    std::mem::forget(unsent.0);
                }
            });
        });
        runnable.schedule();
        task
    }
}

/// Installs the parked executor as this process's local executor.
pub(super) fn install() {
    let _ = executor_core::try_init_local_executor(ParkedExecutor);
}

/// Runs every parked runnable, and every one those park in turn, until the
/// queue is empty.
pub(super) fn drain() {
    PARKED.with(|(_, receiver)| {
        while let Ok(runnable) = receiver.try_recv() {
            runnable.run();
        }
    });
}
