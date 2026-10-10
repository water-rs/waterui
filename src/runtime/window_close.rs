//! A window's cancellable close request.
//!
//! One [`CloseRequest`] rides inside every [`Window`](crate::window::Window)
//! and is shared by `Rc` with the [`WindowHandle`](crate::window::WindowHandle)s
//! the window hands out — the per-window sibling of the termination machine in
//! `termination`. The backend arms the machine with the environment the window
//! renders under when it realizes the window ([`CloseRequest::arm`]), and files
//! every close request through the one entry point,
//! [`CloseRequest::request`]: the title-bar close button, the window manager's
//! close — X11 `WM_DELETE_WINDOW`, Wayland `xdg_toplevel.close`, `WM_CLOSE` —
//! a Close Window menu command, `performClose:`, and
//! [`WindowHandle::request_close`](crate::window::WindowHandle::request_close)
//! all land there. A `state` write of [`WindowState::Closed`],
//! [`WindowHandle::close`](crate::window::WindowHandle::close), app
//! termination and the last-window policy are not requests and never ask.
//!
//! With an `on_close_request` handler installed the machine asks it; the
//! answer is a [`CloseReply`] arriving on the runner's local executor — never
//! inside a platform delegate call. `Close` writes `state = Closed` and the
//! normal teardown runs; `Cancel` writes nothing. With no handler a request
//! closes the window immediately; a window declared `closable == false` drops
//! the request before the handler runs.
//!
//! The machine allows one question per window: a request arriving while a
//! question is open is dropped. A programmatic close while the question is
//! open closes the window and drops the future, cancelling it, as a
//! `Required` termination supersedes `on_quit_request` — the machine's
//! subscription on `window_state` watches for the `Closed` edge. A `Close`
//! reply that lands after the window already closed is a no-op.
//!
//! Hydrolysis Android, the UIKit runner and the web backend never call
//! `request` — those surfaces have no window close request — so a handler
//! installed there simply never runs. Filing a request before the window is
//! realized panics with a clear message.

use alloc::boxed::Box;
use alloc::rc::Rc;
use core::cell::RefCell;
use core::fmt;
use core::future::Future;
use core::pin::Pin;

use executor_core::AnyLocalExecutorTask;
use nami::{Binding, Signal};
use waterui_core::Environment;

use crate::task::spawn_local;

use super::window::WindowState;

/// What an [`Window::on_close_request`](crate::window::Window::on_close_request)
/// handler answers for a close request.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CloseReply {
    /// Close the window: `Window::state` is written
    /// [`WindowState::Closed`] and the normal teardown runs.
    Close,
    /// Refuse the close: the window stays open and nothing is written.
    Cancel,
}

/// A close-request hook erased together with the future it returns.
type CloseRequestHook = Box<dyn FnMut(&Environment) -> Question>;

/// The future an `on_close_request` hook returns: the open question.
type Question = Pin<Box<dyn Future<Output = CloseReply>>>;

/// The machine's one-question-at-a-time state.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum State {
    /// No question open; the next request starts one.
    Idle,
    /// `on_close_request` is deciding a request.
    Asking,
}

/// What a single request decides to do next.
enum Step {
    /// Ask `on_close_request`: the question its hook returned.
    Ask(Question),
    /// No handler stands in the way: write `WindowState::Closed`.
    Close,
    /// The request was dropped — `closable` is false, a question is already
    /// open, or the window is already closed.
    Ignore,
}

/// Everything `Window`, `WindowHandle` and the question's task share.
struct Shared {
    state: State,
    /// The window's `closable`, mirrored by `Window::request_close` and at
    /// `arm` so a `WindowHandle::request_close` files against the declared
    /// value.
    closable: bool,
    /// The binding a `Close` reply writes, and the binding the subscription
    /// in `close_watch` reads to drop an open question on a programmatic
    /// close.
    window_state: Binding<WindowState>,
    /// The environment the window renders under, armed when the backend
    /// realizes the window — a request filed before then panics.
    env: Option<Environment>,
    hook: Option<CloseRequestHook>,
    /// The in-flight question's task, kept because dropping it cancels the
    /// future — how a programmatic close supersedes an open question.
    asking: Option<AnyLocalExecutorTask<()>>,
    /// The subscription cancelling `asking` when the window reaches
    /// `Closed` by any path — the machine's own `Close` write finds no task
    /// to cancel, so every edge it catches is a programmatic close.
    /// Installed at construction through a `Weak` to the machine; the guard
    /// lives here so the subscription dies with the machine.
    close_watch: Option<<Binding<WindowState> as Signal>::Guard>,
}

/// The close-request machine one [`Window`](crate::window::Window) carries.
///
/// Cloning shares the machine — a `WindowHandle` files requests into the same
/// machine the backend armed. `CloseRequest` is backend-facing plumbing: the
/// application surface is [`Window::on_close_request`](crate::window::Window::on_close_request)
/// and [`WindowHandle::request_close`](crate::window::WindowHandle::request_close).
#[doc(hidden)]
#[derive(Clone)]
pub struct CloseRequest {
    inner: Rc<RefCell<Shared>>,
}

impl fmt::Debug for CloseRequest {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("CloseRequest")
            .field("state", &self.inner.borrow().state)
            .finish_non_exhaustive()
    }
}

impl CloseRequest {
    /// Builds the machine for a window: `window_state` is the binding a
    /// `Close` reply writes and the binding watched for a programmatic close.
    pub(crate) fn new(window_state: Binding<WindowState>, closable: bool) -> Self {
        let this = Self {
            inner: Rc::new(RefCell::new(Shared {
                state: State::Idle,
                closable,
                window_state: window_state.clone(),
                env: None,
                hook: None,
                asking: None,
                close_watch: None,
            })),
        };
        let weak = Rc::downgrade(&this.inner);
        let watch = window_state.watch(move |context| {
            if *context.value() == WindowState::Closed
                && let Some(inner) = weak.upgrade()
            {
                Self { inner }.cancel_asking();
            }
        });
        this.inner.borrow_mut().close_watch = Some(watch);
        this
    }

    /// Installs the `on_close_request` hook — [`Window::on_close_request`]
    /// delegates here.
    pub(crate) fn set_hook(&self, hook: CloseRequestHook) {
        self.inner.borrow_mut().hook = Some(hook);
    }

    /// Mirrors the window's live `closable` into the machine — the file path
    /// every `Window::request_close` takes, and `arm`.
    pub fn set_closable(&self, closable: bool) {
        self.inner.borrow_mut().closable = closable;
    }

    /// Arms the machine with the environment the window renders under.
    ///
    /// A backend calls it when it realizes the window; the handler's
    /// extractor arguments resolve against this environment. It is idempotent
    /// — `request` arms the same way — and a request filed through
    /// [`WindowHandle::request_close`](crate::window::WindowHandle::request_close)
    /// uses the armed environment.
    pub fn arm(&self, env: &Environment) {
        let mut shared = self.inner.borrow_mut();
        if shared.env.is_none() {
            shared.env = Some(env.clone());
        }
    }

    /// Whether the application installed an `on_close_request` handler.
    ///
    /// A backend reads it where the platform wants a synchronous verdict —
    /// `windowShouldClose:` returns `NO` only when a handler could refuse.
    #[must_use]
    pub fn has_close_handler(&self) -> bool {
        self.inner.borrow().hook.is_some()
    }

    /// Files a close request — the entry point every close path takes.
    ///
    /// `env` is the environment the window renders under; the machine is
    /// armed with it on the way in. `closable == false` drops the request
    /// before the handler runs; so does an open question or an already-closed
    /// window. With no handler the request writes `WindowState::Closed`
    /// immediately; with one the handler's future runs on the local executor.
    pub fn request(&self, env: &Environment) {
        match self.step(env) {
            Step::Ask(question) => self.ask(question),
            Step::Close => {
                // The write sits outside the borrow: a watcher may re-enter
                // the machine.
                let window_state = self.inner.borrow().window_state.clone();
                window_state.set(WindowState::Closed);
            }
            Step::Ignore => {}
        }
    }

    /// Files a close request on the armed environment — the
    /// [`WindowHandle::request_close`](crate::window::WindowHandle::request_close)
    /// path.
    ///
    /// # Panics
    ///
    /// Panics when the window was never realized on a backend that arms
    /// close requests — the backend's `arm` has not run. On a platform with
    /// no close-request support (Android, UIKit, web) it panics the same
    /// way: nothing ever arms the machine there.
    pub(crate) fn request_armed(&self) {
        let env = {
            let shared = self.inner.borrow();
            shared.env.clone().expect(
                "WindowHandle::request_close called on a window that was never realized, or whose \
                 platform has no close requests",
            )
        };
        self.request(&env);
    }

    /// Moves the machine for one request and decides what follows. No
    /// application code runs while the machine is borrowed: the question's
    /// future is built from the hook after the borrow ends, so a hook that
    /// re-enters `request` — or `state.set` — leaves the question stale; it
    /// is then dropped unasked.
    fn step(&self, env: &Environment) -> Step {
        let (mut hook, env) = {
            let mut shared = self.inner.borrow_mut();
            if shared.env.is_none() {
                shared.env = Some(env.clone());
            }
            if !shared.closable
                || shared.state == State::Asking
                || shared.window_state.snapshot() == WindowState::Closed
            {
                return Step::Ignore;
            }
            let Some(hook) = shared.hook.take() else {
                return Step::Close;
            };
            shared.state = State::Asking;
            (hook, env.clone())
        };
        let question = hook(&env);
        let still_asking = {
            let mut shared = self.inner.borrow_mut();
            shared.hook = Some(hook);
            shared.state == State::Asking
        };
        if still_asking {
            Step::Ask(question)
        } else {
            // Dropped after the borrow ends: the future is application code.
            drop(question);
            Step::Ignore
        }
    }

    /// Runs the open question: the answer reaches [`answer`](Self::answer)
    /// through a local-executor task, kept in `Shared::asking` so a
    /// programmatic close can cancel it.
    fn ask(&self, question: Question) {
        let inner = Rc::clone(&self.inner);
        let task = spawn_local(async move {
            let reply = question.await;
            Self { inner }.answer(reply);
        });
        self.inner.borrow_mut().asking = Some(task);
    }

    /// Applies the answered question. A late `Close` to an already-closed
    /// window is a no-op; a late answer to a question a programmatic close
    /// already superseded never arrives — the task was cancelled.
    fn answer(&self, reply: CloseReply) {
        enum Next {
            Close(Binding<WindowState>),
            Nothing,
        }
        let (asking, next) = {
            let mut shared = self.inner.borrow_mut();
            let asking = shared.asking.take();
            let next = match (shared.state, reply) {
                (State::Asking, CloseReply::Close) => {
                    shared.state = State::Idle;
                    if shared.window_state.snapshot() == WindowState::Closed {
                        Next::Nothing
                    } else {
                        Next::Close(shared.window_state.clone())
                    }
                }
                (State::Asking, CloseReply::Cancel) => {
                    shared.state = State::Idle;
                    Next::Nothing
                }
                _ => Next::Nothing,
            };
            (asking, next)
        };
        // The handle is this running task's own: detached, it finishes.
        if let Some(asking) = asking {
            asking.detach();
        }
        match next {
            Next::Close(window_state) => window_state.set(WindowState::Closed),
            Next::Nothing => {}
        }
    }

    /// Drops the open question's task, cancelling its future — the
    /// programmatic-close path.
    fn cancel_asking(&self) {
        let asking = {
            let mut shared = self.inner.borrow_mut();
            if shared.state == State::Asking {
                shared.state = State::Idle;
            }
            shared.asking.take()
        };
        // Dropped after the borrow ends: cancelling runs the future's drop,
        // which may report into this machine itself.
        drop(asking);
    }
}

#[cfg(test)]
mod tests {
    use alloc::rc::Rc;
    use std::cell::Cell;
    use std::future::pending;
    use std::sync::mpsc;

    use executor_core::LocalExecutor;
    use executor_core::async_task::{self, AsyncTask, Runnable};
    use waterui_core::binding;

    use super::*;
    use crate::window::Window;

    /// A `spawn_local` executor that parks runnables until [`drain`] runs
    /// them — a hook future's progress is then observable step by step
    /// without waiting on any clock. The same harness `termination`'s tests
    /// use.
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

    /// Runs every parked runnable, and every one those park in turn, until
    /// the queue is empty.
    fn drain() {
        PARKED.with(|(_, receiver)| {
            while let Ok(runnable) = receiver.try_recv() {
                runnable.run();
            }
        });
    }

    /// A flag a dropped future reports through — how a cancelled question
    /// shows it died.
    struct Dropped(Rc<Cell<bool>>);

    impl Drop for Dropped {
        fn drop(&mut self) {
            self.0.set(true);
        }
    }

    fn window() -> Window {
        Window::new("w", binding(WindowState::Normal), || ())
    }

    #[test]
    fn a_request_with_no_handler_closes_immediately() {
        install_executor();
        let env = Environment::new();
        let window = window();
        window.arm_close_requests(&env);
        window.request_close(&env);
        assert_eq!(window.state.snapshot(), WindowState::Closed);
    }

    #[test]
    fn a_cancel_leaves_the_window_open_and_rearms_the_question() {
        install_executor();
        let env = Environment::new();
        let calls = Rc::new(Cell::new(0u32));
        let window = window().on_close_request({
            let calls = Rc::clone(&calls);
            move || {
                calls.set(calls.get() + 1);
                async { CloseReply::Cancel }
            }
        });
        window.arm_close_requests(&env);
        window.request_close(&env);
        // The request filed, but the answer is a future: nothing is written
        // until the executor runs it.
        assert_eq!(window.state.snapshot(), WindowState::Normal);
        drain();
        assert_eq!(window.state.snapshot(), WindowState::Normal);
        assert_eq!(calls.get(), 1);

        // A refused close leaves the machine idle: the next request asks
        // again.
        window.request_close(&env);
        drain();
        assert_eq!(calls.get(), 2);
    }

    #[test]
    fn a_close_reply_writes_closed() {
        install_executor();
        let env = Environment::new();
        let window = window().on_close_request(|| async { CloseReply::Close });
        window.arm_close_requests(&env);
        window.request_close(&env);
        assert_eq!(window.state.snapshot(), WindowState::Normal);
        drain();
        assert_eq!(window.state.snapshot(), WindowState::Closed);
    }

    #[test]
    fn one_question_is_asked_per_window() {
        install_executor();
        let env = Environment::new();
        let calls = Rc::new(Cell::new(0u32));
        let window = window().on_close_request({
            let calls = Rc::clone(&calls);
            move || {
                calls.set(calls.get() + 1);
                pending()
            }
        });
        window.arm_close_requests(&env);
        window.request_close(&env);
        window.request_close(&env);
        assert_eq!(calls.get(), 1, "the second request is dropped while asking");
    }

    #[test]
    fn a_programmatic_close_supersedes_the_question() {
        install_executor();
        let env = Environment::new();
        let dropped = Rc::new(Cell::new(false));
        let window = window().on_close_request({
            let dropped = Rc::clone(&dropped);
            move || {
                let dropped = Dropped(Rc::clone(&dropped));
                async move {
                    let _dropped = dropped;
                    pending().await
                }
            }
        });
        window.arm_close_requests(&env);
        window.request_close(&env);
        window.state.set(WindowState::Closed);
        // Cancelling marks the task; the future dies when the executor next
        // turns the cancelled runnable — draining drops it unpolled.
        drain();
        assert!(
            dropped.get(),
            "a programmatic close must drop the open question's future"
        );
        assert_eq!(window.state.snapshot(), WindowState::Closed);
    }

    #[test]
    fn a_late_close_reply_to_a_closed_window_is_a_no_op() {
        install_executor();
        let env = Environment::new();
        let window = window().on_close_request(|| async { CloseReply::Close });
        window.arm_close_requests(&env);
        window.request_close(&env);
        window.state.set(WindowState::Closed);
        // The question was superseded; draining must not resurrect or panic.
        drain();
        assert_eq!(window.state.snapshot(), WindowState::Closed);
    }

    #[test]
    fn closable_false_drops_the_request_before_the_handler() {
        install_executor();
        let env = Environment::new();
        let calls = Rc::new(Cell::new(0u32));
        let mut window = window().on_close_request({
            let calls = Rc::clone(&calls);
            move || {
                calls.set(calls.get() + 1);
                async { CloseReply::Close }
            }
        });
        window.closable = false;
        window.arm_close_requests(&env);
        window.request_close(&env);
        drain();
        assert_eq!(calls.get(), 0);
        assert_eq!(window.state.snapshot(), WindowState::Normal);
    }

    #[test]
    fn a_handle_request_routes_through_the_handler() {
        install_executor();
        let env = Environment::new();
        let calls = Rc::new(Cell::new(0u32));
        let window = window().on_close_request({
            let calls = Rc::clone(&calls);
            move || {
                calls.set(calls.get() + 1);
                async { CloseReply::Cancel }
            }
        });
        let handle = window.handle();
        window.arm_close_requests(&env);
        handle.request_close();
        drain();
        assert_eq!(calls.get(), 1);
        assert_eq!(window.state.snapshot(), WindowState::Normal);
    }

    #[test]
    fn a_handle_close_bypasses_the_handler() {
        install_executor();
        let env = Environment::new();
        let calls = Rc::new(Cell::new(0u32));
        let window = window().on_close_request({
            let calls = Rc::clone(&calls);
            move || {
                calls.set(calls.get() + 1);
                async { CloseReply::Cancel }
            }
        });
        let handle = window.handle();
        window.arm_close_requests(&env);
        handle.close();
        drain();
        assert_eq!(calls.get(), 0);
        assert_eq!(window.state.snapshot(), WindowState::Closed);
    }

    #[test]
    #[should_panic(expected = "never realized, or whose platform has no close requests")]
    fn a_handle_request_before_realization_panics() {
        let window = window();
        window.handle().request_close();
    }
}
