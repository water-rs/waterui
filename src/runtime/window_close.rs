//! A window's cancellable close request.
//!
//! One [`CloseRequest`] rides inside every [`Window`](crate::window::Window)
//! and is shared by `Rc` with the [`WindowHandle`](crate::window::WindowHandle)s
//! the window hands out — the per-window sibling of the termination machine in
//! `termination`. The backend arms it with the environment the window renders
//! under and the window's `closable` when it realizes the window, and files
//! every user close through `Window::request_close`: the title-bar close
//! button, the window manager's close — X11 `WM_DELETE_WINDOW`, Wayland
//! `xdg_toplevel.close`, `WM_CLOSE` — a Close Window menu command and
//! `performClose:`. [`WindowHandle::request_close`](crate::window::WindowHandle::request_close)
//! files into the same machine. A `state` write of [`WindowState::Closed`],
//! [`WindowHandle::close`](crate::window::WindowHandle::close), application
//! termination and the last-window policy are not requests and never ask.
//!
//! With an `on_close_request` handler installed the machine asks it; the
//! answer is a [`CloseReply`] arriving on the runner's local executor — never
//! inside a platform delegate call. `Close` writes `state = Closed` and the
//! normal teardown runs; `Cancel` writes nothing. With no handler a request
//! closes the window immediately; a window declared `closable == false` drops
//! the request before the handler runs.
//!
//! The machine asks one question per window: a request arriving while a
//! question is open is dropped. The machine watches `state`, so a
//! programmatic close while the question is open drops the question's task,
//! cancelling its future, as a `Required` termination supersedes
//! `on_quit_request` — which is also why a `Close` reply can never land on an
//! already-closed window.
//!
//! Hydrolysis Android, the `UIKit` runner and the web backend have no window
//! close request, so they never arm the machine: a handler installed there
//! never runs, and `WindowHandle::request_close` panics.

use alloc::boxed::Box;
use alloc::rc::{Rc, Weak};
use core::cell::RefCell;
use core::fmt;
use core::future::Future;
use core::mem;
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
pub type CloseRequestHook = Box<dyn FnMut(&Environment) -> Question>;

/// The future an `on_close_request` hook returns: the open question.
pub type Question = Pin<Box<dyn Future<Output = CloseReply>>>;

/// What the backend told the machine when it realized the window.
struct Armed {
    /// The environment the window renders under; the handler extracts from it.
    env: Environment,
    /// The window's declared `closable`.
    closable: bool,
}

/// The machine's one-question-at-a-time state.
enum Phase {
    /// No question open; the next request starts one.
    Idle,
    /// `on_close_request` is deciding a request. The task is `None` while the
    /// hook builds the question, and afterwards holds the task answering it —
    /// kept because dropping it cancels the question's future.
    Asking(Option<AnyLocalExecutorTask<()>>),
}

/// Everything `Window`, `WindowHandle` and the question's task share.
struct Shared {
    phase: Phase,
    /// The binding a `Close` reply writes, and the one the machine watches
    /// for a programmatic close.
    window_state: Binding<WindowState>,
    /// Set when the backend realizes the window; a request before then panics.
    armed: Option<Armed>,
    hook: Option<CloseRequestHook>,
    /// The subscription dropping the open question when `window_state`
    /// reaches `Closed` by any path. It lives here so it dies with the machine.
    _close_watch: <Binding<WindowState> as Signal>::Guard,
}

/// The close-request machine one [`Window`](crate::window::Window) carries.
///
/// Cloning shares the machine — a `WindowHandle` files requests into the same
/// machine the backend armed.
#[derive(Clone)]
pub struct CloseRequest {
    inner: Rc<RefCell<Shared>>,
}

impl fmt::Debug for CloseRequest {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let shared = self.inner.borrow();
        f.debug_struct("CloseRequest")
            .field("asking", &matches!(shared.phase, Phase::Asking(_)))
            .field("armed", &shared.armed.is_some())
            .field("has_handler", &shared.hook.is_some())
            .finish_non_exhaustive()
    }
}

impl CloseRequest {
    /// Builds the machine for a window whose open state is `window_state`.
    pub(crate) fn new(window_state: &Binding<WindowState>) -> Self {
        let inner = Rc::new_cyclic(|machine: &Weak<RefCell<Shared>>| {
            let machine = machine.clone();
            let close_watch = window_state.watch(move |context| {
                if *context.value() == WindowState::Closed
                    && let Some(inner) = machine.upgrade()
                {
                    Self { inner }.cancel_question();
                }
            });
            RefCell::new(Shared {
                phase: Phase::Idle,
                window_state: window_state.clone(),
                armed: None,
                hook: None,
                _close_watch: close_watch,
            })
        });
        Self { inner }
    }

    /// Installs the `on_close_request` hook, replacing any set before.
    pub(crate) fn set_hook(&self, hook: CloseRequestHook) {
        self.inner.borrow_mut().hook = Some(hook);
    }

    /// Records the environment the window renders under and its `closable`.
    pub(crate) fn arm(&self, env: &Environment, closable: bool) {
        self.inner.borrow_mut().armed = Some(Armed {
            env: env.clone(),
            closable,
        });
    }

    /// Whether the application installed an `on_close_request` handler.
    pub(crate) fn has_handler(&self) -> bool {
        self.inner.borrow().hook.is_some()
    }

    /// Files a close request.
    ///
    /// `closable == false`, an open question or an already-closed window
    /// drops it. With no handler it writes `WindowState::Closed` now; with
    /// one, the handler's question runs on the local executor. No
    /// application code runs while the machine is borrowed: a hook that
    /// closes the window while it builds its question leaves that question
    /// stale, and it is dropped unasked.
    ///
    /// # Panics
    ///
    /// When the machine was never armed — the window was never realized, or
    /// its platform has no window close request (Android, `UIKit`, web).
    pub(crate) fn request(&self) {
        let (mut hook, env) = {
            let mut shared = self.inner.borrow_mut();
            let armed = shared.armed.as_ref().expect(
                "a close request was filed for a window that was never realized, or whose \
                 platform has no window close request (Android, UIKit, web)",
            );
            if !armed.closable
                || matches!(shared.phase, Phase::Asking(_))
                || shared.window_state.snapshot() == WindowState::Closed
            {
                return;
            }
            let env = armed.env.clone();
            let Some(hook) = shared.hook.take() else {
                let window_state = shared.window_state.clone();
                drop(shared);
                window_state.set(WindowState::Closed);
                return;
            };
            shared.phase = Phase::Asking(None);
            (hook, env)
        };
        let question = hook(&env);
        let still_asking = {
            let mut shared = self.inner.borrow_mut();
            shared.hook = Some(hook);
            matches!(shared.phase, Phase::Asking(None))
        };
        if !still_asking {
            // Dropped after the borrow ends: the future is application code.
            drop(question);
            return;
        }
        // The task holds the machine weakly: the machine owns the task, so a
        // window dropped with its question open drops — and cancels — it.
        let machine = Rc::downgrade(&self.inner);
        let task = spawn_local(async move {
            let reply = question.await;
            if let Some(inner) = machine.upgrade() {
                Self { inner }.answer(reply);
            }
        });
        // `spawn_local` never polls inline, so the question is still open.
        self.inner.borrow_mut().phase = Phase::Asking(Some(task));
    }

    /// Applies the answered question.
    fn answer(&self, reply: CloseReply) {
        let close = {
            let mut shared = self.inner.borrow_mut();
            // The handle is this running task's own: detached, it finishes.
            if let Phase::Asking(Some(task)) = mem::replace(&mut shared.phase, Phase::Idle) {
                task.detach();
            }
            (reply == CloseReply::Close).then(|| shared.window_state.clone())
        };
        if let Some(window_state) = close {
            window_state.set(WindowState::Closed);
        }
    }

    /// Drops the open question's task, cancelling its future — the
    /// programmatic-close path.
    fn cancel_question(&self) {
        let phase = mem::replace(&mut self.inner.borrow_mut().phase, Phase::Idle);
        // Dropped after the borrow ends: cancelling runs the future's drop,
        // which is application code.
        drop(phase);
    }
}

#[cfg(test)]
mod tests {
    use alloc::rc::Rc;
    use std::cell::Cell;
    use std::future::pending;

    use waterui_core::binding;

    use super::super::parked_executor::{drain, install as install_executor};
    use super::*;
    use crate::window::Window;

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

    /// A window whose handler counts its questions and answers `reply`.
    fn counting(reply: CloseReply) -> (Window, Rc<Cell<u32>>) {
        let calls = Rc::new(Cell::new(0u32));
        let window = window().on_close_request({
            let calls = Rc::clone(&calls);
            move || {
                calls.set(calls.get() + 1);
                async move { reply }
            }
        });
        (window, calls)
    }

    #[test]
    fn a_request_with_no_handler_closes_immediately() {
        install_executor();
        let window = window();
        window.request_close(&Environment::new());
        assert_eq!(window.state.snapshot(), WindowState::Closed);
    }

    #[test]
    fn a_cancel_leaves_the_window_open_and_rearms_the_question() {
        install_executor();
        let env = Environment::new();
        let (window, calls) = counting(CloseReply::Cancel);
        window.request_close(&env);
        // The answer is a future: nothing is written until the executor runs
        // it.
        assert_eq!(window.state.snapshot(), WindowState::Normal);
        drain();
        assert_eq!(window.state.snapshot(), WindowState::Normal);
        assert_eq!(calls.get(), 1);

        window.request_close(&env);
        drain();
        assert_eq!(calls.get(), 2, "a refused close leaves the machine idle");
    }

    #[test]
    fn a_close_reply_writes_closed() {
        install_executor();
        let (window, _) = counting(CloseReply::Close);
        window.request_close(&Environment::new());
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
        window.request_close(&env);
        window.request_close(&env);
        window.handle().request_close();
        drain();
        assert_eq!(calls.get(), 1, "requests are dropped while asking");
    }

    /// A pending question whose future reports its drop through the returned
    /// flag.
    fn pending_question() -> (Window, Rc<Cell<bool>>) {
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
        (window, dropped)
    }

    #[test]
    fn a_programmatic_close_cancels_the_question() {
        install_executor();
        let (window, dropped) = pending_question();
        window.request_close(&Environment::new());
        drain();
        window.handle().close();
        // Cancelling marks the task; the future dies when the executor next
        // turns the cancelled runnable.
        drain();
        assert!(dropped.get(), "a programmatic close drops the question");
        assert_eq!(window.state.snapshot(), WindowState::Closed);
    }

    #[test]
    fn dropping_the_window_cancels_the_question() {
        install_executor();
        let (window, dropped) = pending_question();
        window.request_close(&Environment::new());
        drain();
        drop(window);
        drain();
        assert!(dropped.get(), "the machine owns its question's task");
    }

    #[test]
    fn closable_false_drops_the_request_before_the_handler() {
        install_executor();
        let (mut window, calls) = counting(CloseReply::Close);
        window.closable = false;
        window.request_close(&Environment::new());
        window.handle().request_close();
        drain();
        assert_eq!(calls.get(), 0);
        assert_eq!(window.state.snapshot(), WindowState::Normal);
    }

    #[test]
    fn a_handle_request_routes_through_the_handler() {
        install_executor();
        let (window, calls) = counting(CloseReply::Cancel);
        window.arm_close_requests(&Environment::new());
        window.handle().request_close();
        drain();
        assert_eq!(calls.get(), 1);
        assert_eq!(window.state.snapshot(), WindowState::Normal);
    }

    #[test]
    fn a_handle_close_bypasses_the_handler() {
        install_executor();
        let (window, calls) = counting(CloseReply::Cancel);
        window.arm_close_requests(&Environment::new());
        window.handle().close();
        drain();
        assert_eq!(calls.get(), 0);
        assert_eq!(window.state.snapshot(), WindowState::Closed);
    }

    #[test]
    #[should_panic(expected = "never realized")]
    fn a_handle_request_before_realization_panics() {
        window().handle().request_close();
    }
}
