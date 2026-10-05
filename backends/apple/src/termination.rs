//! The application's termination hooks on `AppKit`'s quit path.
//!
//! Every user and system quit on macOS — the Quit menu item and its ⌘Q,
//! Quit in the Dock menu, a quit Apple event, a logout, restart or
//! shutdown — arrives as `applicationShouldTerminate:`. The delegate files
//! a [`Cancellable`](TerminationKind::Cancellable) request with the
//! termination machine and answers `NSTerminateLater`; the machine's
//! decision comes back through [`AppKitTerminationHost`] as
//! `replyToApplicationShouldTerminate:`.
//!
//! A logout, restart or shutdown is cancellable too: `AppKit` asks the
//! application before the session ends, and a `NO` reply cancels the logout
//! instead of being overruled, so `on_quit_request` gets its say as it does
//! for ⌘Q. The quit Apple event's `kAEQuitReason` attribute tells these
//! apart from an ordinary quit, but nothing here answers differently for it.
//!
//! The quits `AppKit` does not ask about are
//! [`Required`](TerminationKind::Required): a termination signal, the last
//! window closing under [`LastWindowPolicy::Quit`], and a launch that
//! declares no window under it. Once the machine finishes one of those, or a
//! [`Quit`](waterui::app::Quit) request from the application, no question is
//! open, so the host sends `terminate:` itself — and the delegate answers
//! that one `NSTerminateNow` without filing it with the machine again.

use alloc::rc::Rc;
use core::cell::Cell;

use cocoa_ui::MainThreadMarker;
use cocoa_ui::appkit::{Application, TerminateReply};
use cocoa_ui::signal::TerminationSignals;
use waterui::app::{
    LastWindowPolicy, Termination, TerminationHandle, TerminationHost, TerminationKind,
};
use waterui_backend_core::Environment;

/// The `AppKit` calls the gate answers through.
trait QuitTarget: 'static {
    /// `replyToApplicationShouldTerminate:`.
    fn reply(&self, terminate: bool);
    /// `terminate:` — which asks `applicationShouldTerminate:` again.
    fn terminate(&self);
}

impl QuitTarget for Application {
    fn reply(&self, terminate: bool) {
        self.reply_to_should_terminate(terminate);
    }
    fn terminate(&self) {
        Self::terminate(self);
    }
}

/// Where `AppKit`'s quit question stands.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Phase {
    /// No question is open.
    Idle,
    /// `applicationShouldTerminate:` is filing its request; the machine has
    /// not answered yet.
    Deciding,
    /// The machine answered while `applicationShouldTerminate:` was still
    /// filing — the delegate returns the answer directly.
    Decided(bool),
    /// `applicationShouldTerminate:` answered `NSTerminateLater`; `AppKit`
    /// waits for the reply.
    Pending,
    /// Termination proceeds: the next `applicationShouldTerminate:` is the
    /// host's own `terminate:` and quits.
    Exiting,
}

/// Matches the machine's answers to `AppKit`'s quit question.
struct QuitGate<T> {
    target: T,
    phase: Cell<Phase>,
}

impl<T: QuitTarget> QuitGate<T> {
    const fn new(target: T) -> Self {
        Self {
            target,
            phase: Cell::new(Phase::Idle),
        }
    }

    /// `applicationShouldTerminate:`: files the quit through `request`,
    /// unless termination already proceeds or a question is already open.
    fn should_terminate(&self, request: impl FnOnce()) -> TerminateReply {
        match self.phase.get() {
            Phase::Exiting => return TerminateReply::Now,
            // `AppKit` is already waiting for the open question's reply.
            Phase::Pending => return TerminateReply::Later,
            Phase::Idle => {}
            phase @ (Phase::Deciding | Phase::Decided(_)) => {
                unreachable!("applicationShouldTerminate: re-entered while {phase:?}")
            }
        }
        self.phase.set(Phase::Deciding);
        request();
        match self.phase.get() {
            Phase::Deciding => {
                self.phase.set(Phase::Pending);
                TerminateReply::Later
            }
            Phase::Decided(true) => {
                self.phase.set(Phase::Exiting);
                TerminateReply::Now
            }
            Phase::Decided(false) => {
                self.phase.set(Phase::Idle);
                TerminateReply::Cancel
            }
            phase @ (Phase::Idle | Phase::Pending | Phase::Exiting) => {
                unreachable!("the machine left the quit question {phase:?} while it was filed")
            }
        }
    }

    /// The machine's `terminate`.
    fn terminate(&self) {
        match self.phase.replace(Phase::Exiting) {
            Phase::Idle => self.target.terminate(),
            Phase::Deciding => self.phase.set(Phase::Decided(true)),
            Phase::Pending => self.target.reply(true),
            phase @ (Phase::Decided(_) | Phase::Exiting) => {
                unreachable!("the machine reported terminate while the quit question was {phase:?}")
            }
        }
    }

    /// The machine's `refuse`. A refused [`Quit`](waterui::app::Quit)
    /// request opened no question, so nothing answers it.
    fn refuse(&self) {
        match self.phase.get() {
            Phase::Idle => {}
            Phase::Deciding => self.phase.set(Phase::Decided(false)),
            Phase::Pending => {
                self.phase.set(Phase::Idle);
                self.target.reply(false);
            }
            phase @ (Phase::Decided(_) | Phase::Exiting) => {
                unreachable!("the machine reported refuse while the quit question was {phase:?}")
            }
        }
    }
}

/// The termination machine's [`TerminationHost`] on macOS.
struct AppKitTerminationHost<T>(Rc<QuitGate<T>>);

impl<T: QuitTarget> TerminationHost for AppKitTerminationHost<T> {
    fn terminate(&self) {
        self.0.terminate();
    }
    fn refuse(&self) {
        self.0.refuse();
    }
}

/// The running termination machine and every macOS path into it.
pub struct Session {
    termination: TerminationHandle,
    gate: Rc<QuitGate<Application>>,
    last_window: LastWindowPolicy,
    _signals: TerminationSignals,
}

impl Session {
    /// Starts the machine on `env` — the environment windows and menus are
    /// realized under, which receives the [`Quit`](waterui::app::Quit)
    /// service — and turns the termination signals into required requests.
    pub fn start(
        termination: Termination,
        env: &mut Environment,
        last_window: LastWindowPolicy,
        mtm: MainThreadMarker,
    ) -> Self {
        let gate = Rc::new(QuitGate::new(Application::shared(mtm)));
        let termination = termination.start(env, AppKitTerminationHost(Rc::clone(&gate)));
        let signals = TerminationSignals::install(mtm, {
            let termination = termination.clone();
            move |_, signal| {
                tracing::info!(?signal, "termination signal: terminating the application");
                termination.request(TerminationKind::Required);
            }
        });
        Self {
            termination,
            gate,
            last_window,
            _signals: signals,
        }
    }

    /// `applicationShouldTerminate:`.
    pub fn should_terminate(&self) -> TerminateReply {
        self.gate
            .should_terminate(|| self.termination.request(TerminationKind::Cancellable))
    }

    /// No window is left — the last one closed, or the launch declared
    /// none: under [`LastWindowPolicy::Quit`] the application ends, as a
    /// required request.
    pub fn no_window_left(&self) {
        if matches!(self.last_window, LastWindowPolicy::Quit) {
            self.termination.request(TerminationKind::Required);
        }
    }
}

#[cfg(test)]
mod tests {
    use alloc::rc::Rc;
    use alloc::vec::Vec;
    use core::cell::RefCell;

    use cocoa_ui::appkit::TerminateReply;

    use super::{QuitGate, QuitTarget};

    /// What the gate sent to `AppKit`.
    #[derive(Debug, Clone, Copy, PartialEq, Eq)]
    enum Sent {
        Reply(bool),
        Terminate,
    }

    #[derive(Clone, Default)]
    struct FakeApp(Rc<RefCell<Vec<Sent>>>);

    impl FakeApp {
        fn sent(&self) -> Vec<Sent> {
            self.0.borrow().clone()
        }
    }

    impl QuitTarget for FakeApp {
        fn reply(&self, terminate: bool) {
            self.0.borrow_mut().push(Sent::Reply(terminate));
        }
        fn terminate(&self) {
            self.0.borrow_mut().push(Sent::Terminate);
        }
    }

    fn new_gate() -> (Rc<QuitGate<FakeApp>>, FakeApp) {
        let app = FakeApp::default();
        (Rc::new(QuitGate::new(app.clone())), app)
    }

    #[test]
    fn quit_gate_answers_a_synchronous_decision_directly() {
        let (gate, app) = new_gate();
        assert_eq!(
            gate.should_terminate(|| gate.terminate()),
            TerminateReply::Now
        );
        assert_eq!(app.sent(), []);

        let (gate, app) = new_gate();
        assert_eq!(
            gate.should_terminate(|| gate.refuse()),
            TerminateReply::Cancel
        );
        assert_eq!(app.sent(), []);
    }

    #[test]
    fn quit_gate_replies_to_a_deferred_decision() {
        let (gate, app) = new_gate();
        assert_eq!(gate.should_terminate(|| {}), TerminateReply::Later);
        // A second quit while AppKit waits is not filed again.
        assert_eq!(
            gate.should_terminate(|| panic!("filed twice")),
            TerminateReply::Later
        );
        gate.refuse();
        assert_eq!(app.sent(), [Sent::Reply(false)]);

        // The refused question closed: the next quit is filed again.
        assert_eq!(gate.should_terminate(|| {}), TerminateReply::Later);
        gate.terminate();
        assert_eq!(app.sent(), [Sent::Reply(false), Sent::Reply(true)]);
    }

    #[test]
    fn quit_gate_does_not_file_its_own_terminate() {
        let (gate, app) = new_gate();
        // A required request or a `Quit` finished with no question open.
        gate.terminate();
        assert_eq!(app.sent(), [Sent::Terminate]);
        // The `terminate:` it sent comes back as a quit question.
        assert_eq!(
            gate.should_terminate(|| panic!("the host's own terminate: was filed")),
            TerminateReply::Now
        );
    }

    #[test]
    fn quit_gate_ignores_a_refused_request_that_opened_no_question() {
        let (gate, app) = new_gate();
        gate.refuse();
        assert_eq!(app.sent(), []);
        assert_eq!(gate.should_terminate(|| {}), TerminateReply::Later);
    }
}
