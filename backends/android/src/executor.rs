//! The `LocalExecutor` bound to the main `Looper`.
//!
//! `native_executor::NativeExecutor` has no place here: a task queued to it
//! runs on its worker threads, never on the main looper — the only thread
//! that may touch views. The binding is the platform's own, mirroring how
//! `native_executor` reaches an `ALooper` on Android: a channel carries the
//! `Runnable`s and a socket pair carries the wake. Scheduling writes the
//! runnable to the channel and one byte to the socket; the main looper
//! wakes on the fd, and the callback — which owns the channel's receiving
//! end — drains and runs everything queued. That is the `CFRunLoop` wake
//! source the Apple executor gets for free, spelled out for `ALooper`.

use std::io::{Read, Write};
use std::os::fd::{AsRawFd, BorrowedFd};
use std::os::unix::net::UnixStream;
use std::sync::Arc;
use std::sync::mpsc::{Sender, channel};

use executor_core::LocalExecutor;
use executor_core::async_task::{AsyncTask, Runnable};
use ndk::looper::{FdEvent, ThreadLooper};

/// The executor handed to `try_init_local_executor`. `!Send`: the pieces it
/// clones are `Send`, but the executor itself stays on the thread that
/// created it — the main thread — because `spawn_local`'s first poll runs
/// inline, and that contract only holds there.
pub(crate) struct LooperExecutor {
    /// Schedules a `Runnable` for the looper's next drain.
    schedule: Sender<Runnable>,
    /// One byte wakes the looper; the bytes themselves carry nothing.
    wake: Arc<UnixStream>,
}

impl LocalExecutor for LooperExecutor {
    type Task<T: 'static> = AsyncTask<T>;

    /// The scheduling closure every spawned `Runnable` carries: a `send`
    /// plus one wake byte. `async-task` requires it `Send + Sync` — both
    /// halves are — so runnables may be scheduled off any thread, while the
    /// run itself stays on the main looper.
    fn spawn_local<Fut>(&self, fut: Fut) -> Self::Task<Fut::Output>
    where
        Fut: std::future::Future + 'static,
    {
        let schedule = self.schedule.clone();
        let wake = self.wake.clone();
        let (runnable, task) = executor_core::async_task::spawn_local(fut, move |runnable| {
            // A failed send means the looper callback is gone — the runtime
            // is being torn down, so the runnable is dead weight anyway.
            if schedule.send(runnable).is_ok() {
                let mut wake: &UnixStream = &wake;
                let _ = wake.write_all(&[1]);
            }
        });
        // The first poll runs inline — `async-task`'s contract for a local
        // executor, and it stays on the creating (main) thread.
        runnable.run();
        task
    }
}

/// Binds the executor to the calling thread's `ALooper`.
///
/// Must run on the main thread: `ThreadLooper::for_thread` answers this
/// thread's looper, and on the main thread that is the main looper every
/// view operation is serialized through. Called once — a second bind would
/// install a second fd callback racing the first.
///
/// # Panics
///
/// When the thread has no `ALooper`, or the looper rejects the fd.
pub(crate) fn install() -> LooperExecutor {
    let (schedule, runnables) = channel::<Runnable>();
    let (mut read, wake) = UnixStream::pair().expect("a Unix socket pair is available");
    read.set_nonblocking(true).expect("O_NONBLOCK set");

    let looper = ThreadLooper::for_thread().expect("the main thread has an ALooper");
    // SAFETY: the raw fd stays open inside the callback — the socket is
    // moved into the closure itself — for as long as the registration
    // stands, and the callback never returns `false` to lift it.
    let fd = unsafe { BorrowedFd::borrow_raw(read.as_raw_fd()) };
    looper
        .add_fd_with_callback(fd, FdEvent::INPUT, move |_fd, _events| {
            // Drain the wake bytes first; a byte with no runnable is
            // possible when `send` won the race but `write` lost it.
            let mut sink = [0u8; 64];
            while read.read(&mut sink).is_ok_and(|n| n > 0) {}
            while let Ok(runnable) = runnables.try_recv() {
                runnable.run();
            }
            // `true` keeps the fd registered — the channel lives as long as
            // the runtime does.
            true
        })
        .expect("the main looper accepts the executor's fd");
    LooperExecutor {
        schedule,
        wake: Arc::new(wake),
    }
}
