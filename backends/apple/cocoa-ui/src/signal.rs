//! The signals that ask a process to stop, delivered on the main thread.
//!
//! No Cocoa event carries `SIGINT`, `SIGTERM` or `SIGHUP`, so a process that
//! wants to stop cleanly when a shell, a terminal or a service manager asks
//! has to listen for them itself. [`TerminationSignals`] listens through
//! dispatch signal sources, which observe a signal's delivery on a queue of
//! their own: the first signal reaches the caller's handler on the main
//! thread, as ordinary main-queue work. A signal the launcher set to be
//! ignored — `nohup` ignores `SIGHUP` — stays ignored and is not watched.
//!
//! # Safety
//!
//! The `unsafe` here swaps the process's dispositions for the three signals
//! with `sigaction` and creates dispatch sources for them. The handler it
//! installs does nothing at all — a dispatch source only observes a delivery,
//! so the signal's default action has to be disarmed for the process to
//! survive it — and touches no state. A caught signal resets to its default
//! action across `exec`, so child processes start with the dispositions they
//! would have had anyway; an ignored one would not. The event handler block
//! captures only `Send` and `Sync` values. Its forced exit resets the
//! signal to its default action before raising it, which only ends the
//! process.

use std::cell::Cell;
use std::mem::MaybeUninit;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

use block2::RcBlock;
use dispatch2::{
    _dispatch_source_type_signal, DispatchObject, DispatchQueue, DispatchRetained, DispatchSource,
    MainThreadBound,
};
use objc2::MainThreadMarker;

/// A signal that asks the process to stop.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum TerminationSignal {
    /// `SIGINT`: Ctrl-C in the terminal that started the process.
    Interrupt,
    /// `SIGTERM`: `kill`, `launchd` stopping a job, a supervisor.
    Terminate,
    /// `SIGHUP`: the terminal that started the process went away.
    Hangup,
}

impl TerminationSignal {
    const ALL: [Self; 3] = [Self::Interrupt, Self::Terminate, Self::Hangup];

    /// The signal's number.
    #[must_use]
    pub const fn number(self) -> libc::c_int {
        match self {
            Self::Interrupt => libc::SIGINT,
            Self::Terminate => libc::SIGTERM,
            Self::Hangup => libc::SIGHUP,
        }
    }
}

type Handler = Box<dyn FnOnce(MainThreadMarker, TerminationSignal)>;

/// One watched signal: the source observing it and the disposition it
/// replaced, restored when the watch ends.
struct Watch {
    signal: TerminationSignal,
    source: DispatchRetained<DispatchSource>,
    previous: libc::sigaction,
}

/// Listens for [`TerminationSignal`]s until dropped.
///
/// The first signal of any of the three reaches the handler on the main
/// thread. Every later one — including a second delivery that dispatch
/// merged into the first — ends the process at once by that signal's
/// default action, so the parent sees it die by the real signal, without
/// waiting for the application: it had its chance to stop and did not take
/// it, and a process whose main thread is stuck must stay killable from the
/// terminal that started it. A signal ignored when the guard is installed
/// is left ignored and not watched.
#[must_use = "the signals are only observed while the guard is alive"]
pub struct TerminationSignals {
    watches: Vec<Watch>,
}

impl std::fmt::Debug for TerminationSignals {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_list()
            .entries(self.watches.iter().map(|watch| watch.signal))
            .finish()
    }
}

impl TerminationSignals {
    /// Starts listening; `handler` runs on the main thread for the first
    /// signal, and never again.
    ///
    /// # Panics
    ///
    /// If the system refuses to change a signal's disposition.
    pub fn install(
        mtm: MainThreadMarker,
        handler: impl FnOnce(MainThreadMarker, TerminationSignal) + 'static,
    ) -> Self {
        let handler: Handler = Box::new(handler);
        let handler = Arc::new(MainThreadBound::new(Cell::new(Some(handler)), mtm));
        let delivered = Arc::new(AtomicBool::new(false));
        let queue = DispatchQueue::new("cocoa-ui.termination-signals", None);
        let watches = TerminationSignal::ALL
            .into_iter()
            .filter_map(|signal| {
                let previous = disarm(signal)?;
                // SAFETY: a signal source takes the signal's number as its
                // handle and no mask; see the module safety note.
                let source = unsafe {
                    DispatchSource::new(
                        (&raw const _dispatch_source_type_signal).cast_mut(),
                        usize::try_from(signal.number()).expect("signal numbers are positive"),
                        0,
                        Some(&queue),
                    )
                };
                let handler = Arc::clone(&handler);
                let delivered = Arc::clone(&delivered);
                // The source releases its handler, and with it this
                // reference to itself, once it is cancelled.
                let observed = source.retain();
                let block = RcBlock::new(move || {
                    // Dispatch merges deliveries that arrive before the
                    // handler runs; the merged count says how many there were.
                    let repeated = observed.data() > 1;
                    if delivered.swap(true, Ordering::SeqCst) || repeated {
                        tracing::warn!(
                            ?signal,
                            "termination signal repeated; ending the process without waiting for the application"
                        );
                        die_by(signal);
                        return;
                    }
                    let handler = Arc::clone(&handler);
                    crate::main_queue::enqueue(move |mtm| {
                        let handler = handler
                            .get(mtm)
                            .take()
                            .expect("only the first termination signal reaches the handler");
                        handler(mtm, signal);
                    });
                });
                // SAFETY: dispatch copies the block, which captures only
                // `Send` and `Sync` values; see the module safety note.
                unsafe { source.set_event_handler_with_block(RcBlock::as_ptr(&block)) };
                source.activate();
                Some(Watch {
                    signal,
                    source,
                    previous,
                })
            })
            .collect();
        Self { watches }
    }
}

impl Drop for TerminationSignals {
    fn drop(&mut self) {
        for watch in &self.watches {
            // The disposition goes back first, so a signal arriving while
            // the watch ends takes the action the process had before rather
            // than falling into the do-nothing handler unobserved.
            // SAFETY: restores the disposition `disarm` replaced; see the
            // module safety note.
            let status = unsafe {
                libc::sigaction(
                    watch.signal.number(),
                    &raw const watch.previous,
                    std::ptr::null_mut(),
                )
            };
            assert_eq!(
                status,
                0,
                "restoring the {:?} disposition failed: {}",
                watch.signal,
                std::io::Error::last_os_error()
            );
            // The cancelled source releases its block, and with it possibly
            // the handler's last reference, on the signal queue; that drop
            // waits for the main thread, which never waits on that queue.
            watch.source.cancel();
        }
    }
}

/// Ends the process by `signal`'s default action.
///
/// Runs on the signal queue's worker thread. A dispatch worker blocks the
/// termination signals and cannot take a thread-directed one, so `raise`
/// falls back to signalling the process: the main thread takes the signal
/// and its default action ends the process, possibly just after `raise`
/// returns.
fn die_by(signal: TerminationSignal) {
    // SAFETY: `SIG_DFL` is a valid disposition; see the module safety note.
    let previous = unsafe { libc::signal(signal.number(), libc::SIG_DFL) };
    assert_ne!(
        previous,
        libc::SIG_ERR,
        "resetting {signal:?} to its default action failed: {}",
        std::io::Error::last_os_error()
    );
    // SAFETY: the signal's action is now the default, which ends the process.
    let status = unsafe { libc::raise(signal.number()) };
    assert_eq!(
        status,
        0,
        "raising {signal:?} failed: {}",
        std::io::Error::last_os_error()
    );
}

/// Replaces `signal`'s default action with a handler that does nothing, and
/// answers the disposition it replaced — or, when the launcher set the
/// signal to be ignored, puts that back and answers `None`: the launcher's
/// choice stands, and the signal is not watched.
fn disarm(signal: TerminationSignal) -> Option<libc::sigaction> {
    const extern "C" fn observed_by_dispatch(_: libc::c_int) {}

    // SAFETY: an all-zero `sigaction` is a valid value of the C struct; every
    // field that matters is set below.
    let mut action: libc::sigaction = unsafe { MaybeUninit::zeroed().assume_init() };
    action.sa_sigaction = observed_by_dispatch as extern "C" fn(libc::c_int) as libc::sighandler_t;
    action.sa_flags = libc::SA_RESTART;
    let mut previous = MaybeUninit::<libc::sigaction>::uninit();
    // SAFETY: both pointers are valid for the call; see the module safety
    // note for the handler installed.
    let status =
        unsafe { libc::sigaction(signal.number(), &raw const action, previous.as_mut_ptr()) };
    assert_eq!(
        status,
        0,
        "disarming {signal:?} failed: {}",
        std::io::Error::last_os_error()
    );
    // SAFETY: a successful `sigaction` wrote the previous disposition.
    let previous = unsafe { previous.assume_init() };
    if previous.sa_sigaction != libc::SIG_IGN {
        return Some(previous);
    }
    // SAFETY: puts back the disposition just replaced.
    let status =
        unsafe { libc::sigaction(signal.number(), &raw const previous, std::ptr::null_mut()) };
    assert_eq!(
        status,
        0,
        "restoring the ignored {signal:?} failed: {}",
        std::io::Error::last_os_error()
    );
    None
}
