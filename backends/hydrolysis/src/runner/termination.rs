//! Termination signals for the windowed runner on Unix.
//!
//! No windowing system turns SIGINT, SIGTERM or SIGHUP into a winit event, so
//! the runner listens for them itself. [`TerminationSignals`] watches the
//! set: a signal's first delivery is the graceful request it hands to
//! `handler`, and every delivery after that ends the process by the signal's
//! own default action — the disposition reset to `SIG_DFL`, the signal
//! unblocked and re-raised — because a process whose event loop wedged must
//! stay killable from the terminal that started it, and the parent reads a
//! death by the signal, not an exit status invented on a handler thread.
//!
//! First-vs-repeated is decided inside the signal handler, not on the watcher
//! thread: the exfiltrator behind `Signals` holds one flag per signal, so
//! deliveries that arrive before the watcher reads merge into one and a
//! repeat counted there could go unseen. Each watched signal registers a
//! conditional-default action over a shared `armed` flag ahead of the action
//! that sets it — the registry runs a signal's actions in registration
//! order — so the first delivery arms the flag and any later one takes the
//! default action. After the guard drops, the conditional-default actions
//! stay registered with the flag armed: unregistering cannot hand the signal
//! back to its previous handler, and the emulated default is the `SIG_DFL`
//! behaviour the process had at launch. A repeat is a delivery after the
//! first one ran: the kernel itself merges a second standard signal sent
//! while the first is still pending into one delivery.
//!
//! A signal the launcher set to `SIG_IGN` — `nohup`'s SIGHUP, SIGINT in the
//! background job of a non-interactive shell — stays ignored and is never
//! watched: the launcher's choice stands. The Apple backend implements the
//! same contract in `cocoa_ui::signal` (`TerminationSignals`); sharing the
//! signal-source logic through `waterui-backend-core` would make it a
//! foundation contract, so this keeps it local to the runner.

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::thread::JoinHandle;

use signal_hook::iterator::{Handle, Signals};
use signal_hook::low_level::unregister;
use signal_hook::{SigId, flag};

/// A signal that asks a windowed process to stop.
#[derive(Debug, Clone, Copy)]
pub(super) enum TerminationSignal {
    /// `SIGINT`: Ctrl-C in the terminal that started the process.
    Interrupt,
    /// `SIGTERM`: `kill`, a service manager or the desktop session ending.
    Terminate,
    /// `SIGHUP`: the terminal that started the process went away.
    Hangup,
}

impl TerminationSignal {
    /// The watched set.
    const ALL: [Self; 3] = [Self::Interrupt, Self::Terminate, Self::Hangup];

    /// The signal's number.
    const fn number(self) -> libc::c_int {
        match self {
            Self::Interrupt => libc::SIGINT,
            Self::Terminate => libc::SIGTERM,
            Self::Hangup => libc::SIGHUP,
        }
    }

    /// The signal `number` names; the watcher only ever yields signals it was
    /// built with.
    fn from_number(number: libc::c_int) -> Self {
        match number {
            libc::SIGINT => Self::Interrupt,
            libc::SIGTERM => Self::Terminate,
            libc::SIGHUP => Self::Hangup,
            _ => unreachable!("the watcher only delivers the signals it was built with"),
        }
    }
}

/// Whether `signal`'s disposition at launch was `SIG_IGN`.
///
/// signal-hook has no query primitive, so this is one read-only `sigaction`
/// ahead of the registrations `Signals::new` performs.
fn ignored_at_launch(signal: TerminationSignal) -> bool {
    let mut previous = std::mem::MaybeUninit::<libc::sigaction>::uninit();
    // SAFETY: a null `act` makes `sigaction` a pure query, and on success
    // `previous` is fully written.
    let status =
        unsafe { libc::sigaction(signal.number(), std::ptr::null(), previous.as_mut_ptr()) };
    assert_eq!(
        status,
        0,
        "hydrolysis runner: querying the {signal:?} disposition failed: {}",
        std::io::Error::last_os_error()
    );
    // SAFETY: `sigaction` succeeded, so `previous` was written.
    unsafe { previous.assume_init() }.sa_sigaction == libc::SIG_IGN
}

/// Watches the termination signals while it is alive.
///
/// The first delivery of a watched signal runs `handler`; a repeated delivery
/// ends the process by that signal's default action instead — the decision
/// is the shared [`TerminationSignals::armed`] flag's, taken inside the
/// signal handler, so merged deliveries cannot hide a repeat.
#[derive(Debug)]
#[must_use = "the signals are only watched while the guard is alive"]
pub(super) struct TerminationSignals {
    /// Closes the delivery pipe on drop, ending the watcher thread's
    /// iterator; the `Signals` the thread owns unregisters the deliveries
    /// when it drops at the thread's end.
    handle: Handle,
    /// The arming actions, one per watched signal; `drop` unregisters them.
    /// The conditional-default actions are deliberately not tracked: they
    /// stay registered past `drop` so a late signal still dies by its
    /// default action.
    arming: Vec<SigId>,
    /// Set by the first delivery of any watched signal, and by `drop`: a
    /// delivery that finds it armed takes the signal's default action.
    armed: Arc<AtomicBool>,
    /// Reads the deliveries; `Option` so `drop` can join it.
    thread: Option<JoinHandle<()>>,
}

impl TerminationSignals {
    /// Starts watching; `handler` runs once, for the first delivered signal,
    /// on the watcher thread. Signals the launcher ignored stay ignored.
    ///
    /// # Panics
    ///
    /// When a disposition cannot be queried, the signals cannot be
    /// registered, or the watcher thread cannot be spawned.
    pub(super) fn install(handler: impl FnOnce(TerminationSignal) + Send + 'static) -> Self {
        let watched: Vec<libc::c_int> = TerminationSignal::ALL
            .iter()
            .copied()
            .filter(|signal| !ignored_at_launch(*signal))
            .map(TerminationSignal::number)
            .collect();
        // One flag decides first-vs-repeated inside the signal handler
        // itself. Registry actions run in registration order, so the
        // conditional default must precede the arming action: the first
        // delivery finds `armed` clear and only sets it, and a later one
        // finds it set and dies by the signal's default action.
        let armed = Arc::new(AtomicBool::new(false));
        let mut arming = Vec::with_capacity(watched.len());
        for &signal in &watched {
            flag::register_conditional_default(signal, Arc::clone(&armed))
                .expect("hydrolysis runner: failed to register the termination default");
            arming.push(
                flag::register(signal, Arc::clone(&armed))
                    .expect("hydrolysis runner: failed to arm the termination flag"),
            );
        }
        // Registered after the flag actions, the delivery reaches the watcher
        // only while `armed` is still clear — the one graceful request.
        let mut signals = Signals::new(&watched)
            .expect("hydrolysis runner: failed to watch the termination signals");
        let handle = signals.handle();
        let mut handler = Some(handler);
        let thread = std::thread::Builder::new()
            .name("hydrolysis-termination".into())
            .spawn(move || {
                for number in &mut signals {
                    let handler = handler
                        .take()
                        .expect("only the first termination signal is the graceful request");
                    handler(TerminationSignal::from_number(number));
                }
            })
            .expect("hydrolysis runner: failed to spawn the termination watcher");
        Self {
            handle,
            arming,
            armed,
            thread: Some(thread),
        }
    }
}

impl Drop for TerminationSignals {
    fn drop(&mut self) {
        // Arming turns every later delivery into a default-action death
        // through the conditional-default actions, which stay registered:
        // `unregister` never restores the previous handler, and the emulated
        // default is the `SIG_DFL` the process had at launch. The arming
        // actions and the `Signals` delivery do unregister.
        self.armed.store(true, Ordering::SeqCst);
        for &action in &self.arming {
            unregister(action);
        }
        self.handle.close();
        if let Some(thread) = self.thread.take()
            && let Err(panic) = thread.join()
        {
            std::panic::resume_unwind(panic);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::TerminationSignals;
    use std::io::{BufRead, BufReader, Write};
    use std::process::{Child, ChildStderr, Command, ExitStatus, Stdio};
    use std::sync::mpsc::{Receiver, RecvTimeoutError, TryRecvError};
    use std::time::Duration;

    /// Turns this test binary's re-execution into the child half of the tests.
    const CHILD_ENV: &str = "WATERUI_HYDROLYSIS_TERMINATION_CHILD";
    /// Marks the child whose launcher — itself, before the watcher installs —
    /// ignores SIGHUP.
    const IGNORE_HUP_ENV: &str = "WATERUI_HYDROLYSIS_TERMINATION_IGNORE_HUP";

    /// The protocol lines the parent waits on.
    const READY: &str = "ready";

    /// The bound on every wait for the child: a wedged child fails the test
    /// instead of hanging it.
    const TIMEOUT: Duration = Duration::from_secs(30);

    /// Writes a protocol line; stderr is the report channel because the
    /// child's libtest runner writes its own output to stdout.
    fn report(line: &str) {
        let mut stderr = std::io::stderr().lock();
        writeln!(stderr, "{line}")
            .and_then(|()| stderr.flush())
            .expect("child: the report pipe broke");
    }

    /// The child half: ignores SIGHUP first when the launcher is modeled as
    /// having done so, installs the watcher, reports its readiness and the
    /// one graceful request, then parks — only a signal can end it.
    fn child_main() -> ! {
        if std::env::var_os(IGNORE_HUP_ENV).is_some() {
            // SAFETY: sets this process's SIGHUP disposition to SIG_IGN ahead
            // of `install`, reproducing a launcher that ignored it (`nohup`).
            unsafe {
                libc::signal(libc::SIGHUP, libc::SIG_IGN);
            }
        }
        let _signals = TerminationSignals::install(|signal| {
            report(&format!("requested {}", signal.number()));
        });
        report(READY);
        loop {
            std::thread::park();
        }
    }

    /// The child-process entry point: a no-op in the parent's own run, the
    /// child main when `CHILD_ENV` marks the re-execution.
    #[test]
    fn termination_signal_child() {
        if std::env::var_os(CHILD_ENV).is_none() {
            return;
        }
        child_main();
    }

    /// Owns the child end-to-end: the pid to signal it, and the exit status
    /// its waiter thread reports. Dropping the guard kills and reaps a
    /// still-running child, so a failed assertion cannot leak it.
    #[derive(Debug)]
    struct ChildGuard {
        /// The child's pid; only the waiter thread reaps, so the pid names
        /// the child until a status has been reported.
        pid: libc::pid_t,
        /// Receives the child's exit status once from the waiter thread.
        status: Receiver<ExitStatus>,
    }

    impl ChildGuard {
        /// Sends `signal` to the child.
        fn send(&self, signal: libc::c_int) {
            // SAFETY: `kill` delivers `signal` to the child this test owns.
            let status = unsafe { libc::kill(self.pid, signal) };
            assert_eq!(
                status,
                0,
                "sending signal {signal} to the child failed: {}",
                std::io::Error::last_os_error()
            );
        }

        /// Asserts the child has not exited; `why` says what should have
        /// kept it alive.
        fn assert_running(&self, why: &str) {
            match self.status.try_recv() {
                Err(TryRecvError::Empty) => {}
                Ok(status) => panic!("{why}: the child already exited ({status:?})"),
                Err(TryRecvError::Disconnected) => panic!("{why}: the child's waiter is gone"),
            }
        }

        /// Waits — bounded by [`TIMEOUT`] — for the child to die and asserts
        /// it died by `signal`: the default disposition, not an exit status.
        fn assert_dies_by(&self, signal: libc::c_int) {
            use std::os::unix::process::ExitStatusExt;
            let status = match self.status.recv_timeout(TIMEOUT) {
                Ok(status) => status,
                Err(RecvTimeoutError::Timeout) => {
                    panic!("the child did not exit within {TIMEOUT:?}")
                }
                Err(RecvTimeoutError::Disconnected) => panic!("the child's waiter is gone"),
            };
            assert_eq!(
                status.signal(),
                Some(signal),
                "the child must die by the signal itself, not exit ({status:?})"
            );
        }
    }

    impl Drop for ChildGuard {
        fn drop(&mut self) {
            // A reported status, or a waiter that already reported and
            // exited, means the child was reaped: its pid may name another
            // process by now, so it must not be signalled.
            match self.status.try_recv() {
                Ok(_) | Err(TryRecvError::Disconnected) => return,
                Err(TryRecvError::Empty) => {}
            }
            // SAFETY: SIGKILL to the child this test owns; no status was
            // reported, so the pid is still its — only the waiter thread
            // holds the `Child` that could reap it.
            unsafe {
                libc::kill(self.pid, libc::SIGKILL);
            }
            // SIGKILL cannot be blocked or caught, so the waiter reaps and
            // reports well inside the bound; a waiter that already died
            // disconnects the channel at once.
            let _ = self.status.recv_timeout(TIMEOUT);
        }
    }

    /// Blocks on `child.wait()` off the test thread, so the tests' waits on
    /// the exit status are channel receives with a timeout, not an unbounded
    /// wait of their own.
    fn wait_thread(mut child: Child) -> Receiver<ExitStatus> {
        let (sender, receiver) = std::sync::mpsc::channel();
        std::thread::Builder::new()
            .name("hydrolysis-termination-wait".into())
            .spawn(move || {
                let _ = sender.send(child.wait().expect("waiting on the child"));
            })
            .expect("the child's waiter spawns");
        receiver
    }

    /// Forwards the child's protocol lines — `ready` and `requested <n>` —
    /// from a `read_line` loop on its own thread, skipping anything else its
    /// stderr carries (panic noise, when it fails). EOF or a read error ends
    /// the thread, and the channel's disconnect is how a death shows up in
    /// the protocol reads.
    fn protocol_lines(stderr: ChildStderr) -> Receiver<String> {
        let (sender, receiver) = std::sync::mpsc::channel();
        std::thread::Builder::new()
            .name("hydrolysis-termination-protocol".into())
            .spawn(move || {
                let mut lines = BufReader::new(stderr);
                loop {
                    let mut line = String::new();
                    let Ok(read) = lines.read_line(&mut line) else {
                        break;
                    };
                    if read == 0 {
                        break;
                    }
                    let line = line.trim_end();
                    if (line == READY || line.starts_with("requested "))
                        && sender.send(line.to_owned()).is_err()
                    {
                        break;
                    }
                }
            })
            .expect("the child's protocol reader spawns");
        receiver
    }

    /// The child's next protocol line, bounded by [`TIMEOUT`].
    fn recv_protocol_line(lines: &Receiver<String>, while_waiting_for: &str) -> String {
        match lines.recv_timeout(TIMEOUT) {
            Ok(line) => line,
            Err(RecvTimeoutError::Timeout) => {
                panic!("the child did not report {while_waiting_for} within {TIMEOUT:?}")
            }
            Err(RecvTimeoutError::Disconnected) => {
                panic!("the child died before {while_waiting_for}")
            }
        }
    }

    /// Re-executes this test binary as the watched child; `ignore_hup` asks it
    /// to ignore SIGHUP before installing the watcher.
    fn spawn_child(ignore_hup: bool) -> (ChildGuard, Receiver<String>) {
        let mut command = Command::new(std::env::current_exe().expect("the test binary's path"));
        command
            .arg("--exact")
            .arg("runner::termination::tests::termination_signal_child")
            // libtest captures `stdout()` writes by default; the protocol must
            // reach the pipe.
            .arg("--nocapture")
            .env(CHILD_ENV, "1")
            // libtest's own output stays on stdout; the protocol is stderr.
            .stdout(Stdio::null())
            .stderr(Stdio::piped());
        if ignore_hup {
            command.env(IGNORE_HUP_ENV, "1");
        }
        let mut child = command.spawn().expect("the child spawns");
        let stderr = child.stderr.take().expect("the child's stderr is piped");
        let pid = libc::pid_t::try_from(child.id()).expect("the child's pid fits pid_t");
        let guard = ChildGuard {
            pid,
            status: wait_thread(child),
        };
        (guard, protocol_lines(stderr))
    }

    /// Two SIGTERMs, the graceful request observed between them: the first
    /// reaches the handler, the second ends the process by the signal
    /// itself.
    #[test]
    fn a_repeated_termination_signal_dies_by_that_signal() {
        let (child, lines) = spawn_child(false);
        assert_eq!(
            recv_protocol_line(&lines, "readiness"),
            READY,
            "the child must report its watcher installed"
        );
        child.send(libc::SIGTERM);
        assert_eq!(
            recv_protocol_line(&lines, "the graceful request"),
            format!("requested {}", libc::SIGTERM),
            "the first signal must reach the handler as a request"
        );
        child.assert_running("the first signal asks; it does not kill");
        child.send(libc::SIGTERM);
        child.assert_dies_by(libc::SIGTERM);
    }

    /// A signal the launcher ignored (`nohup`'s SIGHUP) stays ignored: it is
    /// neither the graceful request nor the forced exit, and the later SIGTERM
    /// arrives as the first request.
    #[test]
    fn a_signal_ignored_at_launch_stays_ignored() {
        let (child, lines) = spawn_child(true);
        assert_eq!(
            recv_protocol_line(&lines, "readiness"),
            READY,
            "the child must report its watcher installed"
        );
        child.send(libc::SIGHUP);
        child.send(libc::SIGTERM);
        assert_eq!(
            recv_protocol_line(&lines, "the graceful request"),
            format!("requested {}", libc::SIGTERM),
            "SIGHUP must not have reached the watcher — the request must be the SIGTERM"
        );
        child.assert_running("an ignored SIGHUP followed by one SIGTERM leaves the child alive");
        child.send(libc::SIGTERM);
        child.assert_dies_by(libc::SIGTERM);
    }
}
