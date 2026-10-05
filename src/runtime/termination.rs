//! Cooperative application termination.
//!
//! One [`Termination`] rides inside [`AppParts`](crate::app::AppParts) to the
//! runner that hosts the application. [`Termination::start`] wires it to the
//! runner's answer channel — a [`TerminationHost`] — and hands back a
//! [`TerminationHandle`] every quit path reports through: a termination
//! signal, the last window closing, a window-system shutdown query, a
//! [`Quit`](crate::app::Quit) menu item.
//!
//! The machine allows one termination in flight. A
//! [`Cancellable`](TerminationKind::Cancellable) request asks the
//! application's `on_quit_request` handler; [`QuitReply::Quit`] runs
//! `on_terminate` and then `host.terminate()`, [`QuitReply::Cancel`] reports
//! `host.refuse()`. A [`Required`](TerminationKind::Required) request skips
//! the question and runs `on_terminate`; arriving while the question is open
//! it supersedes it. A request arriving while another is already in flight is
//! dropped. With neither hook set, a request terminates immediately. The
//! hook futures run on the runner's local executor, so `start` belongs after
//! that executor exists.
//!
//! iOS, Android and web never call either hook — those platforms kill the
//! process without notice, so their runners never start the machine.

use alloc::boxed::Box;
use alloc::rc::Rc;
use core::cell::RefCell;
use core::fmt;
use core::future::Future;
use core::pin::Pin;

use executor_core::AnyLocalExecutorTask;
use waterui_core::Environment;

use crate::app::QuitReply;
use crate::task::spawn_local;

/// What kind of termination the platform is asking for.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TerminationKind {
    /// The user or the system asked the application to quit, and the
    /// application may still say no — `on_quit_request` runs first.
    Cancellable,
    /// The process is ending on the system's terms — a termination signal,
    /// the last window closing under
    /// [`LastWindowPolicy::Quit`](crate::app::LastWindowPolicy). The question
    /// is skipped; only `on_terminate` runs.
    Required,
}

/// How a runner hears the machine's decision.
///
/// `terminate` is called once termination proceeds — after `on_terminate`
/// finishes, or immediately when no hook stands in the way. `refuse` is
/// called when `on_quit_request` answers [`QuitReply::Cancel`], so the
/// runner can undo whatever it did to hold the quit open — a Windows
/// `ShutdownBlockReason`, a paused logout.
pub trait TerminationHost: 'static {
    /// The application is allowed to end: finish teardown.
    fn terminate(&self);
    /// The application declined to quit: release any shutdown veto.
    fn refuse(&self);
}

/// The runner's end of the machine. Cloning shares it, so every quit path —
/// a signal's event, a window subclass proc, a [`Quit`](crate::app::Quit)
/// installed in the environment — reports into the same machine.
#[derive(Clone)]
pub struct TerminationHandle {
    inner: Rc<RefCell<Shared>>,
}

impl fmt::Debug for TerminationHandle {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("TerminationHandle")
            .field("state", &self.inner.borrow().state)
            .finish_non_exhaustive()
    }
}

impl TerminationHandle {
    /// Ask for termination.
    ///
    /// A `Cancellable` request while the machine is idle asks
    /// `on_quit_request`; the machine is `Asking` until the handler answers.
    /// A `Required` request while `Asking` supersedes the open question —
    /// its task is dropped, cancelling it — and termination proceeds. Any
    /// request while a termination is already in flight, and a repeated
    /// cancellable request while the question is open, is dropped.
    pub fn request(&self, kind: TerminationKind) {
        let step = {
            let mut shared = self.inner.borrow_mut();
            match shared.state {
                State::Idle => {
                    let ask =
                        kind == TerminationKind::Cancellable && shared.on_quit_request.is_some();
                    shared.state = if ask {
                        State::Asking
                    } else {
                        State::Terminating
                    };
                    if ask { Step::Ask } else { Step::Finish }
                }
                State::Asking => match kind {
                    TerminationKind::Required => {
                        // Dropping the task cancels the question's future.
                        shared.asking = None;
                        shared.state = State::Terminating;
                        Step::Finish
                    }
                    TerminationKind::Cancellable => Step::Ignore,
                },
                State::Terminating => Step::Ignore,
            }
        };
        match step {
            Step::Ask => self.ask(),
            Step::Finish => self.finish(),
            Step::Ignore => {}
        }
    }

    /// Whether the application declared either termination hook.
    ///
    /// A runner reads it to decide whether a system quit query is worth
    /// vetoing at all — `WM_QUERYENDSESSION` returns `TRUE` outright when
    /// no hook could refuse the shutdown anyway.
    #[must_use]
    pub fn has_hooks(&self) -> bool {
        self.inner.borrow().has_hooks
    }

    /// Runs the open question: the answer reaches [`answer`](Self::answer)
    /// through a local-executor task, kept in `Shared::asking` so a
    /// superseding `Required` request can cancel it.
    fn ask(&self) {
        let question = {
            let mut shared = self.inner.borrow_mut();
            let env = shared.env.clone();
            shared.on_quit_request.as_mut().map(|hook| hook(&env))
        };
        let Some(question) = question else {
            // `request` only enters `Asking` when the hook exists.
            return;
        };
        let inner = Rc::clone(&self.inner);
        let task = spawn_local(async move {
            let reply = question.await;
            Self { inner }.answer(reply);
        });
        self.inner.borrow_mut().asking = Some(task);
    }

    /// Applies the answered question. A late answer to a question a
    /// `Required` request already superseded is dropped.
    fn answer(&self, reply: QuitReply) {
        enum Next {
            Finish,
            Refuse,
            Nothing,
        }
        let next = {
            let mut shared = self.inner.borrow_mut();
            // The task running this answer no longer needs a handle kept.
            shared.asking = None;
            match (shared.state, reply) {
                (State::Asking, QuitReply::Quit) => {
                    shared.state = State::Terminating;
                    Next::Finish
                }
                (State::Asking, QuitReply::Cancel) => {
                    shared.state = State::Idle;
                    Next::Refuse
                }
                _ => Next::Nothing,
            }
        };
        match next {
            Next::Finish => self.finish(),
            Next::Refuse => self.inner.borrow().host.refuse(),
            Next::Nothing => {}
        }
    }

    /// Runs `on_terminate` and then `host.terminate()`, or terminates
    /// immediately when no shutdown work was declared.
    fn finish(&self) {
        let (on_terminate, env, host) = {
            let mut shared = self.inner.borrow_mut();
            (
                shared.on_terminate.take(),
                shared.env.clone(),
                Rc::clone(&shared.host),
            )
        };
        match on_terminate {
            Some(hook) => {
                spawn_local(async move {
                    hook(&env).await;
                    host.terminate();
                })
                .detach();
            }
            None => host.terminate(),
        }
    }
}

/// The machine's one-at-a-time state.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum State {
    /// No termination in flight; the next request starts one.
    Idle,
    /// `on_quit_request` is deciding a cancellable request.
    Asking,
    /// `on_terminate` is running, or `host.terminate()` was already
    /// reported and the runner is tearing down.
    Terminating,
}

/// What a single request decides to do next.
enum Step {
    /// Ask `on_quit_request`.
    Ask,
    /// Go straight to `on_terminate`/`host.terminate()`.
    Finish,
    /// The request was redundant.
    Ignore,
}

/// A quit-request hook erased together with the future it returns.
type QuitRequestHook = Box<dyn FnMut(&Environment) -> Pin<Box<dyn Future<Output = QuitReply>>>>;

/// A termination hook erased together with the future it returns.
type TerminateHook = Box<dyn FnOnce(&Environment) -> Pin<Box<dyn Future<Output = ()>>>>;

/// Everything `start` shares between the handle and the futures it drives.
struct Shared {
    state: State,
    /// Whether the application declared either hook — fixed at `start`.
    has_hooks: bool,
    /// The composition-root environment the hooks extract from.
    env: Environment,
    /// The runner's answer channel.
    host: Rc<dyn TerminationHost>,
    on_quit_request: Option<QuitRequestHook>,
    on_terminate: Option<TerminateHook>,
    /// The in-flight question's task, kept because dropping it cancels the
    /// future — how a `Required` request supersedes an open question.
    asking: Option<AnyLocalExecutorTask<()>>,
}

/// The application's termination hooks, carried to a runner inside
/// [`AppParts`](crate::app::AppParts) and started there.
///
/// Built by [`App::on_quit_request`](crate::app::App::on_quit_request) and
/// [`App::on_terminate`](crate::app::App::on_terminate); see the module
/// documentation for the machine it drives.
#[derive(Default)]
pub struct Termination {
    pub(crate) on_quit_request: Option<QuitRequestHook>,
    pub(crate) on_terminate: Option<TerminateHook>,
}

impl Termination {
    /// Hand the machine to a runner.
    ///
    /// `env` is the composition-root environment the hooks' extractors read
    /// — pass the fully assembled one — and `host` the runner's answer
    /// channel. The returned handle is what every quit path reports
    /// through. Requires the runner's local executor if a hook is set:
    /// their futures spawn on it.
    pub fn start(self, env: Environment, host: impl TerminationHost) -> TerminationHandle {
        let has_hooks = self.on_quit_request.is_some() || self.on_terminate.is_some();
        TerminationHandle {
            inner: Rc::new(RefCell::new(Shared {
                state: State::Idle,
                has_hooks,
                env,
                host: Rc::new(host),
                on_quit_request: self.on_quit_request,
                on_terminate: self.on_terminate,
                asking: None,
            })),
        }
    }
}

impl fmt::Debug for Termination {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Termination")
            .field("on_quit_request", &self.on_quit_request.is_some())
            .field("on_terminate", &self.on_terminate.is_some())
            .finish()
    }
}

#[cfg(test)]
mod tests {
    use alloc::string::String;
    use alloc::vec::Vec;
    use std::sync::mpsc;

    use executor_core::LocalExecutor;
    use executor_core::async_task::{self, AsyncTask, Runnable};

    use super::*;
    use crate::app::{App, QuitReply};

    /// A `spawn_local` executor that parks runnables until [`drain`] runs
    /// them — a hook future's progress is then observable step by step
    /// without waiting on any clock.
    struct ParkedExecutor;

    thread_local! {
        static PARKED: (mpsc::Sender<Runnable>, mpsc::Receiver<Runnable>) =
            mpsc::channel();
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

    fn install_executor() {
        let _ = executor_core::try_init_local_executor(ParkedExecutor);
    }

    /// Runs every runnable parked so far; the answers they produce may park
    /// follow-up work for the next call.
    fn drain() {
        PARKED.with(|(_, receiver)| {
            while let Ok(runnable) = receiver.try_recv() {
                runnable.run();
            }
        });
    }

    /// A `TerminationHost` that records the machine's decisions.
    #[derive(Clone, Default)]
    struct Log {
        calls: Rc<RefCell<Vec<String>>>,
    }

    impl Log {
        fn calls(&self) -> Vec<String> {
            self.calls.borrow().clone()
        }
    }

    impl TerminationHost for Log {
        fn terminate(&self) {
            self.calls.borrow_mut().push("terminate".into());
        }
        fn refuse(&self) {
            self.calls.borrow_mut().push("refuse".into());
        }
    }

    /// The app's end of the test: a `Termination` built through the real
    /// `App` hooks, started against `log`.
    fn start(app: App, log: &Log) -> TerminationHandle {
        app.into_parts()
            .termination
            .start(Environment::new(), log.clone())
    }

    #[test]
    fn a_cancel_refuses_and_rearms_the_question() {
        install_executor();
        let log = Log::default();
        let app = App::new_with_windows(Vec::new(), Environment::new())
            .on_quit_request(|| async { QuitReply::Cancel });
        let handle = start(app, &log);

        handle.request(TerminationKind::Cancellable);
        drain();
        assert_eq!(log.calls(), ["refuse"]);

        // A refused quit leaves the machine idle: the next request asks again.
        handle.request(TerminationKind::Cancellable);
        drain();
        assert_eq!(log.calls(), ["refuse", "refuse"]);
    }

    #[test]
    fn a_quit_runs_on_terminate_then_terminates() {
        install_executor();
        let log = Log::default();
        let hook_log = log.clone();
        let app = App::new_with_windows(Vec::new(), Environment::new())
            .on_quit_request(|| async { QuitReply::Quit })
            .on_terminate(move || {
                let hook_log = hook_log.clone();
                async move {
                    hook_log.calls.borrow_mut().push("shutdown".into());
                }
            });
        let handle = start(app, &log);

        handle.request(TerminationKind::Cancellable);
        drain(); // the question answers
        drain(); // `on_terminate` runs and reports to the host
        assert_eq!(log.calls(), ["shutdown", "terminate"]);
    }

    #[test]
    fn a_required_request_skips_the_question() {
        install_executor();
        let log = Log::default();
        let asked = log.clone();
        let shutdown = log.clone();
        let app = App::new_with_windows(Vec::new(), Environment::new())
            .on_quit_request(move || {
                asked.calls.borrow_mut().push("asked".into());
                async { QuitReply::Cancel }
            })
            .on_terminate(move || {
                let shutdown = shutdown.clone();
                async move {
                    shutdown.calls.borrow_mut().push("shutdown".into());
                }
            });
        let handle = start(app, &log);

        handle.request(TerminationKind::Required);
        drain();
        assert_eq!(log.calls(), ["shutdown", "terminate"]);
    }

    #[test]
    fn a_required_request_supersedes_an_open_question() {
        install_executor();
        let log = Log::default();
        let asked = log.clone();
        let shutdown = log.clone();
        let app = App::new_with_windows(Vec::new(), Environment::new())
            .on_quit_request(move || {
                asked.calls.borrow_mut().push("asked".into());
                // The question stays open — its future never answers.
                core::future::pending::<QuitReply>()
            })
            .on_terminate(move || {
                let shutdown = shutdown.clone();
                async move {
                    shutdown.calls.borrow_mut().push("shutdown".into());
                }
            });
        let handle = start(app, &log);

        handle.request(TerminationKind::Cancellable);
        drain();
        assert_eq!(log.calls(), ["asked"]);

        // The required request cancels the in-flight question and proceeds.
        handle.request(TerminationKind::Required);
        drain();
        assert_eq!(log.calls(), ["asked", "shutdown", "terminate"]);
    }

    #[test]
    fn a_repeated_request_while_in_flight_is_dropped() {
        install_executor();
        let log = Log::default();
        let asked = log.clone();
        let app =
            App::new_with_windows(Vec::new(), Environment::new()).on_quit_request(move || {
                asked.calls.borrow_mut().push("asked".into());
                core::future::pending::<QuitReply>()
            });
        let handle = start(app, &log);

        handle.request(TerminationKind::Cancellable);
        handle.request(TerminationKind::Cancellable);
        drain();
        // The question was asked once, not twice.
        assert_eq!(log.calls(), ["asked"]);

        // Once terminating, further requests are dropped as well.
        handle.request(TerminationKind::Required);
        handle.request(TerminationKind::Required);
        handle.request(TerminationKind::Cancellable);
        drain();
        assert_eq!(log.calls(), ["asked", "terminate"]);
    }

    #[test]
    fn a_request_without_hooks_terminates_immediately() {
        install_executor();
        let log = Log::default();
        let app = App::new_with_windows(Vec::new(), Environment::new());
        let handle = start(app, &log);

        handle.request(TerminationKind::Cancellable);
        // Synchronous — nothing was parked on the executor at all.
        assert_eq!(log.calls(), ["terminate"]);
    }
}
